//! Owner-thread QuickJS-NG harness, not a browser or a hostile-code sandbox.
//!
//! Engine access is explicit and synchronous. Native callbacks must use their supplied
//! `Ctx`, stay short, and never re-enter this owner. Interrupts cannot preempt Rust code.
//! Hosts wrap each `execute_job` in their structural turn, enforce the count and whole
//! pump time from `limits`, and arrange a wake when `is_job_pending` remains true.
//! Drain rejection updates after entries/jobs, outside any context entry. Updates are
//! owned and keep stable IDs when a rejection is later handled. Drain again after
//! diagnostic formatting, which can itself run getters and produce new rejections.
//! Dropping the owner unregisters tracking and releases all tracker/module roots before
//! the context and runtime; it does not invoke application disposal or pump more jobs.

#![deny(unsafe_code)]

mod web_globals;

use rquickjs::{
    Context, Ctx, Function, Module, Object, Persistent, Runtime, Value, WriteOptions,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fmt,
    path::Path,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Initial desktop host policy, not application performance targets.
#[derive(Clone, Debug)]
pub struct RuntimeLimits {
    /// Maximum synchronous script/callback/job execution: 100 ms in release (250 ms in debug).
    pub entry_timeout: Duration,
    /// Whole smoke bootstrap, including module jobs and runSmoke: 2 s by default.
    pub bootstrap_timeout: Duration,
    /// Bounded disposal entry: 20 ms in release (200 ms in debug), independent of normal entries.
    pub cleanup_timeout: Duration,
    /// Maximum attempted jobs per pump, including throwing jobs: 256 by default.
    pub max_jobs_per_pump: usize,
    /// Whole pump deadline, also applied inside each job: 100 ms in release (250 ms in debug).
    pub job_timeout: Duration,
    /// Engine allocator limit: 64 MiB. Does not cover native/GPU/Rust allocations.
    /// Requires the default allocator; do not enable rquickjs rust-alloc/allocator.
    pub memory_limit_bytes: usize,
    /// Engine stack limit: 1 MiB by default.
    pub stack_limit_bytes: usize,
    /// Exception-property access has its own bounded formatting entry: 5 ms.
    pub diagnostic_timeout: Duration,
    /// Maximum bytes retained per message/stack, truncated on a UTF-8 boundary.
    pub max_diagnostic_bytes: usize,
    /// Maximum rejection identities retained for later-handled updates: 256.
    /// Oldest identities are evicted on overflow, reported by `drain_rejections`.
    pub max_tracked_rejections: usize,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        let default_turn_timeout = Duration::from_millis(if cfg!(debug_assertions) { 250 } else { 100 });
        Self {
            entry_timeout: default_turn_timeout,
            bootstrap_timeout: Duration::from_secs(2),
            cleanup_timeout: Duration::from_millis(if cfg!(debug_assertions) { 200 } else { 20 }),
            max_jobs_per_pump: 256,
            job_timeout: default_turn_timeout,
            memory_limit_bytes: 64 * 1024 * 1024,
            stack_limit_bytes: 1024 * 1024,
            diagnostic_timeout: Duration::from_millis(5),
            max_diagnostic_bytes: 16 * 1024,
            max_tracked_rejections: 256,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticKind {
    Engine,
    Exception,
    Interrupted,
    BudgetExceeded,
    PendingModule,
    InvalidLimits,
    Io,
}

/// An owned diagnostic; no JS value, context, or runtime reference escapes in errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub phase: &'static str,
    pub message: String,
    pub stack: Option<String>,
    /// Original generated stack, retained by a host when source mapping changes `stack`.
    pub generated_stack: Option<String>,
}

impl Diagnostic {
    fn new(kind: DiagnosticKind, phase: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            phase,
            message: message.into(),
            stack: None,
            generated_stack: None,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.phase, self.message)?;
        if let Some(stack) = &self.stack {
            write!(f, "\n{stack}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostic {}

struct InterruptState {
    deadline: Cell<Option<Instant>>,
    interrupted: Cell<bool>,
    external_cancel: Arc<AtomicBool>,
}

impl InterruptState {
    fn new(external_cancel: Arc<AtomicBool>) -> Self {
        Self {
            deadline: Cell::new(None),
            interrupted: Cell::new(false),
            external_cancel,
        }
    }
}

struct EntryGuard<'a>(&'a InterruptState);

impl Drop for EntryGuard<'_> {
    fn drop(&mut self) {
        self.0.deadline.set(None);
    }
}

/// Result of a single bounded turn. The host must schedule a real follow-up wake
/// when `has_pending_jobs` is true; the harness never spins a UI event loop.
#[derive(Debug)]
pub struct JobPump {
    pub attempted_jobs: usize,
    pub has_pending_jobs: bool,
    pub budget_exhausted: bool,
    pub diagnostics: Vec<Diagnostic>,
}

/// An owned rejection state transition. An ID remains stable until handling or
/// capacity eviction. The reason's stack retains its original generated source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectionUpdate {
    pub id: u64,
    pub handled: bool,
    pub diagnostic: Diagnostic,
}

/// Policy controlling how [`JsRuntime::drain_rejections`] handles net unhandled
/// promise rejections and tracking capacity overflows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectionPolicy {
    /// Fail on the earliest net-unhandled rejection, tracking capacity eviction,
    /// or non-quiescing diagnostic getters.
    Strict,
    /// Drain all net rejection transitions (`handled: false` and cross-batch
    /// `handled: true`) to getter quiescence without failing on unhandled rejections.
    Log,
}

/// Drain result. Evicted identities cannot report later handling; hosts should
/// surface `dropped_rejections` rather than silently losing this observability.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct RejectionDrain {
    pub updates: Vec<RejectionUpdate>,
    pub dropped_rejections: usize,
}

/// Borrowed single-entry handle for executing one bounded JavaScript entry inside
/// a Bridge turn. Intentionally omits between-turn operations such as
/// [`JsRuntime::drain_rejections`], [`JsRuntime::load_module`], and
/// [`JsRuntime::pump_jobs`].
#[must_use]
pub struct JsTurn<'a>(&'a mut JsRuntime);

impl fmt::Debug for JsTurn<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsTurn").finish_non_exhaustive()
    }
}

