//! Owner-thread isolated compute worker bindings with staged reload deferral,
//! live OS-thread workers, and deterministic lockstep preview scheduling.
use super::{Mode, Work, data_property, owner, service_error};
use crate::{
    assets::{AssetSource, fixture::FixtureRuntime},
    package::Package,
};
use host_services::compute_worker::{
    DEFAULT_WORKER_HEAP_BYTES, DeterministicWorkerEnv, INBOUND_MAILBOX_CAPACITY, LiveWorkerHandle,
    LockstepWorker, MAX_WORKERS_PER_SESSION, MIN_WORKER_HEAP_BYTES, WorkerConfig, WorkerError,
    WorkerEvent, WorkerSessionMemoryBudget, serialize_plain_js_payload, validate_json_payload,
};
use rquickjs::{
    Ctx, Exception, Function, Object, Persistent, Value,
    function::{Func, Opt},
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

enum WorkersBackend {
    Live {
        budget: WorkerSessionMemoryBudget,
    },
    Preview {
        fixture: Rc<FixtureRuntime>,
        budget: WorkerSessionMemoryBudget,
    },
}

enum WorkerHandleKind {
    Staged {
        config: WorkerConfig,
        queued: VecDeque<String>,
    },
    Live(LiveWorkerHandle),
    Lockstep(Box<LockstepWorker>),
}

struct WorkerListener {
    id: u64,
    callback: Persistent<Function<'static>>,
}

struct WorkerEntry {
    handle: WorkerHandleKind,
    on_message: Vec<WorkerListener>,
    on_error: Vec<WorkerListener>,
}

pub(crate) struct WorkerDelivery {
    pub(crate) generation: u64,
    pub(crate) worker_id: u64,
    pub(crate) event: WorkerEvent,
}

pub(crate) enum WorkerStep {
    Delivery(WorkerDelivery),
    QuantumStepped,
}

pub(super) struct WorkersState {
    package: Option<Rc<Package>>,
    backend: WorkersBackend,
    staging: Cell<bool>,
    entries: RefCell<BTreeMap<u64, WorkerEntry>>,
    buffered: RefCell<VecDeque<WorkerDelivery>>,
    next_listener_id: Cell<u64>,
    poll_cursor: Cell<u64>,
}

impl WorkersState {
    pub(super) fn new(package: Option<Rc<Package>>, source: &AssetSource) -> Self {
        let backend = match source {
            AssetSource::Fixture(fixture) => WorkersBackend::Preview {
                fixture: fixture.clone(),
                budget: WorkerSessionMemoryBudget::default(),
            },
            AssetSource::Live(_) => WorkersBackend::Live {
                budget: WorkerSessionMemoryBudget::default(),
            },
        };
        Self {
            package,
            backend,
            staging: Cell::new(false),
            entries: RefCell::new(BTreeMap::new()),
            buffered: RefCell::new(VecDeque::new()),
            next_listener_id: Cell::new(0),
            poll_cursor: Cell::new(0),
        }
    }

    pub(super) fn begin_staged_mount(&self) {
        self.staging.set(true);
    }

    pub(super) fn commit_staged_mount(&self, generation: u64, wake: &Arc<dyn Fn() + Send + Sync>) {
        self.staging.set(false);
        let mut entries = self.entries.borrow_mut();
        let mut woke = false;
        let ids: Vec<u64> = entries.keys().copied().collect();
        for id in ids {
            let Some(entry) = entries.get_mut(&id) else {
                continue;
            };
            let staged = match &mut entry.handle {
                WorkerHandleKind::Staged { config, queued } => {
                    let cfg = config.clone();
                    let msgs = std::mem::take(queued);
                    Some((cfg, msgs))
                }
                _ => None,
            };
            let Some((config, queued)) = staged else {
                continue;
            };
            woke = true;
            match self.activate_handle(config, wake) {
                Ok(mut active) => {
                    for msg in queued {
                        let res = match &mut active {
                            WorkerHandleKind::Live(h) => h.post_message(msg),
                            WorkerHandleKind::Lockstep(h) => h.post_message(msg),
                            WorkerHandleKind::Staged { .. } => Ok(()),
                        };
                        if let Err(err) = res {
                            self.buffered.borrow_mut().push_back(WorkerDelivery {
                                generation,
                                worker_id: id,
                                event: WorkerEvent::Error(err),
                            });
                            break;
                        }
                    }
                    entry.handle = active;
                }
                Err(err) => {
                    self.buffered.borrow_mut().push_back(WorkerDelivery {
                        generation,
                        worker_id: id,
                        event: WorkerEvent::Error(err),
                    });
                    self.buffered.borrow_mut().push_back(WorkerDelivery {
                        generation,
                        worker_id: id,
                        event: WorkerEvent::Terminated,
                    });
                }
            }
        }
        drop(entries);
        if woke {
            wake();
        }
    }

    fn activate_handle(
        &self,
        mut config: WorkerConfig,
        wake: &Arc<dyn Fn() + Send + Sync>,
    ) -> Result<WorkerHandleKind, WorkerError> {
        match &self.backend {
            WorkersBackend::Live { budget } => {
                config.deterministic_env = None;
                let handle = LiveWorkerHandle::spawn(config, budget, Arc::clone(wake))?;
                Ok(WorkerHandleKind::Live(handle))
            }
            WorkersBackend::Preview { fixture, budget } => {
                config.deterministic_env = Some(DeterministicWorkerEnv {
                    initial_epoch_ms: fixture.worker_now_ms(),
                    rng_seed: fixture.next_worker_seed(),
                });
                let handle = LockstepWorker::new(config, budget)?;
                Ok(WorkerHandleKind::Lockstep(Box::new(handle)))
            }
        }
    }

    pub(super) fn count(&self) -> usize {
        self.entries.borrow().len()
    }

    pub(super) fn pending_count(&self) -> usize {
        self.buffered.borrow().len() + self.entries.borrow().values().map(|entry| {
            match &entry.handle {
                WorkerHandleKind::Staged { queued, .. } => 1 + queued.len(),
                WorkerHandleKind::Lockstep(worker) => {
                    usize::from(worker.has_outbound()) + usize::from(worker.has_pending_quantum())
                }
                WorkerHandleKind::Live(_) => 0,
            }
        }).sum::<usize>()
    }

    pub(super) fn has_events(&self) -> bool {
        if !self.buffered.borrow().is_empty() {
            return true;
        }
        if self.staging.get() {
            return false;
        }
        let mut entries = self.entries.borrow_mut();
        for (&id, entry) in entries.iter_mut() {
            match &mut entry.handle {
                WorkerHandleKind::Lockstep(worker) => {
                    if worker.has_outbound() || worker.has_pending_quantum() {
                        return true;
                    }
                }
                WorkerHandleKind::Live(worker) => {
                    if let Some(event) = worker.try_recv() {
                        // Buffer the peeked event so nothing is dropped.
                        self.buffered.borrow_mut().push_back(WorkerDelivery {
                            generation: 0,
                            worker_id: id,
                            event,
                        });
                        return true;
                    }
                }
                WorkerHandleKind::Staged { .. } => {}
            }
        }
        false
    }

    pub(super) fn next_step(&self, generation: u64) -> Option<WorkerStep> {
        if let Some(mut delivery) = self.buffered.borrow_mut().pop_front() {
            if delivery.generation == 0 {
                delivery.generation = generation;
            }
            return Some(WorkerStep::Delivery(delivery));
        }
        if self.staging.get() {
            return None;
        }
        let mut entries = self.entries.borrow_mut();
        if entries.is_empty() {
            return None;
        }
        let after = self.poll_cursor.get();
        let keys: Vec<u64> = entries
            .range((std::ops::Bound::Excluded(after), std::ops::Bound::Unbounded))
            .map(|(&k, _)| k)
            .chain(entries.range(..=after).map(|(&k, _)| k))
            .collect();

        // First drain any ready outbound messages across workers in fair round-robin order.
        for &id in &keys {
            let Some(entry) = entries.get_mut(&id) else {
                continue;
            };
            let event = match &mut entry.handle {
                WorkerHandleKind::Live(worker) => worker.try_recv(),
                WorkerHandleKind::Lockstep(worker) => worker.try_recv(),
                WorkerHandleKind::Staged { .. } => None,
            };
            if let Some(event) = event {
                self.poll_cursor.set(id);
                return Some(WorkerStep::Delivery(WorkerDelivery {
                    generation,
                    worker_id: id,
                    event,
                }));
            }
        }

        // In deterministic preview mode, step at most one lockstep quantum on the next ready worker.
        if let WorkersBackend::Preview { fixture, .. } = &self.backend {
            let now_ms = fixture.worker_now_ms();
            for &id in &keys {
                let Some(entry) = entries.get_mut(&id) else {
                    continue;
                };
                if let WorkerHandleKind::Lockstep(worker) = &mut entry.handle
                    && worker.has_pending_quantum()
                {
                    worker.step_quantum(now_ms);
                    self.poll_cursor.set(id);
                    if let Some(event) = worker.try_recv() {
                        return Some(WorkerStep::Delivery(WorkerDelivery {
                            generation,
                            worker_id: id,
                            event,
                        }));
                    }
                    return Some(WorkerStep::QuantumStepped);
                }
            }
        }
        None
    }

    pub(super) fn stop_all(&self) {
        self.buffered.borrow_mut().clear();
        for entry in self.entries.borrow_mut().values_mut() {
            match &mut entry.handle {
                WorkerHandleKind::Live(worker) => worker.terminate(),
                WorkerHandleKind::Lockstep(worker) => worker.terminate(),
                WorkerHandleKind::Staged { queued, .. } => queued.clear(),
            }
        }
    }

    pub(super) fn clear(&self) {
        self.stop_all();
        self.entries.borrow_mut().clear();
        self.buffered.borrow_mut().clear();
    }

    fn next_listener_id(&self) -> u64 {
        let next = self.next_listener_id.get().wrapping_add(1);
        self.next_listener_id.set(next);
        next
    }
}

impl Work {
    pub(super) fn install_workers<'js>(self: &Rc<Self>, services: &Object<'js>) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        services.prop(
            "spawnWorker",
            data_property(Func::from(
                move |ctx: Ctx<'js>, name: String, options: Opt<Value<'js>>| {
                    owner(&weak, &ctx)?.spawn_worker(&ctx, name, options.0)
                },
            )),
        )?;
        Ok(())
    }

    fn spawn_worker<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        name: String,
        options: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        if self.workers.entries.borrow().len() >= MAX_WORKERS_PER_SESSION {
            let err = service_error(
                ctx,
                "E_RESOURCE_LIMIT",
                &format!("maximum of {MAX_WORKERS_PER_SESSION} concurrent compute workers reached"),
            )?;
            return Err(ctx.throw(err.into_value()));
        }
        let package = self.workers.package.as_ref().ok_or_else(|| {
            match service_error(ctx, "E_CAPABILITY", "compute workers require a sealed application package") {
                Ok(err) => ctx.throw(err.into_value()),
                Err(e) => e,
            }
        })?;
        let (module_id, payload) = package.load_worker_payload(&name).map_err(|msg| {
            let code = if msg.contains("integrity mismatch") { "E_BUNDLE" } else { "E_CAPABILITY" };
            match service_error(ctx, code, &msg) {
                Ok(err) => ctx.throw(err.into_value()),
                Err(e) => e,
            }
        })?;

        let mut heap_limit_bytes = DEFAULT_WORKER_HEAP_BYTES;
        let mut initial_on_message = Vec::new();
        let mut initial_on_error = Vec::new();

        if let Some(opt_val) = options
            && !opt_val.is_undefined()
            && !opt_val.is_null()
        {
            let obj = opt_val
                .into_object()
                .ok_or_else(|| Exception::throw_type(ctx, "spawnWorker options must be an object"))?;
            if let Ok(val) = obj.get::<_, Value<'js>>("maxHeapBytes")
                && !val.is_undefined()
            {
                let bytes = val
                    .as_number()
                    .filter(|n| n.is_finite() && n.fract() == 0.0 && *n >= MIN_WORKER_HEAP_BYTES as f64 && *n <= DEFAULT_WORKER_HEAP_BYTES as f64)
                    .ok_or_else(|| {
                        Exception::throw_range(
                            ctx,
                            &format!(
                                "worker maxHeapBytes must be an integer between {MIN_WORKER_HEAP_BYTES} and {DEFAULT_WORKER_HEAP_BYTES}"
                            ),
                        )
                    })?;
                heap_limit_bytes = bytes as usize;
            }
            if let Ok(val) = obj.get::<_, Value<'js>>("onMessage")
                && !val.is_undefined()
            {
                let cb = val
                    .into_function()
                    .ok_or_else(|| Exception::throw_type(ctx, "worker onMessage option must be a function"))?;
                initial_on_message.push(WorkerListener {
                    id: self.workers.next_listener_id(),
                    callback: Persistent::save(ctx, cb),
                });
            }
            if let Ok(val) = obj.get::<_, Value<'js>>("onError")
                && !val.is_undefined()
            {
                let cb = val
                    .into_function()
                    .ok_or_else(|| Exception::throw_type(ctx, "worker onError option must be a function"))?;
                initial_on_error.push(WorkerListener {
                    id: self.workers.next_listener_id(),
                    callback: Persistent::save(ctx, cb),
                });
            }
        }

        let mut config = match payload {
            js_runtime::VerifiedModulePayload::Source(source) => {
                WorkerConfig::new(name, module_id, source)
            }
            js_runtime::VerifiedModulePayload::Bytecode(bytes) => {
                WorkerConfig::new(name, module_id, String::new()).with_bytecode(bytes)
            }
        };
        config.heap_limit_bytes = heap_limit_bytes;

        let id = self.id(ctx)?;
        let handle = if self.workers.staging.get() {
            WorkerHandleKind::Staged {
                config,
                queued: VecDeque::new(),
            }
        } else {
            self.workers.activate_handle(config, &self.wake).map_err(|err| {
                match service_error(ctx, err.code, &err.message) {
                    Ok(js_err) => ctx.throw(js_err.into_value()),
                    Err(e) => e,
                }
            })?
        };

        self.workers.entries.borrow_mut().insert(
            id,
            WorkerEntry {
                handle,
                on_message: initial_on_message,
                on_error: initial_on_error,
            },
        );
        if !self.workers.staging.get() {
            (self.wake)();
        }

        let js_handle = Object::new(ctx.clone())?;
        let weak = Rc::downgrade(self);
        js_handle.prop(
            "postMessage",
            data_property(Func::from(move |ctx: Ctx<'js>, payload: Value<'js>| {
                let owner = owner(&weak, &ctx)?;
                owner.worker_post_message(&ctx, id, payload)
            })),
        )?;
        let weak = Rc::downgrade(self);
        js_handle.prop(
            "onMessage",
            data_property(Func::from(move |ctx: Ctx<'js>, cb: Function<'js>| {
                let owner = owner(&weak, &ctx)?;
                owner.worker_add_listener(&ctx, id, cb, false)
            })),
        )?;
        let weak = Rc::downgrade(self);
        js_handle.prop(
            "onError",
            data_property(Func::from(move |ctx: Ctx<'js>, cb: Function<'js>| {
                let owner = owner(&weak, &ctx)?;
                owner.worker_add_listener(&ctx, id, cb, true)
            })),
        )?;
        let weak = Rc::downgrade(self);
        js_handle.prop(
            "terminate",
            data_property(Func::from(move || {
                if let Some(owner) = weak.upgrade() {
                    owner.worker_terminate(id);
                }
            })),
        )?;
        let weak = Rc::downgrade(self);
        js_handle.prop(
            "isTerminated",
            data_property(Func::from(move || -> bool {
                weak.upgrade()
                    .is_none_or(|owner| !owner.workers.entries.borrow().contains_key(&id))
            })),
        )?;
        Ok(js_handle)
    }

    fn worker_post_message<'js>(&self, ctx: &Ctx<'js>, id: u64, payload: Value<'js>) -> rquickjs::Result<()> {
        let raw = serialize_plain_js_payload(ctx, payload)?;
        if let Err(err) = validate_json_payload(&raw) {
            let js_err = service_error(ctx, err.code, &err.message)?;
            return Err(ctx.throw(js_err.into_value()));
        }
        let mut entries = self.workers.entries.borrow_mut();
        let Some(entry) = entries.get_mut(&id) else {
            let js_err = service_error(ctx, "E_WORKER_CLOSED", "worker is terminated")?;
            return Err(ctx.throw(js_err.into_value()));
        };
        let result = match &mut entry.handle {
            WorkerHandleKind::Staged { queued, .. } => {
                if queued.len() >= INBOUND_MAILBOX_CAPACITY {
                    Err(WorkerError::new("E_BACKPRESSURE", "worker inbound mailbox is full"))
                } else {
                    queued.push_back(raw);
                    Ok(())
                }
            }
            WorkerHandleKind::Live(worker) => worker.post_message(raw),
            WorkerHandleKind::Lockstep(worker) => worker.post_message(raw),
        };
        drop(entries);
        if let Err(err) = result {
            let js_err = service_error(ctx, err.code, &err.message)?;
            return Err(ctx.throw(js_err.into_value()));
        }
        if !self.workers.staging.get() {
            (self.wake)();
        }
        Ok(())
    }

    fn worker_add_listener<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        worker_id: u64,
        callback: Function<'js>,
        is_error: bool,
    ) -> rquickjs::Result<Function<'js>> {
        let listener_id = self.workers.next_listener_id();
        {
            let mut entries = self.workers.entries.borrow_mut();
            if let Some(entry) = entries.get_mut(&worker_id) {
                let target = if is_error { &mut entry.on_error } else { &mut entry.on_message };
                target.push(WorkerListener {
                    id: listener_id,
                    callback: Persistent::save(ctx, callback),
                });
            }
        }
        let weak = Rc::downgrade(self);
        Function::new(ctx.clone(), move || {
            if let Some(owner) = weak.upgrade()
                && let Some(entry) = owner.workers.entries.borrow_mut().get_mut(&worker_id)
            {
                let target = if is_error { &mut entry.on_error } else { &mut entry.on_message };
                target.retain(|item| item.id != listener_id);
            }
        })
    }

    fn worker_terminate(&self, id: u64) {
        let removed = self.workers.entries.borrow_mut().remove(&id);
        if let Some(mut entry) = removed {
            match &mut entry.handle {
                WorkerHandleKind::Live(worker) => worker.terminate(),
                WorkerHandleKind::Lockstep(worker) => worker.terminate(),
                WorkerHandleKind::Staged { queued, .. } => queued.clear(),
            }
        }
        self.workers
            .buffered
            .borrow_mut()
            .retain(|delivery| delivery.worker_id != id);
    }

    pub(crate) fn begin_staged_workers(&self) {
        self.workers.begin_staged_mount();
    }

    pub(crate) fn commit_staged_workers(&self) {
        self.workers.commit_staged_mount(self.generation, &self.wake);
    }

    pub(crate) fn next_worker_step(&self) -> Option<WorkerStep> {
        if self.mode.get() != Mode::Mounted {
            return None;
        }
        self.workers.next_step(self.generation)
    }

    pub(crate) fn settle_worker_event<'js>(
        &self,
        ctx: &Ctx<'js>,
        delivery: WorkerDelivery,
    ) -> rquickjs::Result<()> {
        if delivery.generation != self.generation || self.mode.get() != Mode::Mounted {
            return Ok(());
        }
        match delivery.event {
            WorkerEvent::Message(raw) => {
                let listeners: Vec<Persistent<Function<'static>>> = {
                    let entries = self.workers.entries.borrow();
                    let Some(entry) = entries.get(&delivery.worker_id) else {
                        return Ok(());
                    };
                    entry.on_message.iter().map(|l| l.callback.clone()).collect()
                };
                for cb in listeners {
                    // Re-check live membership in case an earlier listener terminated this worker.
                    if !self.workers.entries.borrow().contains_key(&delivery.worker_id) {
                        break;
                    }
                    let parsed: Value<'js> = ctx.json_parse(raw.as_bytes())?;
                    cb.restore(ctx)?.call::<_, ()>((parsed,))?;
                }
            }
            WorkerEvent::Error(err) => {
                let listeners: Vec<Persistent<Function<'static>>> = {
                    let entries = self.workers.entries.borrow();
                    let Some(entry) = entries.get(&delivery.worker_id) else {
                        return Ok(());
                    };
                    entry.on_error.iter().map(|l| l.callback.clone()).collect()
                };
                let message = if let Some(stack) = err.stack
                    && let Some(pkg) = &self.workers.package
                {
                    let remapped = pkg.remap_diagnostic(js_runtime::Diagnostic {
                        kind: js_runtime::DiagnosticKind::Exception,
                        phase: "worker",
                        message: err.message.clone(),
                        stack: Some(stack),
                        generated_stack: None,
                    });
                    match remapped.stack {
                        Some(s) => format!("{}\n{s}", remapped.message),
                        None => remapped.message,
                    }
                } else {
                    err.message
                };
                for cb in listeners {
                    if !self.workers.entries.borrow().contains_key(&delivery.worker_id) {
                        break;
                    }
                    let obj = Object::new(ctx.clone())?;
                    obj.prop("code", data_property(err.code))?;
                    obj.prop("message", data_property(message.as_str()))?;
                    cb.restore(ctx)?.call::<_, ()>((obj,))?;
                }
            }
            WorkerEvent::Terminated => {
                self.worker_terminate(delivery.worker_id);
            }
        }
        Ok(())
    }
}
