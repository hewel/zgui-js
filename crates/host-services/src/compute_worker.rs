//! Isolated background JavaScript compute workers.
//!
//! Each live worker owns a dedicated OS thread and a dedicated QuickJS `Runtime`
//! with zero UI bridge, zero Solid graph, zero inherited host capabilities, and
//! no transferable native handles. In deterministic preview mode, the exact same
//! `WorkerEngine` runs in lockstep quanta without spawning OS threads.

use js_runtime::{Diagnostic, DiagnosticKind, JsRuntime, RejectionPolicy, RuntimeLimits};
use rquickjs::{Ctx, Exception, Function, Object, Value, function::Func};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fmt,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread,
    time::Duration,
};

pub const MAX_WORKERS_PER_SESSION: usize = 8;
pub const MIN_WORKER_HEAP_BYTES: usize = 256 * 1024;
pub const DEFAULT_WORKER_HEAP_BYTES: usize = 16 * 1024 * 1024;
pub const DEFAULT_AGGREGATE_SESSION_HEAP_BYTES: usize = 32 * 1024 * 1024;
pub const WORKER_STACK_BYTES: usize = 512 * 1024;
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub const MAX_MESSAGE_DEPTH: usize = 32;
pub const INBOUND_MAILBOX_CAPACITY: usize = 32;
pub const OUTBOUND_MAILBOX_CAPACITY: usize = 32;

pub fn default_turn_timeout() -> Duration {
    Duration::from_millis(if cfg!(debug_assertions) { 250 } else { 100 })
}

pub fn default_init_timeout() -> Duration {
    Duration::from_millis(if cfg!(debug_assertions) { 1000 } else { 500 })
}

/// Owned error reported by an isolated compute worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerError {
    pub code: &'static str,
    pub message: String,
    pub stack: Option<String>,
}

impl WorkerError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            stack: None,
        }
    }

    fn from_diagnostic(diagnostic: Diagnostic) -> Self {
        let code = match diagnostic.kind {
            DiagnosticKind::Interrupted | DiagnosticKind::BudgetExceeded => "E_WORKER_TIMEOUT",
            DiagnosticKind::Exception | DiagnosticKind::Engine
                if diagnostic.message.to_ascii_lowercase().contains("out of memory") =>
            {
                "E_WORKER_OOM"
            }
            _ => "E_WORKER_EXCEPTION",
        };
        Self {
            code,
            message: diagnostic.message,
            stack: diagnostic.stack,
        }
    }
}

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for WorkerError {}

/// Event delivered from a compute worker to its owning session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerEvent {
    Message(String),
    Error(WorkerError),
    Terminated,
}

/// Deterministic virtual clock and PRNG seed for preview lockstep execution.
#[derive(Clone, Copy, Debug)]
pub struct DeterministicWorkerEnv {
    pub initial_epoch_ms: u64,
    pub rng_seed: u64,
}

/// Configuration for a verified compute worker module.
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    pub name: String,
    pub module_id: String,
    pub source: String,
    pub bytecode: Option<Vec<u8>>,
    pub heap_limit_bytes: usize,
    pub turn_timeout: Duration,
    pub init_timeout: Duration,
    pub max_jobs_per_turn: usize,
    pub deterministic_env: Option<DeterministicWorkerEnv>,
}

impl WorkerConfig {
    pub fn new(name: impl Into<String>, module_id: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            module_id: module_id.into(),
            source: source.into(),
            bytecode: None,
            heap_limit_bytes: DEFAULT_WORKER_HEAP_BYTES,
            turn_timeout: default_turn_timeout(),
            init_timeout: default_init_timeout(),
            max_jobs_per_turn: 128,
            deterministic_env: None,
        }
    }

    #[must_use]
    pub fn with_bytecode(mut self, bytecode: Vec<u8>) -> Self {
        self.bytecode = Some(bytecode);
        self
    }
}

struct MemoryBudgetInner {
    limit_bytes: usize,
    used_bytes: AtomicUsize,
}

/// Aggregate session-wide heap reservation shared across all workers in one session.
#[derive(Clone)]
pub struct WorkerSessionMemoryBudget {
    inner: Arc<MemoryBudgetInner>,
}

impl Default for WorkerSessionMemoryBudget {
    fn default() -> Self {
        Self::new(DEFAULT_AGGREGATE_SESSION_HEAP_BYTES)
    }
}