impl JsTurn<'_> {
    pub fn with_context<T>(
        &mut self,
        f: impl for<'js> FnOnce(Ctx<'js>) -> rquickjs::Result<T>,
    ) -> Result<T, Diagnostic> {
        self.0.with_context(f)
    }

    pub fn with_module<T>(
        &mut self,
        action: impl for<'js> FnOnce(Ctx<'js>, Object<'js>) -> rquickjs::Result<T>,
    ) -> Result<T, Diagnostic> {
        self.0.with_module(action)
    }

    pub fn with_cleanup_context<T>(
        &mut self,
        f: impl for<'js> FnOnce(Ctx<'js>) -> rquickjs::Result<T>,
    ) -> Result<T, Diagnostic> {
        self.0.with_cleanup_context(f)
    }

    pub fn execute_job(&mut self, budget: Duration) -> Result<bool, Diagnostic> {
        self.0.execute_job(budget)
    }

    pub fn evaluate_script(&mut self, source: &str) -> Result<String, Diagnostic> {
        self.0.evaluate_script(source)
    }
}

struct TrackedRejection {
    id: u64,
    promise: Persistent<Value<'static>>,
    reason: Option<Persistent<Value<'static>>>,
    diagnostic: Option<Diagnostic>,
    handled: bool,
}

#[derive(Default)]
struct RejectionTracker {
    next_id: u64,
    records: VecDeque<TrackedRejection>,
    // At most two transitions per retained identity; no JS runs while borrowed.
    pending: VecDeque<(u64, bool)>,
    dropped: usize,
}

impl RejectionTracker {
    fn track<'js>(&mut self, ctx: &Ctx<'js>, promise: Value<'js>, reason: Value<'js>, handled: bool, max: usize) {
        let promise = Persistent::save(ctx, promise);
        if let Some(index) = self.records.iter().position(|record| record.promise == promise) {
            if handled && !self.records[index].handled {
                let id = self.records[index].id;
                if self.records[index].diagnostic.is_none()
                    && self.pending.iter().any(|&(pending_id, is_handled)| pending_id == id && !is_handled)
                {
                    self.records.remove(index);
                    self.pending.retain(|&(pending_id, _)| pending_id != id);
                } else {
                    self.records[index].handled = true;
                    self.pending.push_back((id, true));
                }
            }
            return;
        }
        if handled {
            return;
        }
        if self.records.len() == max {
            let oldest = self.records.pop_front().expect("nonzero tracking capacity");
            self.pending.retain(|(id, _)| *id != oldest.id);
            self.dropped = self.dropped.saturating_add(1);
        }
        self.next_id = self.next_id.checked_add(1).expect("rejection identity exhausted");
        let id = self.next_id;
        self.records.push_back(TrackedRejection {
            id,
            promise,
            reason: Some(Persistent::save(ctx, reason)),
            diagnostic: None,
            handled: false,
        });
        self.pending.push_back((id, false));
    }
}

/// Wire format identifier sealed in `manifest.json` for compiled QuickJS bytecode packages.
pub const BYTECODE_FORMAT: &str = "qjs-bytecode-v1";

/// Deterministic QuickJS bytecode ABI tag tying pinned engine revision, target architecture,
/// endianness, and pointer width so incompatible `.qjsb` payloads fail closed before loading.
#[must_use]
pub fn quickjs_abi_tag() -> String {
    let endian = if cfg!(target_endian = "little") { "le" } else { "be" };
    format!(
        "qjs-ng-6d46d07d-rquickjs-73fcd606-{}-{endian}-{}",
        std::env::consts::ARCH,
        usize::BITS,
    )
}

struct CompileDirResolver {
    root: std::path::PathBuf,
}

impl Resolver for CompileDirResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let base_dir = base.rsplit_once('/').map_or("", |(dir, _)| dir);
        let mut parts: Vec<&str> = base_dir
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        for seg in name.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Err(rquickjs::Error::new_resolving_message(
                            base,
                            name,
                            "module specifier escapes compile root",
                        ));
                    }
                }
                other => parts.push(other),
            }
        }
        let normalized = parts.join("/");
        let full = self.root.join(&normalized);
        if full.is_file() {
            Ok(normalized)
        } else {
            Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                format!("sibling chunk not found in compile root: {normalized}"),
            ))
        }
    }
}

struct CompileDirLoader {
    root: std::path::PathBuf,
}

impl Loader for CompileDirLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        let full = self.root.join(name);
        let source = std::fs::read_to_string(&full).map_err(|err| {
            rquickjs::Error::new_loading_message(name, format!("failed to read {name}: {err}"))
        })?;
        Module::declare(ctx.clone(), name, source)
    }
}

/// Compile an ESM module source into deterministic QuickJS bytecode while retaining debug
/// line/column tables so sidecar source-map diagnostic remapping stays accurate.
pub fn compile_module_bytecode(name: &str, source: &str) -> Result<Vec<u8>, Diagnostic> {
    compile_module_bytecode_in_dir(name, source, None)
}

/// Compile an ESM module source into deterministic QuickJS bytecode, optionally resolving
/// sibling static imports inside `source_dir` during `Module::declare`.
pub fn compile_module_bytecode_in_dir(
    name: &str,
    source: &str,
    source_dir: Option<&Path>,
) -> Result<Vec<u8>, Diagnostic> {
    let runtime = Runtime::new().map_err(|e| {
        Diagnostic::new(DiagnosticKind::Engine, "bytecode compile", e.to_string())
    })?;
    if let Some(dir) = source_dir {
        let root = dir.to_path_buf();
        runtime.set_loader(
            CompileDirResolver { root: root.clone() },
            CompileDirLoader { root },
        );
    }
    let context = Context::full(&runtime).map_err(|e| {
        Diagnostic::new(DiagnosticKind::Engine, "bytecode compile", e.to_string())
    })?;
    context.with(|ctx| {
        let module = Module::declare(ctx.clone(), name, source).map_err(|err| {
            let detail = match err {
                rquickjs::Error::Exception => {
                    let caught = ctx.catch();
                    format!("exception while compiling {name}: {caught:?}")
                }
                other => other.to_string(),
            };
            Diagnostic::new(DiagnosticKind::Exception, "bytecode compile", detail)
        })?;
        module
            .write(WriteOptions {
                strip_source: false,
                strip_debug: false,
                ..WriteOptions::default()
            })
            .map_err(|err| {
                Diagnostic::new(DiagnosticKind::Engine, "bytecode compile", err.to_string())
            })
    })
}

/// Compile a verified ESM module strictly from in-memory [`VerifiedModuleProvider`] bytes
/// (zero disk reads, eliminating TOCTOU) while resolving static graph edges through the same provider.
pub fn compile_verified_module_bytecode(
    canonical_name: &str,
    provider: Rc<dyn VerifiedModuleProvider>,
) -> Result<Vec<u8>, Diagnostic> {
    let source = match provider.load(canonical_name) {
        Ok(VerifiedModulePayload::Source(source)) => source,
        Ok(VerifiedModulePayload::Bytecode(_)) => {
            return Err(Diagnostic::new(
                DiagnosticKind::Engine,
                "bytecode compile",
                format!("verified module {canonical_name} is already bytecode; expected source"),
            ));
        }
        Err(message) => {
            return Err(Diagnostic::new(
                DiagnosticKind::Exception,
                "bytecode compile",
                message,
            ));
        }
    };
    let runtime = Runtime::new().map_err(|e| {
        Diagnostic::new(DiagnosticKind::Engine, "bytecode compile", e.to_string())
    })?;
    let limits = RuntimeLimits::default();
    runtime.set_memory_limit(limits.memory_limit_bytes);
    runtime.set_max_stack_size(limits.stack_limit_bytes);
    runtime.set_loader(
        ProviderResolver {
            provider: Rc::clone(&provider),
        },
        ProviderLoader { provider },
    );
    let context = Context::full(&runtime).map_err(|e| {
        Diagnostic::new(DiagnosticKind::Engine, "bytecode compile", e.to_string())
    })?;
    context.with(|ctx| {
        let module = Module::declare(ctx.clone(), canonical_name, source).map_err(|err| {
            let detail = match err {
                rquickjs::Error::Exception => {
                    let caught = ctx.catch();
                    format!("exception while compiling {canonical_name}: {caught:?}")
                }
                other => other.to_string(),
            };
            Diagnostic::new(DiagnosticKind::Exception, "bytecode compile", detail)
        })?;
        module
            .write(WriteOptions {
                strip_source: false,
                strip_debug: false,
                ..WriteOptions::default()
            })
            .map_err(|err| {
                Diagnostic::new(DiagnosticKind::Engine, "bytecode compile", err.to_string())
            })
    })
}

#[allow(unsafe_code)]
fn load_verified_bytecode_module<'js>(
    ctx: Ctx<'js>,
    bytes: &[u8],
) -> rquickjs::Result<Module<'js, Declared>> {
    if bytes.is_empty() {
        return Err(rquickjs::Error::new_loading_message(
            "<bytecode>",
            "empty QuickJS bytecode payload",
        ));
    }
    // SAFETY: Callers must pass only QuickJS bytecode compiled for this exact `quickjs_abi_tag()`
    // and verified fail-closed via SHA-256 `artifactDigests` before reaching `JsRuntime`.
    // QuickJS-NG (`JS_READ_OBJ_ROM_DATA == 0`) copies the bytecode stream out of `bytes`.
    unsafe { Module::load(ctx, bytes) }
}

/// Verified module payload supplied by a host-owned [`VerifiedModuleProvider`].
#[derive(Clone, Debug)]
pub enum VerifiedModulePayload {
    Source(String),
    Bytecode(Vec<u8>),
}

/// Host-supplied closed-graph module provider backed strictly by verified
/// package bytes (never ambient filesystem, cwd, or network resolution).
pub trait VerifiedModuleProvider: 'static {
    /// Resolve `specifier` imported from `referrer` into a canonical verified module ID.
    fn resolve(&self, referrer: &str, specifier: &str) -> Result<String, String>;
    /// Load verified ESM source or ABI-verified bytecode for a canonical module ID.
    fn load(&self, canonical_name: &str) -> Result<VerifiedModulePayload, String>;
}

struct ProviderResolver {
    provider: Rc<dyn VerifiedModuleProvider>,
}

impl Resolver for ProviderResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        self.provider
            .resolve(base, name)
            .map_err(|msg| rquickjs::Error::new_resolving_message(base, name, msg))
    }
}

struct ProviderLoader {
    provider: Rc<dyn VerifiedModuleProvider>,
}

impl Loader for ProviderLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        match self
            .provider
            .load(name)
            .map_err(|msg| rquickjs::Error::new_loading_message(name, msg))?
        {
            VerifiedModulePayload::Source(source) => Module::declare(ctx.clone(), name, source),
            VerifiedModulePayload::Bytecode(bytes) => {
                load_verified_bytecode_module(ctx.clone(), &bytes)
            }
        }
    }
}

enum ModuleInput<'a> {
    Source { name: &'a str, source: &'a str },
    Bytecode(&'a [u8]),
}

pub struct JsRuntime {
    // Drop clears both module and tracker roots before context/runtime destruction.
    module: Option<Persistent<Object<'static>>>,
    rejections: Rc<RefCell<RejectionTracker>>,
    // Field declaration order is intentional: context releases before runtime.
    context: Context,
    runtime: Runtime,
    interrupt: Rc<InterruptState>,
    limits: RuntimeLimits,
    enclosing_deadline: Option<Rc<Cell<Instant>>>,
    max_entry_duration: Cell<Duration>,
}

impl JsRuntime {
    pub fn new(limits: RuntimeLimits) -> Result<Self, Diagnostic> {
        Self::with_cancel_handle(limits, Arc::new(AtomicBool::new(false)))
    }