impl WorkerSessionMemoryBudget {
    pub fn new(limit_bytes: usize) -> Self {
        Self {
            inner: Arc::new(MemoryBudgetInner {
                limit_bytes,
                used_bytes: AtomicUsize::new(0),
            }),
        }
    }

    pub fn try_reserve(&self, bytes: usize) -> Result<WorkerMemoryLease, WorkerError> {
        let mut current = self.inner.used_bytes.load(Ordering::Acquire);
        loop {
            let next = current.checked_add(bytes).filter(|&total| total <= self.inner.limit_bytes);
            let Some(next) = next else {
                return Err(WorkerError::new(
                    "E_RESOURCE_LIMIT",
                    format!(
                        "aggregate session worker heap budget ({} bytes) exceeded",
                        self.inner.limit_bytes
                    ),
                ));
            };
            match self.inner.used_bytes.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(WorkerMemoryLease {
                        inner: Arc::clone(&self.inner),
                        bytes,
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    pub fn reserved_bytes(&self) -> usize {
        self.inner.used_bytes.load(Ordering::Acquire)
    }
}

/// RAII reservation released when a worker engine exits or is dropped.
pub struct WorkerMemoryLease {
    inner: Arc<MemoryBudgetInner>,
    bytes: usize,
}

impl Drop for WorkerMemoryLease {
    fn drop(&mut self) {
        self.inner.used_bytes.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// Validates a serialized JSON worker message against byte and nesting depth limits.
pub fn validate_json_payload(raw: &str) -> Result<(), WorkerError> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(WorkerError::new(
            "E_PAYLOAD_INVALID",
            format!("worker message exceeds {MAX_MESSAGE_BYTES} byte limit"),
        ));
    }
    let parsed: serde_json::Value = serde_json::from_str(raw).map_err(|err| {
        WorkerError::new("E_PAYLOAD_INVALID", format!("worker message must be valid JSON: {err}"))
    })?;
    check_json_depth(&parsed, 0)
}

fn check_json_depth(value: &serde_json::Value, depth: usize) -> Result<(), WorkerError> {
    if depth > MAX_MESSAGE_DEPTH {
        return Err(WorkerError::new(
            "E_PAYLOAD_INVALID",
            format!("worker message exceeds maximum nesting depth of {MAX_MESSAGE_DEPTH}"),
        ));
    }
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                check_json_depth(item, depth + 1)?;
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                check_json_depth(item, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Validates and serializes a plain JSON-compatible JS value inside a QuickJS context.
/// Rejects functions, symbols, BigInt, circular graphs, native handles, and non-plain prototypes.
pub fn serialize_plain_js_payload<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<String> {
    let validator: Function<'js> = ctx.eval(PLAIN_JSON_SERIALIZER_SCRIPT)?;
    let raw: String = validator.call((value, MAX_MESSAGE_BYTES as i32, MAX_MESSAGE_DEPTH as i32))?;
    Ok(raw)
}

const PLAIN_JSON_SERIALIZER_SCRIPT: &str = r#"(function(value, maxBytes, maxDepth) {
    const objProto = Object.prototype;
    const arrProto = Array.prototype;
    const seen = new Set();
    function check(v, depth) {
        if (depth > maxDepth) {
            throw new RangeError("worker message exceeds maximum nesting depth");
        }
        if (v === null) return;
        const t = typeof v;
        if (t === "boolean" || t === "string") return;
        if (t === "number") {
            if (!Number.isFinite(v)) {
                throw new TypeError("worker message numbers must be finite");
            }
            return;
        }
        if (t !== "object") {
            throw new TypeError("worker message must contain only plain JSON data");
        }
        if (seen.has(v)) {
            throw new TypeError("worker message cannot contain circular references");
        }
        const proto = Object.getPrototypeOf(v);
        if (Array.isArray(v)) {
            if (proto !== arrProto) {
                throw new TypeError("worker message arrays must be plain Arrays");
            }
            seen.add(v);
            for (let i = 0; i < v.length; i++) {
                check(v[i], depth + 1);
            }
            seen.delete(v);
            return;
        }
        if (proto !== objProto && proto !== null) {
            throw new TypeError("worker message cannot transfer host handles or class instances");
        }
        seen.add(v);
        const keys = Object.keys(v);
        for (let i = 0; i < keys.length; i++) {
            const val = v[keys[i]];
            if (val !== undefined) {
                check(val, depth + 1);
            }
        }
        seen.delete(v);
    }
    if (value === undefined) {
        throw new TypeError("worker message payload cannot be undefined");
    }
    check(value, 0);
    const encoded = JSON.stringify(value);
    if (typeof encoded !== "string" || encoded.length > maxBytes) {
        throw new RangeError("worker message exceeds byte limit");
    }
    return encoded;
})"#;

/// Single-threaded isolated QuickJS worker instance shared by both live threads
/// and deterministic preview lockstep stepping.
pub struct WorkerEngine {
    runtime: JsRuntime,
    cancel: Arc<AtomicBool>,
    outbound: Rc<RefCell<VecDeque<String>>>,
    outbound_overflow: Rc<Cell<bool>>,
    virtual_now_ms: Option<Rc<Cell<u64>>>,
    max_jobs_per_turn: usize,
    _lease: WorkerMemoryLease,
}

impl WorkerEngine {
    pub fn initialize(
        config: &WorkerConfig,
        cancel: Arc<AtomicBool>,
        lease: WorkerMemoryLease,
    ) -> Result<(Self, Vec<WorkerEvent>), WorkerError> {
        let heap = config
            .heap_limit_bytes
            .clamp(MIN_WORKER_HEAP_BYTES, DEFAULT_WORKER_HEAP_BYTES);
        let limits = RuntimeLimits {
            entry_timeout: config.turn_timeout,
            bootstrap_timeout: config.init_timeout,
            cleanup_timeout: Duration::from_millis(20),
            max_jobs_per_pump: max_jobs_per_turn_compat(config.max_jobs_per_turn),
            job_timeout: config.turn_timeout,
            memory_limit_bytes: heap,
            stack_limit_bytes: WORKER_STACK_BYTES,
            diagnostic_timeout: Duration::from_millis(5),
            max_diagnostic_bytes: 8 * 1024,
            max_tracked_rejections: 64,
        };
        let mut runtime = JsRuntime::with_cancel_handle(limits, Arc::clone(&cancel))
            .map_err(WorkerError::from_diagnostic)?;
        let outbound = Rc::new(RefCell::new(VecDeque::new()));
        let outbound_overflow = Rc::new(Cell::new(false));
        let virtual_now_ms = config
            .deterministic_env
            .map(|env| Rc::new(Cell::new(env.initial_epoch_ms)));

        let out_ref = Rc::clone(&outbound);
        let overflow_ref = Rc::clone(&outbound_overflow);
        let det_env = config.deterministic_env;
        let now_ref = virtual_now_ms.clone();

        runtime
            .with_context(move |ctx| {
                install_worker_globals(&ctx, out_ref, overflow_ref, det_env, now_ref)
            })
            .map_err(WorkerError::from_diagnostic)?;

        if let Some(bytecode) = &config.bytecode {
            runtime.load_bytecode_module(bytecode)
        } else {
            runtime.load_module(&config.module_id, &config.source)
        }
        .map_err(WorkerError::from_diagnostic)?;

        let mut engine = Self {
            runtime,
            cancel,
            outbound,
            outbound_overflow,
            virtual_now_ms,
            max_jobs_per_turn: config.max_jobs_per_turn,
            _lease: lease,
        };
        let events = engine.pump_and_collect();
        Ok((engine, events))
    }

    pub fn set_virtual_now_ms(&self, now_ms: u64) {
        if let Some(cell) = &self.virtual_now_ms {
            cell.set(now_ms);
        }
    }

    pub fn deliver_message(&mut self, json_payload: &str) -> Vec<WorkerEvent> {
        if self.cancel.load(Ordering::Acquire) {
            return vec![WorkerEvent::Terminated];
        }
        let payload = json_payload.to_owned();
        let invoke = self.runtime.with_module(move |ctx, module| {
            let parsed: Value<'_> = ctx.json_parse(payload.as_bytes())?;
            if let Ok(handler) = module.get::<_, Function<'_>>("onMessage") {
                handler.call::<_, ()>((parsed,))?;
                return Ok(());
            }
            if let Ok(handler) = ctx.globals().get::<_, Function<'_>>("onmessage") {
                handler.call::<_, ()>((parsed,))?;
                return Ok(());
            }
            Err(Exception::throw_type(
                &ctx,
                "worker must export onMessage(payload) or assign self.onmessage",
            ))
        });

        let mut events = Vec::new();
        if let Err(diag) = invoke {
            if self.cancel.load(Ordering::Acquire) {
                return vec![WorkerEvent::Terminated];
            }
            events.push(WorkerEvent::Error(WorkerError::from_diagnostic(diag)));
        }
        events.extend(self.pump_and_collect());
        events
    }

    fn pump_and_collect(&mut self) -> Vec<WorkerEvent> {
        let mut events = Vec::new();
        if self.cancel.load(Ordering::Acquire) {
            self.outbound.borrow_mut().clear();
            return vec![WorkerEvent::Terminated];
        }
        let pump = self.runtime.pump_jobs();
        if self.cancel.load(Ordering::Acquire) {
            self.outbound.borrow_mut().clear();
            return vec![WorkerEvent::Terminated];
        }
        for msg in self.outbound.borrow_mut().drain(..) {
            events.push(WorkerEvent::Message(msg));
        }
        if self.outbound_overflow.replace(false) {
            events.push(WorkerEvent::Error(WorkerError::new(
                "E_BACKPRESSURE",
                "worker outbound mailbox capacity exceeded",
            )));
        }
        for diag in pump.diagnostics {
            events.push(WorkerEvent::Error(WorkerError::from_diagnostic(diag)));
        }
        if pump.budget_exhausted && pump.has_pending_jobs && self.max_jobs_per_turn > 0 {
            events.push(WorkerEvent::Error(WorkerError::new(
                "E_WORKER_TIMEOUT",
                "worker turn microtask budget exceeded",
            )));
        }
        if let Err(diag) = self.runtime.drain_rejections(RejectionPolicy::Strict) {
            events.push(WorkerEvent::Error(WorkerError::from_diagnostic(diag)));
        }
        events
    }
}

fn max_jobs_per_turn_compat(value: usize) -> usize {
    value.max(1)
}

fn install_worker_globals<'js>(
    ctx: &Ctx<'js>,
    outbound: Rc<RefCell<VecDeque<String>>>,
    outbound_overflow: Rc<Cell<bool>>,
    deterministic_env: Option<DeterministicWorkerEnv>,
    virtual_now_ms: Option<Rc<Cell<u64>>>,
) -> rquickjs::Result<()> {
    let globals = ctx.globals();

    let post_fn = Func::from(move |ctx: Ctx<'js>, payload: Value<'js>| -> rquickjs::Result<()> {
        let raw = serialize_plain_js_payload(&ctx, payload)?;
        validate_json_payload(&raw)
            .map_err(|err| Exception::throw_range(&ctx, &err.message))?;
        let mut queue = outbound.borrow_mut();
        if queue.len() >= OUTBOUND_MAILBOX_CAPACITY {
            outbound_overflow.set(true);
            return Err(Exception::throw_range(
                &ctx,
                "worker outbound mailbox capacity exceeded",
            ));
        }
        queue.push_back(raw);
        Ok(())
    });
    globals.set("postMessage", post_fn)?;
    globals.set("self", globals.clone())?;

    // Minimal bounded console for worker diagnostics; never grants host capabilities.
    let console = Object::new(ctx.clone())?;
    for level in ["log", "info", "warn", "error", "debug"] {
        console.set(level, Func::from(move || -> rquickjs::Result<()> { Ok(()) }))?;
    }
    globals.set("console", console)?;

    if let Some(env) = deterministic_env {
        let now_cell = virtual_now_ms.expect("virtual_now_ms accompanies deterministic_env");
        let date_obj: Object<'js> = globals.get("Date")?;
        date_obj.set(
            "now",
            Func::from(move || -> f64 { now_cell.get() as f64 }),
        )?;
        let rng_state = Rc::new(Cell::new(env.rng_seed ^ 0x9e37_79b9_7f4a_7c15));
        let math_obj: Object<'js> = globals.get("Math")?;
        math_obj.set(
            "random",
            Func::from(move || -> f64 {
                let mut z = rng_state.get().wrapping_add(0x9e37_79b9_7f4a_7c15);
                rng_state.set(z);
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                let bits = (z ^ (z >> 31)) >> 11;
                (bits as f64) / ((1u64 << 53) as f64)
            }),
        )?;
    }

    Ok(())
}

/// Live OS-thread-backed isolated compute worker handle.
/// Dropping or terminating closes admission immediately, triggers QuickJS
/// interrupt, and never blocks the UI thread on `thread::join`.
pub struct LiveWorkerHandle {
    inbound: Option<SyncSender<String>>,
    outbound: Option<Receiver<WorkerEvent>>,
    cancel: Arc<AtomicBool>,
    closed: bool,
}

impl LiveWorkerHandle {
    pub fn spawn(
        config: WorkerConfig,
        budget: &WorkerSessionMemoryBudget,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, WorkerError> {
        let heap = config
            .heap_limit_bytes
            .clamp(MIN_WORKER_HEAP_BYTES, DEFAULT_WORKER_HEAP_BYTES);
        let lease = budget.try_reserve(heap)?;
        let (in_tx, in_rx) = mpsc::sync_channel::<String>(INBOUND_MAILBOX_CAPACITY);
        let (out_tx, out_rx) = mpsc::sync_channel::<WorkerEvent>(OUTBOUND_MAILBOX_CAPACITY);
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_cancel = Arc::clone(&cancel);
        let thread_name = format!("zgui-worker-{}", config.name);

        let spawned = thread::Builder::new().name(thread_name).spawn(move || {
            run_worker_thread(config, thread_cancel, lease, in_rx, out_tx, wake);
        });
        if let Err(err) = spawned {
            return Err(WorkerError::new(
                "E_RESOURCE_LIMIT",
                format!("failed to spawn worker thread: {err}"),
            ));
        }

        Ok(Self {
            inbound: Some(in_tx),
            outbound: Some(out_rx),
            cancel,
            closed: false,
        })
    }

    pub fn post_message(&mut self, json_payload: String) -> Result<(), WorkerError> {
        if self.closed {
            return Err(WorkerError::new("E_WORKER_CLOSED", "worker is terminated"));
        }
        validate_json_payload(&json_payload)?;
        let Some(tx) = &self.inbound else {
            return Err(WorkerError::new("E_WORKER_CLOSED", "worker is terminated"));
        };
        match tx.try_send(json_payload) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(WorkerError::new(
                "E_BACKPRESSURE",
                "worker inbound mailbox is full",
            )),
            Err(TrySendError::Disconnected(_)) => {
                self.closed = true;
                self.inbound = None;
                Err(WorkerError::new("E_WORKER_CLOSED", "worker has exited"))
            }
        }
    }