    /// Creates a runtime wired to a caller-owned atomic cancellation flag so an
    /// owner thread can interrupt a background worker without blocking or joining.
    pub fn with_cancel_handle(
        limits: RuntimeLimits,
        external_cancel: Arc<AtomicBool>,
    ) -> Result<Self, Diagnostic> {
        let now = Instant::now();
        let durations = [
            limits.entry_timeout,
            limits.bootstrap_timeout,
            limits.cleanup_timeout,
            limits.job_timeout,
            limits.diagnostic_timeout,
        ];
        if durations
            .iter()
            .any(|d| d.is_zero() || now.checked_add(*d).is_none())
            || limits.max_jobs_per_pump == 0
            || limits.max_tracked_rejections == 0
            || limits.memory_limit_bytes == 0
            || limits.stack_limit_bytes == 0
            || limits.max_diagnostic_bytes == 0
        {
            return Err(Diagnostic::new(
                DiagnosticKind::InvalidLimits,
                "configuration",
                "limits must be nonzero and deadlines must fit Instant",
            ));
        }
        let runtime = Runtime::new().map_err(|e| {
            Diagnostic::new(DiagnosticKind::Engine, "runtime creation", e.to_string())
        })?;
        runtime.set_memory_limit(limits.memory_limit_bytes);
        runtime.set_max_stack_size(limits.stack_limit_bytes);
        let interrupt = Rc::new(InterruptState::new(external_cancel));
        let state = interrupt.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            let cancelled = state.external_cancel.load(Ordering::Acquire);
            let expired = state
                .deadline
                .get()
                .is_some_and(|end| Instant::now() >= end);
            if cancelled || expired {
                state.interrupted.set(true);
            }
            cancelled || expired
        })));
        let context = Context::full(&runtime).map_err(|e| {
            Diagnostic::new(DiagnosticKind::Engine, "context creation", e.to_string())
        })?;
        context.with(|ctx| {
            web_globals::install(&ctx).map_err(|error| {
                let message = if error.is_exception() {
                    format!("{:?}", ctx.catch())
                } else {
                    error.to_string()
                };
                Diagnostic::new(DiagnosticKind::Engine, "web globals installation", message)
            })
        })?;
        let rejections = Rc::new(RefCell::new(RejectionTracker::default()));
        let tracker = rejections.clone();
        let max = limits.max_tracked_rejections;
        runtime.set_host_promise_rejection_tracker(Some(Box::new(move |ctx, promise, reason, handled| {
            // Capture only: formatting here could recursively invoke this callback.
            // Persistent::save uses the supplied Ctx and never takes the runtime lock.
            tracker.borrow_mut().track(&ctx, promise, reason, handled, max);
        })));
        Ok(Self {
            module: None,
            rejections,
            context,
            runtime,
            interrupt,
            limits,
            enclosing_deadline: None,
            max_entry_duration: Cell::new(Duration::ZERO),
        })
    }

    /// Install a host-verified closed-graph module loader for static/dynamic ESM chunks.
    pub fn set_module_provider(&mut self, provider: Rc<dyn VerifiedModuleProvider>) {
        let resolver = ProviderResolver {
            provider: Rc::clone(&provider),
        };
        let loader = ProviderLoader { provider };
        self.runtime.set_loader(resolver, loader);
    }

    /// Returns a clone of the atomic cancellation flag wired to QuickJS's interrupt handler.
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.interrupt.external_cancel)
    }

    /// Returns whether external cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.interrupt.external_cancel.load(Ordering::Acquire)
    }

    /// Borrow this runtime exclusively for a single Bridge turn entry.
    pub fn turn(&mut self) -> JsTurn<'_> {
        JsTurn(self)
    }

    /// Enter the current context under the normal entry deadline. Install host
    /// functions here with `Function::new`/`Func`; callbacks receive this same Ctx.
    /// Return owned Rust data only. In particular, callers must not let Persistent
    /// values outlive this owner, or clone context/runtime handles out of it.
    /// This does not pump jobs; call `pump_jobs` after leaving the entry.
    pub fn with_context<T>(
        &mut self,
        f: impl for<'js> FnOnce(Ctx<'js>) -> rquickjs::Result<T>,
    ) -> Result<T, Diagnostic> {
        self.enter(
            "context entry",
            Instant::now() + self.limits.entry_timeout,
            f,
        )
    }

    /// Enter for explicit application disposal under the cleanup deadline.
    /// Throwing/interrupted disposal returns an owned diagnostic; the host must
    /// continue releasing native resources and persistent roots after an error.
    /// This does not pump jobs or automatically invoke a disposer when dropped.
    pub fn with_cleanup_context<T>(
        &mut self,
        f: impl for<'js> FnOnce(Ctx<'js>) -> rquickjs::Result<T>,
    ) -> Result<T, Diagnostic> {
        self.enter_deadline("cleanup", Instant::now() + self.limits.cleanup_timeout, f)
    }

    /// Current policy, including `max_jobs_per_pump` and whole-pump `job_timeout`.
    pub fn limits(&self) -> &RuntimeLimits {
        &self.limits
    }

    /// Clamp all JavaScript entries, jobs, and diagnostic accessors to the owner's
    /// current real deadline. Updating the cell changes the next entry's bound.
    pub fn set_enclosing_deadline(&mut self, deadline: Rc<Cell<Instant>>) {
        self.enclosing_deadline = Some(deadline);
    }

    fn clamp_deadline(&self, deadline: Instant) -> Instant {
        self.enclosing_deadline.as_ref().map_or(deadline, |outer| deadline.min(outer.get()))
    }

    /// Query only outside context entries/native callbacks, on the owner thread.
    pub fn is_job_pending(&self) -> bool {
        self.runtime.is_job_pending()
    }

    /// Attempt exactly one job outside every context entry. `true` means one
    /// successful attempt, `false` an empty queue, and `Err` one failed attempt.
    /// Both successful and failed attempts consume the caller's count budget.
    /// The interrupt deadline is the earlier of the normal entry limit and the
    /// supplied remaining whole-turn budget, including diagnostic accessor work.
    pub fn max_entry_duration(&self) -> Duration {
        self.max_entry_duration.get()
    }

    pub fn stack_limit_bytes(&self) -> usize {
        self.limits.stack_limit_bytes
    }

    pub fn memory_usage_bytes(&self) -> u64 {
        self.runtime.memory_usage().memory_used_size.max(0) as u64
    }

    fn record_entry_duration(&self, started: Instant) {
        let elapsed = started.elapsed();
        if elapsed > self.max_entry_duration.get() {
            self.max_entry_duration.set(elapsed);
        }
    }

    pub fn execute_job(&mut self, budget: Duration) -> Result<bool, Diagnostic> {
        self.interrupt.interrupted.set(false);
        let end = self.clamp_deadline(Instant::now() + budget.min(self.limits.entry_timeout));
        if Instant::now() >= end {
            return Err(Diagnostic::new(DiagnosticKind::BudgetExceeded, "pending job", "entry deadline exceeded"));
        }
        self.interrupt.deadline.set(Some(end));
        let _guard = EntryGuard(&self.interrupt);
        self.update_stack_top();
        let started = Instant::now();
        // The runtime guard is released before entering JobException's context.
        let result = self.runtime.execute_pending_job().map_err(|exception| {
            exception.0.with(|ctx| {
                self.capture_error(&ctx, "pending job", rquickjs::Error::Exception)
            })
        });
        self.record_entry_duration(started);
        result
    }

    /// Drain net promise rejection transitions to getter quiescence outside context
    /// entries and Bridge turns.
    ///
    /// Same-batch `(false -> true)` pairs cancel before returning. Cross-batch `handled: true`
    /// transitions reuse cached diagnostics without re-invoking JS getters. All quiescence
    /// passes share one clamped `diagnostic_timeout` budget and at most
    /// `max_tracked_rejections + 1` passes.
    ///
    /// # Errors
    /// Returns [`DiagnosticKind::BudgetExceeded`] if diagnostic getters fail to quiesce within
    /// `max_tracked_rejections + 1` passes, or under [`RejectionPolicy::Strict`] when tracking
    /// capacity overflowed; under [`RejectionPolicy::Strict`] also returns the earliest
    /// net-unhandled rejection diagnostic.
    pub fn drain_rejections(&mut self, policy: RejectionPolicy) -> Result<RejectionDrain, Diagnostic> {
        let end = self.clamp_deadline(Instant::now() + self.limits.diagnostic_timeout);
        let mut total_dropped = 0usize;
        let mut net_updates: Vec<RejectionUpdate> = Vec::new();
        for _ in 0..=self.limits.max_tracked_rejections {
            let (pending, snapshots, dropped) = {
                let mut tracker = self.rejections.borrow_mut();
                let pending = std::mem::take(&mut tracker.pending);
                let snapshots: Vec<_> = tracker
                    .records
                    .iter()
                    .filter(|record| pending.iter().any(|(id, _)| *id == record.id))
                    .map(|record| (record.id, record.reason.clone(), record.diagnostic.clone()))
                    .collect();
                (pending, snapshots, std::mem::take(&mut tracker.dropped))
            };
            total_dropped = total_dropped.saturating_add(dropped);
            if pending.is_empty() {
                break;
            }
            let needs_context = snapshots.iter().any(|(_, _, cached)| cached.is_none());
            let mut diagnostics = Vec::with_capacity(snapshots.len());
            if needs_context {
                self.interrupt.interrupted.set(false);
                self.interrupt.deadline.set(Some(end));
                let _guard = EntryGuard(&self.interrupt);
                self.update_stack_top();
                self.context.with(|ctx| {
                    for (id, reason, cached) in snapshots {
                        let diagnostic = cached.unwrap_or_else(|| {
                            let reason = reason
                                .expect("unformatted rejection has a reason")
                                .restore(&ctx)
                                .expect("tracker belongs to this runtime");
                            if Instant::now() >= end {
                                Diagnostic::new(
                                    DiagnosticKind::Exception,
                                    "promise rejection",
                                    truncate(
                                        format!("JavaScript threw {} (formatting budget exhausted)", reason.type_of().as_str()),
                                        self.limits.max_diagnostic_bytes,
                                    ),
                                )
                            } else {
                                diagnostic_value(&ctx, "promise rejection", reason, self.limits.max_diagnostic_bytes, end)
                            }
                        });
                        diagnostics.push((id, diagnostic));
                    }
                });
            } else {
                for (id, _, cached) in snapshots {
                    if let Some(diagnostic) = cached {
                        diagnostics.push((id, diagnostic));
                    }
                }
            }
            {
                let mut tracker = self.rejections.borrow_mut();
                for (id, diagnostic) in &diagnostics {
                    if let Some(record) = tracker.records.iter_mut().find(|record| record.id == *id) {
                        record.diagnostic = Some(diagnostic.clone());
                        record.reason = None;
                    }
                }
                tracker.records.retain(|record| !pending.iter().any(|(id, handled)| *id == record.id && *handled));
            }
            for (id, handled) in pending {
                let Some(diagnostic) = diagnostics.iter().find(|(key, _)| *key == id).map(|(_, d)| d.clone()) else {
                    continue;
                };
                if handled {
                    if let Some(pos) = net_updates.iter().position(|update| update.id == id && !update.handled) {
                        net_updates.remove(pos);
                    } else {
                        net_updates.push(RejectionUpdate { id, handled: true, diagnostic });
                    }
                } else {
                    net_updates.push(RejectionUpdate { id, handled: false, diagnostic });
                }
            }
        }
        if !self.rejections.borrow().pending.is_empty() {
            return Err(Diagnostic::new(
                DiagnosticKind::BudgetExceeded,
                "promise rejection",
                "rejection diagnostics did not quiesce",
            ));
        }
        if policy == RejectionPolicy::Strict {
            if total_dropped != 0 {
                return Err(Diagnostic::new(
                    DiagnosticKind::BudgetExceeded,
                    "promise rejection",
                    "rejection tracking limit exceeded",
                ));
            }
            if let Some(unhandled) = net_updates.iter().find(|update| !update.handled) {
                return Err(unhandled.diagnostic.clone());
            }
        }
        Ok(RejectionDrain {
            updates: net_updates,
            dropped_rejections: total_dropped,
        })
    }

    /// Evaluate a global script whose completion value must be a JS string.
    /// Exceptions are captured while the context is live. Jobs are not auto-pumped.
    pub fn evaluate_script(&mut self, source: &str) -> Result<String, Diagnostic> {
        self.enter(
            "script",
            Instant::now() + self.limits.entry_timeout,
            |ctx| ctx.eval(source),
        )
    }

    /// Declare/evaluate a self-contained ESM artifact, observe its evaluation
    /// Promise through the normal bounded job pump, then call synchronous runSmoke.
    /// No loader, host I/O, Promise::finish, or context-level job pump is used.
    /// An unsettled module with no runnable jobs is reported, never busy-waited.
    pub fn run_smoke_module(&mut self, name: &str, source: &str) -> Result<String, Diagnostic> {
        let bootstrap_end = Instant::now() + self.limits.bootstrap_timeout;
        let namespace = self.evaluate_module_namespace(
            ModuleInput::Source { name, source },
            bootstrap_end,
        )?;
        self.enter("runSmoke", bootstrap_end, |ctx| {
            let function: Function = namespace.restore(&ctx)?.get("runSmoke")?;
            function.call(())
        })
    }

    /// Evaluate a preflight module from UTF-8 ESM source. Its namespace remains owned by this runtime.
    pub fn load_module(&mut self, name: &str, source: &str) -> Result<(), Diagnostic> {
        let end = Instant::now() + self.limits.bootstrap_timeout;
        self.module = Some(self.evaluate_module_namespace(
            ModuleInput::Source { name, source },
            end,
        )?);
        Ok(())
    }

    /// Evaluate a preflight module from host-verified QuickJS bytecode.
    /// Callers must verify `bytecodeFormat`, `quickjsAbi`, and SHA-256 `artifactDigests` fail-closed first.
    pub fn load_bytecode_module(&mut self, bytecode: &[u8]) -> Result<(), Diagnostic> {
        let end = Instant::now() + self.limits.bootstrap_timeout;
        self.module = Some(self.evaluate_module_namespace(ModuleInput::Bytecode(bytecode), end)?);
        Ok(())
    }

    /// Access exports without allowing borrowed JS values out of the entry.
    pub fn with_module<T>(&mut self, action: impl for<'js> FnOnce(Ctx<'js>, Object<'js>) -> rquickjs::Result<T>) -> Result<T, Diagnostic> {
        let namespace = self.module.as_ref().cloned().ok_or_else(|| Diagnostic::new(DiagnosticKind::Engine, "module", "no application module loaded"))?;
        self.enter("module export", Instant::now() + self.limits.entry_timeout, |ctx| {
            let object = namespace.restore(&ctx)?;
            action(ctx, object)
        })
    }

    fn evaluate_module_namespace(
        &mut self,
        input: ModuleInput<'_>,
        bootstrap_end: Instant,
    ) -> Result<Persistent<Object<'static>>, Diagnostic> {
        let compile_end = self.clamp_deadline(bootstrap_end);
        if Instant::now() >= compile_end {
            return Err(Diagnostic::new(
                DiagnosticKind::BudgetExceeded,
                "module evaluation",
                "entry deadline exceeded",
            ));
        }
        self.interrupt.interrupted.set(false);
        self.interrupt.deadline.set(Some(compile_end));
        let (namespace, promise) = {
            let _guard = EntryGuard(&self.interrupt);
            self.update_stack_top();
            self.context.with(|ctx| {
                let module = match input {
                    ModuleInput::Source { name, source } => Module::declare(ctx.clone(), name, source),
                    ModuleInput::Bytecode(bytes) => load_verified_bytecode_module(ctx.clone(), bytes),
                }
                .map_err(|error| self.capture_error(&ctx, "module evaluation", error))?;
                let eval_end = self.clamp_deadline(bootstrap_end.min(Instant::now() + self.limits.entry_timeout));
                if Instant::now() >= eval_end {
                    return Err(Diagnostic::new(
                        DiagnosticKind::BudgetExceeded,
                        "module evaluation",
                        "entry deadline exceeded",
                    ));
                }
                self.interrupt.deadline.set(Some(eval_end));
                let started = Instant::now();
                let result = (|| -> rquickjs::Result<_> {
                    let (module, promise) = module.eval()?;
                    let namespace = module.namespace()?;
                    if self.interrupt.interrupted.get()
                        && let Some(Err(error)) = promise.result::<()>()
                    {
                        return Err(error);
                    }
                    Ok((
                        Persistent::save(&ctx, namespace),
                        Persistent::save(&ctx, promise),
                    ))
                })()
                .map_err(|error| self.capture_error(&ctx, "module evaluation", error));
                self.record_entry_duration(started);
                if self.interrupt.interrupted.get() && result.is_ok() {
                    return Err(Diagnostic::new(
                        DiagnosticKind::Interrupted,
                        "module evaluation",
                        "module evaluation interrupted",
                    ));
                }
                result
            })?
        };
        loop {
            let settled = self.enter("module settlement", bootstrap_end, |ctx| {
                promise.clone().restore(&ctx)?.result::<()>().transpose()
            })?;
            if settled.is_some() {
                break;
            }
            let turn = self.pump_jobs_until(bootstrap_end);
            if let Some(diagnostic) = turn.diagnostics.into_iter().next() {
                return Err(diagnostic);
            }
            if Instant::now() >= bootstrap_end {
                return Err(Diagnostic::new(
                    DiagnosticKind::BudgetExceeded,
                    "module settlement",
                    "bootstrap deadline exceeded",
                ));
            }
            if !turn.has_pending_jobs {
                // A final job may have settled the Promise and emptied the queue.
                let settled = self.enter("module settlement", bootstrap_end, |ctx| {
                    promise.clone().restore(&ctx)?.result::<()>().transpose()
                })?;
                if settled.is_some() {
                    break;
                }
                return Err(Diagnostic::new(
                    DiagnosticKind::PendingModule,
                    "module settlement",
                    "module Promise remains pending without runnable jobs; host I/O is unsupported",
                ));
            }
        }
        Ok(namespace)
    }

    pub fn run_smoke_module_file(&mut self, path: impl AsRef<Path>) -> Result<String, Diagnostic> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path).map_err(|e| {
            Diagnostic::new(
                DiagnosticKind::Io,
                "module read",
                format!("{}: {e}", path.display()),
            )
        })?;
        self.run_smoke_module(&path.to_string_lossy(), &source)
    }

    /// Pump at most the configured attempted-job count and elapsed time. Throwing
    /// jobs consume budget and their original exceptions are retained; later jobs
    /// can still run. Invoke only between context entries, on the owner thread.
    pub fn pump_jobs(&mut self) -> JobPump {
        self.pump_jobs_until(Instant::now() + self.limits.job_timeout)
    }

    /// Refresh QuickJS's stack baseline at outer host-to-JS entry boundaries.
    /// `rquickjs` gates its own `update_stack_top` on `feature = "parallel"`, which
    /// otherwise charges host event-loop Rust frames between `JsRuntime::new` and a
    /// later entry against `stack_limit_bytes`. Never call re-entrantly inside JS.
    #[allow(unsafe_code)]
    #[inline(always)]
    fn update_stack_top(&self) {
        // SAFETY: `self.context` is alive and owned by this single-threaded `JsRuntime`,
        // and callers invoke this only at outer host entry boundaries before entering JS.
        unsafe {
            rquickjs::qjs::JS_UpdateStackTop(self.context.get_runtime_ptr());
        }
    }

    fn enter<T>(
        &self,
        phase: &'static str,
        outer_end: Instant,
        f: impl for<'js> FnOnce(Ctx<'js>) -> rquickjs::Result<T>,
    ) -> Result<T, Diagnostic> {
        self.enter_deadline(phase, outer_end.min(Instant::now() + self.limits.entry_timeout), f)
    }

    fn enter_deadline<T>(
        &self,
        phase: &'static str,
        outer_end: Instant,
        f: impl for<'js> FnOnce(Ctx<'js>) -> rquickjs::Result<T>,
    ) -> Result<T, Diagnostic> {
        let outer_end = self.clamp_deadline(outer_end);
        if Instant::now() >= outer_end {
            return Err(Diagnostic::new(
                DiagnosticKind::BudgetExceeded,
                phase,
                "entry deadline exceeded",
            ));
        }
        let end = outer_end;
        self.interrupt.interrupted.set(false);
        self.interrupt.deadline.set(Some(end));
        let _guard = EntryGuard(&self.interrupt);
        self.update_stack_top();
        let started = Instant::now();
        let result = self.context
            .with(|ctx| f(ctx.clone()).map_err(|error| self.capture_error(&ctx, phase, error)));
        self.record_entry_duration(started);
        result
    }

    fn capture_error(
        &self,
        ctx: &Ctx<'_>,
        phase: &'static str,
        error: rquickjs::Error,
    ) -> Diagnostic {
        let interrupted = self.interrupt.interrupted.get();
        if !error.is_exception() {
            return Diagnostic::new(DiagnosticKind::Engine, phase, error.to_string());
        }
        let thrown = ctx.catch();
        // Diagnostics may invoke accessors, so normal exceptions never extend the active
        // entry/job deadline. When the entry itself was interrupted by a timeout, QuickJS
        // throws an uncatchable InternalError whose message/stack are plain data properties;
        // grant bounded `diagnostic_timeout` (still clamped to any enclosing owner deadline)
        // so the timeout reason and JS stack trace are surfaced rather than dropped.
        let formatting_end = self.clamp_deadline(Instant::now() + self.limits.diagnostic_timeout);
        let end = if interrupted {
            formatting_end
        } else {
            self.interrupt.deadline.get().map_or(formatting_end, |active| active.min(formatting_end))
        };
        self.interrupt.deadline.set(Some(end));
        let mut diagnostic = diagnostic_value(ctx, phase, thrown, self.limits.max_diagnostic_bytes, end);
        if interrupted {
            diagnostic.kind = DiagnosticKind::Interrupted;
            if diagnostic.message.is_empty()
                || diagnostic.message == "interrupted"
                || diagnostic.message.starts_with("JavaScript threw ")
            {
                diagnostic.message = "JavaScript execution interrupted (timeout)".into();
            }
        }
        diagnostic
    }

    fn pump_jobs_until(&mut self, outer_end: Instant) -> JobPump {
        let end = self.clamp_deadline(outer_end.min(Instant::now() + self.limits.job_timeout));
        let mut turn = JobPump {
            attempted_jobs: 0,
            has_pending_jobs: false,
            budget_exhausted: false,
            diagnostics: Vec::new(),
        };
        while turn.attempted_jobs < self.limits.max_jobs_per_pump && Instant::now() < end {
            match self.execute_job(end.saturating_duration_since(Instant::now())) {
                Ok(true) => turn.attempted_jobs += 1,
                Ok(false) => break,
                Err(diagnostic) => {
                    turn.attempted_jobs += 1;
                    turn.diagnostics.push(diagnostic);
                }
            }
        }
        turn.has_pending_jobs = self.is_job_pending();
        turn.budget_exhausted = turn.has_pending_jobs
            && (turn.attempted_jobs >= self.limits.max_jobs_per_pump || Instant::now() >= end);
        turn
    }
}

impl Drop for JsRuntime {
    fn drop(&mut self) {
        // The runtime owns the tracker closure, so field order alone cannot release
        // its Rc and rooted contexts early enough.
        self.runtime.set_host_promise_rejection_tracker(None);
        self.rejections.borrow_mut().records.clear();
        self.module = None;
    }
}

fn truncate(mut text: String, max: usize) -> String {
    if text.len() > max {
        text.truncate(text.floor_char_boundary(max));
    }
    text
}

fn diagnostic_value(
    ctx: &Ctx<'_>,
    phase: &'static str,
    value: Value<'_>,
    max: usize,
    deadline: Instant,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(
        DiagnosticKind::Exception,
        phase,
        format!("JavaScript threw {}", value.type_of().as_str()),
    );
    if let Some(string) = value.as_string() {
        if let Ok(message) = string.to_string() {
            diagnostic.message = truncate(message, max);
        }
    } else if let Some(number) = value.as_number() {
        diagnostic.message = number.to_string();
    } else if let Some(boolean) = value.as_bool() {
        diagnostic.message = boolean.to_string();
    } else if let Some(exception) = value.as_exception() {
        for key in ["message", "stack"] {
            if Instant::now() >= deadline { break; }
            if let Ok(Some(text)) = exception.as_object().get::<_, Option<String>>(key) {
                if key == "message" {
                    diagnostic.message = truncate(text, max);
                } else {
                    diagnostic.stack = Some(truncate(text, max));
                }
            }
            // A getter or conversion can throw: discard that secondary exception,
            // not the owned original diagnostic, before subsequent engine use.
            if ctx.has_exception() {
                drop(ctx.catch());
            }
        }
    }
    diagnostic.message = truncate(diagnostic.message, max);
    diagnostic
}