    pub fn try_recv(&mut self) -> Option<WorkerEvent> {
        if self.closed {
            return None;
        }
        let rx = self.outbound.as_ref()?;
        match rx.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.closed = true;
                self.inbound = None;
                self.outbound = None;
                Some(WorkerEvent::Terminated)
            }
        }
    }

    pub fn terminate(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.cancel.store(true, Ordering::Release);
        // Drop channels immediately so the worker thread unblocks and exits detached.
        self.inbound = None;
        self.outbound = None;
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Drop for LiveWorkerHandle {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn run_worker_thread(
    config: WorkerConfig,
    cancel: Arc<AtomicBool>,
    lease: WorkerMemoryLease,
    in_rx: Receiver<String>,
    out_tx: SyncSender<WorkerEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let send_event = |event: WorkerEvent| -> bool {
        if cancel.load(Ordering::Acquire) {
            return false;
        }
        match out_tx.try_send(event) {
            Ok(()) => {
                wake();
                true
            }
            Err(TrySendError::Full(_)) => {
                let _ = out_tx.try_send(WorkerEvent::Error(WorkerError::new(
                    "E_BACKPRESSURE",
                    "worker outbound mailbox is full",
                )));
                wake();
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    };

    let (mut engine, init_events) = match WorkerEngine::initialize(&config, Arc::clone(&cancel), lease) {
        Ok(pair) => pair,
        Err(err) => {
            let _ = send_event(WorkerEvent::Error(err));
            return;
        }
    };
    for event in init_events {
        if !send_event(event) {
            return;
        }
    }

    while let Ok(payload) = in_rx.recv() {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        let events = engine.deliver_message(&payload);
        for event in events {
            if matches!(event, WorkerEvent::Terminated) || !send_event(event) {
                return;
            }
        }
    }
}

/// Deterministic lockstep compute worker for preview mode.
/// Executes the exact same verified worker module inside an isolated QuickJS
/// `WorkerEngine` without spawning OS threads.
pub struct LockstepWorker {
    config: Option<WorkerConfig>,
    engine: Option<WorkerEngine>,
    lease: Option<WorkerMemoryLease>,
    cancel: Arc<AtomicBool>,
    inbound: VecDeque<String>,
    outbound: VecDeque<WorkerEvent>,
    initialized: bool,
    closed: bool,
}

impl LockstepWorker {
    pub fn new(
        config: WorkerConfig,
        budget: &WorkerSessionMemoryBudget,
    ) -> Result<Self, WorkerError> {
        let heap = config
            .heap_limit_bytes
            .clamp(MIN_WORKER_HEAP_BYTES, DEFAULT_WORKER_HEAP_BYTES);
        let lease = budget.try_reserve(heap)?;
        Ok(Self {
            config: Some(config),
            engine: None,
            lease: Some(lease),
            cancel: Arc::new(AtomicBool::new(false)),
            inbound: VecDeque::new(),
            outbound: VecDeque::new(),
            initialized: false,
            closed: false,
        })
    }

    pub fn post_message(&mut self, json_payload: String) -> Result<(), WorkerError> {
        if self.closed {
            return Err(WorkerError::new("E_WORKER_CLOSED", "worker is terminated"));
        }
        validate_json_payload(&json_payload)?;
        if self.inbound.len() >= INBOUND_MAILBOX_CAPACITY {
            return Err(WorkerError::new(
                "E_BACKPRESSURE",
                "worker inbound mailbox is full",
            ));
        }
        self.inbound.push_back(json_payload);
        Ok(())
    }

    pub fn has_pending_quantum(&self) -> bool {
        !self.closed && (!self.initialized || !self.inbound.is_empty())
    }

    pub fn has_outbound(&self) -> bool {
        !self.closed && !self.outbound.is_empty()
    }

    /// Executes at most one deterministic worker quantum (initialization or one inbound message).
    pub fn step_quantum(&mut self, virtual_now_ms: u64) -> bool {
        if self.closed || self.cancel.load(Ordering::Acquire) {
            return false;
        }
        if !self.initialized {
            self.initialized = true;
            let Some(mut config) = self.config.take() else {
                return false;
            };
            let Some(lease) = self.lease.take() else {
                return false;
            };
            if let Some(env) = &mut config.deterministic_env {
                env.initial_epoch_ms = virtual_now_ms;
            }
            match WorkerEngine::initialize(&config, Arc::clone(&self.cancel), lease) {
                Ok((engine, events)) => {
                    self.engine = Some(engine);
                    self.push_outbound(events);
                }
                Err(err) => {
                    self.push_outbound(vec![WorkerEvent::Error(err), WorkerEvent::Terminated]);
                    self.closed = true;
                }
            }
            return true;
        }
        let Some(payload) = self.inbound.pop_front() else {
            return false;
        };
        let Some(engine) = self.engine.as_mut() else {
            return false;
        };
        engine.set_virtual_now_ms(virtual_now_ms);
        let events = engine.deliver_message(&payload);
        self.push_outbound(events);
        true
    }

    fn push_outbound(&mut self, events: Vec<WorkerEvent>) {
        for event in events {
            if self.outbound.len() >= OUTBOUND_MAILBOX_CAPACITY {
                self.outbound.pop_back();
                self.outbound.push_back(WorkerEvent::Error(WorkerError::new(
                    "E_BACKPRESSURE",
                    "worker outbound mailbox is full",
                )));
                break;
            }
            self.outbound.push_back(event);
        }
    }

    pub fn try_recv(&mut self) -> Option<WorkerEvent> {
        if self.closed && self.outbound.is_empty() {
            return None;
        }
        self.outbound.pop_front()
    }

    pub fn terminate(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.cancel.store(true, Ordering::Release);
        self.inbound.clear();
        self.outbound.clear();
        self.config = None;
        self.engine = None;
        self.lease = None;
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Drop for LockstepWorker {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn live_worker_runs_isolated_and_exchanges_bounded_messages() {
        let budget = WorkerSessionMemoryBudget::default();
        let config = WorkerConfig::new(
            "calc",
            "workers/calc.mjs",
            r#"
                export function onMessage(msg) {
                    const isolated =
                        typeof globalThis.__zgui === "undefined" &&
                        typeof globalThis.__zguiServices === "undefined" &&
                        typeof globalThis.__zguiDevState === "undefined" &&
                        typeof globalThis.setTimeout === "undefined";
                    postMessage({ sum: msg.a + msg.b, isolated });
                }
            "#,
        );
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let mut handle = LiveWorkerHandle::spawn(config, &budget, wake).expect("spawn worker");
        handle
            .post_message(r#"{"a":19,"b":23}"#.into())
            .expect("post message");

        let deadline = Instant::now() + Duration::from_secs(3);
        let reply = loop {
            if let Some(event) = handle.try_recv() {
                break event;
            }
            assert!(Instant::now() < deadline, "timed out waiting for worker reply");
            thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(reply, WorkerEvent::Message(r#"{"sum":42,"isolated":true}"#.into()));
        handle.terminate();
    }

    #[test]
    fn live_worker_terminate_interrupts_infinite_loop_without_blocking() {
        let budget = WorkerSessionMemoryBudget::default();
        let mut config = WorkerConfig::new(
            "spin",
            "workers/spin.mjs",
            r#"export function onMessage() { for (;;) {} }"#,
        );
        config.turn_timeout = Duration::from_secs(10);
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let mut handle = LiveWorkerHandle::spawn(config, &budget, wake).expect("spawn spin worker");
        handle.post_message("{}".into()).expect("post spin");
        thread::sleep(Duration::from_millis(20));

        let start = Instant::now();
        handle.terminate();
        assert!(start.elapsed() < Duration::from_millis(50), "terminate blocked UI thread");

        // Detached thread should observe cancel, exit, and release memory budget promptly.
        let deadline = Instant::now() + Duration::from_secs(2);
        while budget.reserved_bytes() != 0 {
            assert!(Instant::now() < deadline, "detached worker did not exit and release budget");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn lockstep_worker_is_deterministic_and_enforces_limits() {
        let budget = WorkerSessionMemoryBudget::new(512 * 1024);
        let mut config = WorkerConfig::new(
            "det",
            "workers/det.mjs",
            r#"
                export function onMessage(msg) {
                    postMessage({
                        seq: msg.seq,
                        now: Date.now(),
                        rand: Math.floor(Math.random() * 1000000),
                    });
                }
            "#,
        );
        config.heap_limit_bytes = 512 * 1024;
        config.deterministic_env = Some(DeterministicWorkerEnv {
            initial_epoch_ms: 1_700_000_000_000,
            rng_seed: 42,
        });

        let mut w1 = LockstepWorker::new(config.clone(), &budget).expect("w1");
        // Aggregate session cap (512 KiB) blocks a second worker immediately.
        let err = LockstepWorker::new(config.clone(), &budget).err().expect("cap");
        assert_eq!(err.code, "E_RESOURCE_LIMIT");

        assert!(w1.step_quantum(1_700_000_000_000));
        w1.post_message(r#"{"seq":1}"#.into()).expect("post");
        assert!(w1.step_quantum(1_700_000_000_500));
        let ev1 = w1.try_recv().expect("ev1");
        w1.terminate();
        assert_eq!(budget.reserved_bytes(), 0);

        // Re-running with the same seed and timestamps yields byte-identical output.
        let mut w2 = LockstepWorker::new(config, &budget).expect("w2");
        assert!(w2.step_quantum(1_700_000_000_000));
        w2.post_message(r#"{"seq":1}"#.into()).expect("post");
        assert!(w2.step_quantum(1_700_000_000_500));
        let ev2 = w2.try_recv().expect("ev2");
        assert_eq!(ev1, ev2);
    }

    #[test]
    fn worker_enforces_mailbox_and_payload_limits() {
        let budget = WorkerSessionMemoryBudget::default();
        let config = WorkerConfig::new("echo", "workers/echo.mjs", "export function onMessage(x) { postMessage(x); }");
        let mut worker = LockstepWorker::new(config, &budget).expect("worker");
        let oversized = format!("\"{}\"", "a".repeat(MAX_MESSAGE_BYTES + 16));
        assert_eq!(worker.post_message(oversized).unwrap_err().code, "E_PAYLOAD_INVALID");

        for i in 0..INBOUND_MAILBOX_CAPACITY {
            worker.post_message(format!("{i}")).expect("within cap");
        }
        assert_eq!(worker.post_message("999".into()).unwrap_err().code, "E_BACKPRESSURE");
    }
}