#[cfg(test)]
mod loader_tests {
    use super::*;
    use std::collections::BTreeMap;

    struct MemoryProvider {
        chunks: BTreeMap<String, String>,
    }

    impl VerifiedModuleProvider for MemoryProvider {
        fn resolve(&self, _referrer: &str, specifier: &str) -> Result<String, String> {
            let normalized = specifier.trim_start_matches("./");
            if self.chunks.contains_key(normalized) {
                Ok(normalized.to_owned())
            } else {
                Err(format!("undeclared chunk: {specifier}"))
            }
        }

        fn load(&self, canonical_name: &str) -> Result<VerifiedModulePayload, String> {
            self.chunks
                .get(canonical_name)
                .cloned()
                .map(VerifiedModulePayload::Source)
                .ok_or_else(|| format!("missing verified chunk: {canonical_name}"))
        }
    }

    #[test]
    fn verified_module_provider_loads_lazy_chunks_and_rejects_undeclared_imports() {
        let mut chunks = BTreeMap::new();
        chunks.insert(
            "chunks/lazy.mjs".into(),
            "export const value = 'loaded-from-verified-chunk';".into(),
        );
        let mut runtime = JsRuntime::new(RuntimeLimits::default()).expect("runtime");
        runtime.set_module_provider(Rc::new(MemoryProvider { chunks }));
        runtime
            .load_module(
                "app.mjs",
                r#"
                    export let result = "pending";
                    export let rejected = "none";
                    export function trigger() {
                        import("./chunks/lazy.mjs").then(m => { result = m.value; });
                        import("./chunks/escape.mjs").catch(err => { rejected = String(err); });
                    }
                "#,
            )
            .expect("load entry");
        runtime
            .with_module(|_ctx, m| {
                let trigger: Function = m.get("trigger")?;
                trigger.call::<_, ()>(())
            })
            .expect("trigger");
        let pump = runtime.pump_jobs();
        assert!(pump.diagnostics.is_empty(), "{:?}", pump.diagnostics);
        let (result, rejected) = runtime
            .with_module(|_ctx, m| {
                let r: String = m.get("result")?;
                let rej: String = m.get("rejected")?;
                Ok((r, rej))
            })
            .expect("read outputs");
        assert_eq!(result, "loaded-from-verified-chunk");
        assert!(rejected.contains("escape.mjs") || rejected.contains("undeclared"), "unexpected rejection: {rejected}");
    }

    #[test]
    fn external_cancel_flag_interrupts_running_script_before_entry_timeout() {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_trigger = Arc::clone(&cancel);
        let limits = RuntimeLimits {
            entry_timeout: Duration::from_secs(5),
            ..RuntimeLimits::default()
        };
        let mut runtime = JsRuntime::with_cancel_handle(limits, cancel).expect("runtime");
        let start = Instant::now();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(25));
            cancel_trigger.store(true, Ordering::Release);
        });
        let err = runtime.evaluate_script("for (;;) {}").expect_err("must interrupt");
        handle.join().expect("join trigger");
        assert_eq!(err.kind, DiagnosticKind::Interrupted);
        assert!(start.elapsed() < Duration::from_secs(1), "cancel took too long: {:?}", start.elapsed());
        assert!(runtime.is_cancelled());
    }

    struct BytecodeProvider {
        chunks: BTreeMap<String, Vec<u8>>,
    }

    impl VerifiedModuleProvider for BytecodeProvider {
        fn resolve(&self, _referrer: &str, specifier: &str) -> Result<String, String> {
            let normalized = specifier.strip_prefix("./").unwrap_or(specifier);
            if self.chunks.contains_key(normalized) {
                Ok(normalized.to_owned())
            } else {
                Err(format!("undeclared bytecode chunk: {specifier}"))
            }
        }

        fn load(&self, canonical_name: &str) -> Result<VerifiedModulePayload, String> {
            self.chunks
                .get(canonical_name)
                .cloned()
                .map(VerifiedModulePayload::Bytecode)
                .ok_or_else(|| format!("missing verified bytecode chunk: {canonical_name}"))
        }
    }

    #[test]
    fn bytecode_modules_compile_deterministically_load_chunks_and_preserve_stack_locations() {
        let entry_src = r#"
export let answer = "pending";
export function trigger() {
    import("./chunks/lazy.mjs").then(m => { answer = m.value; });
}
export function boom() {
    throw new Error("bytecode-boom");
}
"#;
        let chunk_src = "export const value = 'from-qjsb-chunk';\n";
        let bc_a = compile_module_bytecode("app.mjs", entry_src).expect("compile entry A");
        let bc_b = compile_module_bytecode("app.mjs", entry_src).expect("compile entry B");
        assert_eq!(bc_a, bc_b, "bytecode compilation must be deterministic");
        let chunk_bc = compile_module_bytecode("chunks/lazy.mjs", chunk_src).expect("compile chunk");

        let mut chunks = BTreeMap::new();
        chunks.insert("chunks/lazy.mjs".into(), chunk_bc);
        let mut runtime = JsRuntime::new(RuntimeLimits::default()).expect("runtime");
        runtime.set_module_provider(Rc::new(BytecodeProvider { chunks }));
        runtime.load_bytecode_module(&bc_a).expect("load entry bytecode");
        runtime
            .with_module(|_ctx, m| {
                let trigger: Function = m.get("trigger")?;
                trigger.call::<_, ()>(())
            })
            .expect("trigger");
        let pump = runtime.pump_jobs();
        assert!(pump.diagnostics.is_empty(), "{:?}", pump.diagnostics);
        let answer: String = runtime
            .with_module(|_ctx, m| m.get("answer"))
            .expect("read answer");
        assert_eq!(answer, "from-qjsb-chunk");

        let err = runtime
            .with_module(|_ctx, m| {
                let boom: Function = m.get("boom")?;
                boom.call::<_, ()>(())
            })
            .expect_err("boom throws");
        assert!(err.message.contains("bytecode-boom"));
        let stack = err.stack.as_deref().unwrap_or("");
        assert!(stack.contains("app.mjs:7:"), "stack must preserve filename:line:col: {stack}");
    }

    struct InMemoryGraphProvider {
        modules: BTreeMap<String, String>,
    }

    impl VerifiedModuleProvider for InMemoryGraphProvider {
        fn resolve(&self, referrer: &str, specifier: &str) -> Result<String, String> {
            if specifier == "./dep.mjs" && referrer == "zgui-ext://demo/dist/entry.mjs" {
                return Ok("zgui-ext://demo/dist/dep.mjs".into());
            }
            Err(format!("blocked import {specifier} from {referrer}"))
        }

        fn load(&self, canonical_name: &str) -> Result<VerifiedModulePayload, String> {
            self.modules
                .get(canonical_name)
                .cloned()
                .map(VerifiedModulePayload::Source)
                .ok_or_else(|| format!("missing module {canonical_name}"))
        }
    }

    #[test]
    fn compile_verified_module_bytecode_compiles_in_memory_without_disk_reads() {
        let mut modules = BTreeMap::new();
        modules.insert(
            "zgui-ext://demo/dist/entry.mjs".into(),
            "import { value } from './dep.mjs'; export function read() { return value + 1; }\n".into(),
        );
        modules.insert(
            "zgui-ext://demo/dist/dep.mjs".into(),
            "export const value = 41;\n".into(),
        );
        let provider: Rc<dyn VerifiedModuleProvider> = Rc::new(InMemoryGraphProvider { modules });
        let entry_bc = compile_verified_module_bytecode(
            "zgui-ext://demo/dist/entry.mjs",
            Rc::clone(&provider),
        )
        .expect("in-memory bytecode compile succeeds");
        let mut runtime = JsRuntime::new(RuntimeLimits::default()).expect("runtime");
        runtime.set_module_provider(provider);
        runtime.load_bytecode_module(&entry_bc).expect("load compiled entry");
        let answer: i32 = runtime
            .with_module(|_ctx, m| {
                let read: Function = m.get("read")?;
                read.call(())
            })
            .expect("call read");
        assert_eq!(answer, 42);
    }
}
