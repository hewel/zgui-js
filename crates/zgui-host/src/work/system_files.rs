//! Session-owned system directory authority, worker completions, and conflict replies.

mod worker;

use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        mpsc::{Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use host_services::{
    ServiceError,
    files::{
        FileMode,
        grants::{
            GrantKind, authorize_system_grant, persist_grant, resolve_grant_store_path,
            restore_system_grant,
        },
        portal::{PortalDialogKind, run_file_chooser},
    },
    system_files::{
        self as fs, ConflictResolution, EntryRef, Job, JobResult, PreviewSystemFiles, ReadOptions,
        Root, Scope, SortBy, SortDirection, ThumbnailSize, Trash, Volume,
    },
};
use rquickjs::{
    Array, Class, Ctx, Exception, Function, IntoJs, JsLifetime, Object, Persistent, Value,
    class::Trace,
    function::{Func, Opt, This},
};
use serde::Serialize;

use super::files::{parse_dialog_options, read_bounded_utf8, throw_file_error};
use super::{
    ImageLease, MAX_DIRECT_BYTES_PER_SESSION, MAX_DIRECT_IMAGE_BYTES, MAX_DIRECT_IMAGES, Mode,
    Work, data_property, owner, service_error,
};
use crate::{
    assets::{AssetSource, fixture::FixtureRuntime},
    package::SystemFilesCapabilities,
};
pub(crate) use worker::Event;
use worker::{Control, IoPool, PreviewWorker};

const MAX_OPERATIONS: usize = 128;
const MAX_HANDLES: usize = 256;
const MAX_JOBS: usize = 8;
const MAX_WATCHERS: usize = 16;
const MAX_ENTRIES_PER_JOB: u32 = 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);
const MAX_EXTERNAL_TOKENS: usize = 32;

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct SystemDirBrand {
    #[qjs(skip_trace)]
    id: u64,
    #[qjs(skip_trace)]
    generation: u64,
}

pub(crate) enum Completed {
    Void,
    Json(Vec<u8>),
    Directory(Option<Root>),
    Image(Option<Arc<[u8]>>),
    External {
        uri: Result<String, ServiceError>,
        token_present: bool,
    },
}

type Outcome = Result<Completed, ServiceError>;

fn json(value: &impl Serialize) -> Outcome {
    serde_json::to_vec(value)
        .map(Completed::Json)
        .map_err(|error| fault("Io", error.to_string()))
}

fn fault(code: &'static str, message: impl Into<String>) -> ServiceError {
    ServiceError {
        code,
        message: message.into(),
    }
}
fn cancelled() -> ServiceError {
    fault("Cancelled", "system files request cancelled")
}
fn throw(ctx: &Ctx<'_>, error: ServiceError) -> rquickjs::Error {
    throw_file_error(ctx, error.code, &error.message)
}

struct Pending {
    resolve: Persistent<Function<'static>>,
    reject: Persistent<Function<'static>>,
    on_progress: Option<Persistent<Function<'static>>>,
    on_conflict: Option<Persistent<Function<'static>>>,
    control: Arc<Control>,
    job: bool,
}

struct Watch {
    callback: Persistent<Function<'static>>,
    control: Arc<Control>,
}

struct Background {
    worker: JoinHandle<()>,
    job: bool,
}

#[derive(Default)]
pub(super) struct Counts {
    pub handles: usize,
    pub jobs: usize,
    pub conflicts: usize,
    pub watchers: usize,
}

pub(super) struct SystemFilesState {
    capabilities: SystemFilesCapabilities,
    app_id: String,
    fixture: Option<Rc<FixtureRuntime>>,
    roots: RefCell<BTreeMap<u64, Root>>,
    pending: RefCell<BTreeMap<u64, Pending>>,
    external_tokens: RefCell<VecDeque<String>>,
    watches: RefCell<BTreeMap<u64, Watch>>,
    workers: RefCell<Vec<Background>>,
    next: Cell<u64>,
    tx: SyncSender<Event>,
    rx: Receiver<Event>,
    ready: RefCell<VecDeque<Event>>,
    pool: OnceCell<IoPool>,
    preview_worker: OnceCell<PreviewWorker>,
}

impl SystemFilesState {
    pub(super) fn new(
        capabilities: SystemFilesCapabilities,
        app_id: String,
        source: &AssetSource,
    ) -> Self {
        let (tx, rx) = worker::mailbox();
        Self {
            capabilities,
            app_id,
            fixture: match source {
                AssetSource::Live(_) => None,
                AssetSource::Fixture(fixture) => Some(fixture.clone()),
            },
            roots: RefCell::default(),
            pending: RefCell::default(),
            watches: RefCell::default(),
            workers: RefCell::default(),
            next: Cell::new(1),
            external_tokens: RefCell::default(),
            tx,
            rx,
            ready: RefCell::default(),
            pool: OnceCell::new(),
            preview_worker: OnceCell::new(),
        }
    }

    fn id(&self) -> u64 {
        let id = self.next.get();
        self.next.set(
            id.checked_add(1)
                .expect("system files identifier space exhausted"),
        );
        id
    }

    fn preview(&self) -> Option<&Arc<PreviewSystemFiles>> {
        self.fixture.as_ref().map(|fixture| fixture.system_files())
    }

    pub(super) fn dropped_root_slots(&self) -> usize {
        MAX_HANDLES - self.roots.borrow().len()
    }

    fn reopen(&self, path: PathBuf, mode: FileMode) -> Result<Root, ServiceError> {
        match self.preview() {
            Some(fs) => fs.reopen(&path, mode),
            None => Root::reopen(path, mode),
        }
    }

    pub(super) fn locate_drop(&self, path: &Path) -> Option<(PathBuf, fs::EntryKind)> {
        match self.preview() {
            Some(fs) if self.capabilities.read => fs.locate_dropped_entry(path).ok(),
            Some(_) => None,
            None => fs::locate_dropped_entry(path).ok(),
        }
    }

    pub(super) fn dropped_root(
        &self,
        path: PathBuf,
        kind: fs::EntryKind,
    ) -> Option<(Root, String)> {
        if !self.capabilities.read {
            return None;
        }
        if kind == fs::EntryKind::Other {
            return None;
        }
        if let Some(root) = self
            .roots
            .borrow()
            .values()
            .filter(|root| !root.is_entry_only() && path.starts_with(root.path()))
            .max_by_key(|root| (root.mode().can_write(), root.path().as_os_str().len()))
        {
            return Some((root.clone(), root.relative_path(&path)?));
        }
        if kind == fs::EntryKind::Dir {
            let root = self
                .reopen(path, FileMode::Read)
                .ok()?
                .with_user_authorization();
            return Some((root, String::new()));
        }
        let parent = self
            .reopen(path.parent()?.to_owned(), FileMode::Read)
            .ok()?;
        let relative = parent.relative_path(&path)?;
        Some((parent.confine_to_entry(path.file_name()?), relative))
    }

    fn active_roots(&self) -> Vec<Root> {
        self.roots
            .borrow()
            .values()
            .filter(|root| !root.is_entry_only())
            .cloned()
            .collect()
    }

    fn scope(
        &self,
        ctx: &Ctx<'_>,
        id: u64,
        mode: FileMode,
        active: Arc<Vec<Root>>,
    ) -> rquickjs::Result<Scope> {
        let roots = self.roots.borrow();
        let root = roots
            .get(&id)
            .ok_or_else(|| throw_file_error(ctx, "InvalidState", "system directory is closed"))?;
        self.require_mode(ctx, mode)?;
        if mode.can_write() && !root.mode().can_write() {
            return Err(throw_file_error(
                ctx,
                "E_CAPABILITY",
                "system directory grant is read-only",
            ));
        }
        Ok(Scope::with_roots(root.clone(), active))
    }

    fn require_mode(&self, ctx: &Ctx<'_>, mode: FileMode) -> rquickjs::Result<()> {
        require(ctx, self.capabilities.read, "read")?;
        if mode.can_write() {
            require(ctx, self.capabilities.write, "write")?;
        }
        Ok(())
    }

    fn reap(&self) {
        self.workers
            .borrow_mut()
            .retain(|worker| !worker.worker.is_finished());
    }

    pub(super) fn pending_count(&self) -> usize {
        self.pending.borrow().len()
    }

    pub(super) fn has_inflight(&self) -> bool {
        let pending = self.pending.borrow();
        let paused = self.preview_worker.get().is_some()
            && pending.values().any(|request| request.control.waiting());
        pending
            .values()
            .any(|request| !(paused && request.job) && !request.control.waiting())
    }

    pub(super) fn has_events(&self) -> bool {
        self.reap();
        if !self.ready.borrow().is_empty() {
            return true;
        }
        if let Ok(event) = self.rx.try_recv() {
            self.ready.borrow_mut().push_back(event);
            return true;
        }
        false
    }

    fn next_event(&self) -> Option<Event> {
        self.ready
            .borrow_mut()
            .pop_front()
            .or_else(|| self.rx.try_recv().ok())
    }

    pub(super) fn stop(&self) {
        for pending in self.pending.borrow_mut().values() {
            pending.control.cancel();
        }
        self.pending.borrow_mut().clear();
        if let Some(worker) = self.preview_worker.get() {
            worker.stop();
        }
        for watch in self.watches.borrow().values() {
            watch.control.cancel();
        }
    }

    fn job_count(&self) -> usize {
        self.workers
            .borrow()
            .iter()
            .filter(|worker| worker.job)
            .count()
            + self.preview_worker.get().map_or(0, PreviewWorker::jobs)
    }

    pub(super) fn counts(&self) -> Counts {
        self.reap();
        let workers = self.workers.borrow();
        Counts {
            handles: self.roots.borrow().len(),
            jobs: self.job_count(),
            conflicts: self
                .pending
                .borrow()
                .values()
                .filter(|request| request.control.waiting())
                .count(),
            watchers: (workers.iter().filter(|worker| !worker.job).count())
                .max(self.watches.borrow().len()),
        }
    }

    pub(super) fn wait_for_shutdown(&self) {
        // Waiting workers are explicitly woken before disposal. Bound teardown if
        // a live filesystem blocks a syscall; counts still report that worker.
        let end = Instant::now() + Duration::from_millis(75);
        loop {
            while self.rx.try_recv().is_ok() {}
            self.reap();
            if (self.workers.borrow().is_empty()
                && self
                    .preview_worker
                    .get()
                    .is_none_or(PreviewWorker::is_finished))
                || Instant::now() >= end
            {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn clear(&self) {
        self.external_tokens.borrow_mut().clear();
        self.roots.borrow_mut().clear();
        self.watches.borrow_mut().clear();
        self.ready.borrow_mut().clear();
        while self.rx.try_recv().is_ok() {}
    }

    fn preview_worker(&self) -> std::io::Result<&PreviewWorker> {
        if self.preview_worker.get().is_none() {
            self.preview_worker
                .set(PreviewWorker::new()?)
                .unwrap_or_else(|_| unreachable!("owner-thread fixture worker admission"));
        }
        Ok(self
            .preview_worker
            .get()
            .expect("fixture worker initialized"))
    }

    fn spawn(&self, task: worker::Task, tracked: Option<bool>) -> std::io::Result<()> {
        if let Some(job) = tracked {
            let kind = if job { "job" } else { "volumes" };
            let worker = thread::Builder::new()
                .name(format!("zgui-system-{kind}"))
                .spawn(task)?;
            self.workers.borrow_mut().push(Background { worker, job });
            return Ok(());
        }
        if self.pool.get().is_none() {
            self.pool
                .set(IoPool::new()?)
                .unwrap_or_else(|_| unreachable!("owner-thread pool admission"));
        }
        self.pool
            .get()
            .expect("metadata workers initialized")
            .submit(task)
    }

    fn opened(&self, path: &Path, mode: FileMode) -> Result<Root, ServiceError> {
        match self.preview() {
            Some(fs) => fs.open(path, mode),
            None => Root::open(path, mode),
        }
    }
}

fn require(ctx: &Ctx<'_>, allowed: bool, capability: &str) -> rquickjs::Result<()> {
    if allowed {
        Ok(())
    } else {
        Err(throw_file_error(
            ctx,
            "E_CAPABILITY",
            &format!("requires capabilities.systemFiles.{capability}"),
        ))
    }
}

impl Work {
    pub(super) fn install_system_files<'js>(
        self: &Rc<Self>,
        services: &Object<'js>,
    ) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        services.prop(
            "openSystemDirectory",
            data_property(Func::from(move |ctx: Ctx<'js>, path: Value<'js>| {
                owner(&weak, &ctx)?.open_system_directory(&ctx, path)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "openSystemDirectoryDialog",
            data_property(Func::from(move |ctx: Ctx<'js>, opt: Opt<Value<'js>>| {
                owner(&weak, &ctx)?.system_directory_dialog(&ctx, opt.0)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "openDroppedSystemDirectory",
            data_property(Func::from(move |ctx: Ctx<'js>, handle: Value<'js>| {
                let work = owner(&weak, &ctx)?;
                work.require_creation(&ctx)?;
                work.system_files.require_mode(&ctx, FileMode::Read)?;
                let path = work.dropped_directory_path(&ctx, handle)?;
                let dir = work
                    .system_files
                    .reopen(path.into(), FileMode::Read)
                    .map_err(|error| throw(&ctx, error))?
                    .with_user_authorization();
                work.system_handle(&ctx, dir)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "persistSystemGrant",
            data_property(Func::from(move |ctx: Ctx<'js>, handle: Value<'js>| {
                owner(&weak, &ctx)?.persist_system_grant(&ctx, handle)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "restoreSystemGrant",
            data_property(Func::from(move |ctx: Ctx<'js>, token: Value<'js>| {
                owner(&weak, &ctx)?.restore_system_grant(&ctx, token)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "startFileJob",
            data_property(Func::from(
                move |ctx: Ctx<'js>, spec: Object<'js>, opts: Opt<Option<Object<'js>>>| {
                    owner(&weak, &ctx)?.start_file_job(&ctx, spec, opts.0.flatten())
                },
            )),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "listVolumes",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                let work = owner(&weak, &ctx)?;
                require(&ctx, work.system_files.capabilities.volumes, "volumes")?;
                work.system_read(&ctx, |preview| json(&volumes(preview)?))
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "watchVolumes",
            data_property(Func::from(move |ctx: Ctx<'js>, callback: Function<'js>| {
                owner(&weak, &ctx)?.watch_system_volumes(&ctx, callback)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "loadCachedThumbnail",
            data_property(Func::from(move |ctx: Ctx<'js>, options: Object<'js>| {
                owner(&weak, &ctx)?.cached_system_thumbnail(&ctx, options)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "openExternal",
            data_property(Func::from(move |ctx: Ctx<'js>, options: Object<'js>| {
                owner(&weak, &ctx)?.open_external(&ctx, options)
            })),
        )
    }

    pub(crate) fn next_system_file_event(&self) -> Option<Event> {
        (self.mode.get() == Mode::Mounted)
            .then(|| self.system_files.next_event())
            .flatten()
    }

    pub(crate) fn settle_system_file_event<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        event: Event,
    ) -> rquickjs::Result<()> {
        match event {
            Event::Done(id, result) => {
                let Some(pending) = self.system_files.pending.borrow_mut().remove(&id) else {
                    return Ok(());
                };
                match result.and_then(|value| {
                    if pending.control.cancelled() && !pending.job {
                        Err(cancelled())
                    } else {
                        Ok(value)
                    }
                }) {
                    Ok(value) => {
                        let value = self.system_value(ctx, value);
                        match value {
                            Ok(value) => pending.resolve.restore(ctx)?.call::<_, ()>((value,)),
                            Err(error) if error.is_exception() => {
                                pending.reject.restore(ctx)?.call::<_, ()>((ctx.catch(),))
                            }
                            Err(error) => Err(error),
                        }
                    }
                    Err(error) => pending.reject.restore(ctx)?.call::<_, ()>((service_error(
                        ctx,
                        error.code,
                        &error.message,
                    )?,)),
                }
            }
            Event::Progress(id, progress) => {
                let callback = self
                    .system_files
                    .pending
                    .borrow()
                    .get(&id)
                    .filter(|request| !request.control.cancelled())
                    .and_then(|request| request.on_progress.clone());
                if let Some(callback) = callback {
                    callback
                        .restore(ctx)?
                        .call::<_, ()>((serialize(ctx, &progress)?,))?;
                }
                Ok(())
            }
            Event::Conflict(id, conflict) => {
                let Some((control, callback)) = self
                    .system_files
                    .pending
                    .borrow()
                    .get(&id)
                    .and_then(|request| {
                        (!request.control.cancelled())
                            .then(|| (request.control.clone(), request.on_conflict.clone()))
                    })
                else {
                    return Ok(());
                };
                let result = (|| {
                    let Some(callback) = callback else {
                        control.resolve(ConflictResolution::CANCEL);
                        return Ok(());
                    };
                    let value: Value =
                        callback.restore(ctx)?.call((serialize(ctx, &conflict)?,))?;
                    if let Some(promise) = value.as_promise() {
                        let success_control = Arc::downgrade(&control);
                        let settle = Function::new(ctx.clone(), move |value: Value<'js>| {
                            if let Some(control) = success_control.upgrade() {
                                control.resolve(decision(&value));
                            }
                        })?;
                        let error_control = Arc::downgrade(&control);
                        let reject = Function::new(ctx.clone(), move || {
                            if let Some(control) = error_control.upgrade() {
                                control.resolve(ConflictResolution::CANCEL);
                            }
                        })?;
                        let then = promise.then()?;
                        then.call::<_, ()>((This(promise.clone()), settle, reject))?;
                    } else {
                        control.resolve(decision(&value));
                    }
                    Ok::<(), rquickjs::Error>(())
                })();
                if let Err(error) = result {
                    control.resolve(ConflictResolution::CANCEL);
                    if error.is_exception() {
                        let _ = ctx.catch();
                    } else {
                        return Err(error);
                    }
                }
                Ok(())
            }
            Event::Volumes(id, volumes) => {
                let callback = self
                    .system_files
                    .watches
                    .borrow()
                    .get(&id)
                    .map(|watch| watch.callback.clone());
                if let Some(callback) = callback {
                    callback
                        .restore(ctx)?
                        .call::<_, ()>((serialize(ctx, &volumes)?,))?;
                }
                Ok(())
            }
        }
    }

    fn system_value<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        value: Completed,
    ) -> rquickjs::Result<Value<'js>> {
        match value {
            Completed::Void => ().into_js(ctx),
            Completed::Json(bytes) => ctx.json_parse(bytes),
            Completed::Directory(Some(root)) => {
                self.system_handle(ctx, root).map(Object::into_value)
            }
            Completed::Directory(None) | Completed::Image(None) => Ok(Value::new_null(ctx.clone())),
            Completed::Image(Some(bytes)) => {
                let len = u32::try_from(bytes.len())
                    .ok()
                    .filter(|len| *len <= MAX_DIRECT_IMAGE_BYTES)
                    .ok_or_else(|| {
                        Exception::throw_range(ctx, "thumbnail exceeds the direct image limit")
                    })?;
                let budget = self.direct_images.get();
                if budget.handles >= MAX_DIRECT_IMAGES
                    || len > MAX_DIRECT_BYTES_PER_SESSION - budget.bytes
                {
                    return Err(Exception::throw_range(
                        ctx,
                        "session direct image budget exceeded",
                    ));
                }
                self.image_handle(ctx, bytes, ImageLease::Direct(len))
                    .map(Object::into_value)
            }
            Completed::External { uri, token_present } => {
                if let Some(fixture) = &self.system_files.fixture {
                    fixture
                        .record_open_external(self.generation, &uri, token_present)
                        .map_err(|error| throw(ctx, error))?;
                }
                uri.map_err(|error| throw(ctx, error))?;
                ().into_js(ctx)
            }
        }
    }

    fn system_request<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        callbacks: Option<Object<'js>>,
        job: bool,
    ) -> rquickjs::Result<(u64, Arc<Control>, Object<'js>)> {
        self.require_creation(ctx)?;
        let state = &self.system_files;
        if state.pending_count() >= MAX_OPERATIONS {
            return Err(throw_file_error(
                ctx,
                "E_LIMIT",
                "too many concurrent system files requests",
            ));
        }
        let (on_progress, on_conflict) = if let Some(opts) = callbacks {
            (
                opts.get::<_, Option<Function>>("onProgress")?,
                opts.get::<_, Option<Function>>("onConflict")?,
            )
        } else {
            (None, None)
        };
        let id = state.id();
        let control = Arc::new(Control::default());
        let (promise, resolve, reject) = ctx.promise()?;
        let request = Object::new(ctx.clone())?;
        request.prop("promise", data_property(promise))?;
        let weak = Rc::downgrade(self);
        request.prop(
            "cancel",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                if let Some(work) = weak.upgrade() {
                    let requests = &work.system_files.pending;
                    if job {
                        if let Some(pending) = requests.borrow().get(&id) {
                            pending.control.cancel();
                        }
                    } else if let Some(pending) = requests.borrow_mut().remove(&id) {
                        pending.control.cancel();
                        pending.reject.restore(&ctx)?.call::<_, ()>((service_error(
                            &ctx,
                            "Cancelled",
                            "system files request cancelled",
                        )?,))?;
                    }
                }
                Ok::<(), rquickjs::Error>(())
            })),
        )?;
        state.pending.borrow_mut().insert(
            id,
            Pending {
                resolve: Persistent::save(ctx, resolve),
                reject: Persistent::save(ctx, reject),
                on_progress: on_progress.map(|cb| Persistent::save(ctx, cb)),
                on_conflict: on_conflict.map(|cb| Persistent::save(ctx, cb)),
                control: control.clone(),
                job,
            },
        );
        Ok((id, control, request))
    }

    fn system_read<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        run: impl FnOnce(Option<&PreviewSystemFiles>) -> Outcome + Send + 'static,
    ) -> rquickjs::Result<Object<'js>> {
        let (id, control, request) = self.system_request(ctx, None, false)?;
        self.dispatch_system_read(id, control, run);
        Ok(request)
    }

    fn dispatch_system_read(
        &self,
        id: u64,
        control: Arc<Control>,
        run: impl FnOnce(Option<&PreviewSystemFiles>) -> Outcome + Send + 'static,
    ) {
        let state = &self.system_files;
        let preview = state.preview().cloned();
        let deterministic = preview.is_some();
        let tx = state.tx.clone();
        let wake = self.wake.clone();
        let task = Box::new(move || {
            if !control.cancelled() {
                let result = run(preview.as_deref());
                if !control.cancelled() && tx.send(Event::Done(id, result)).is_ok() {
                    wake();
                }
            }
        });
        let result = if deterministic {
            state
                .preview_worker()
                .and_then(|worker| worker.submit(id, false, task))
        } else {
            state.spawn(task, None)
        };
        if let Err(error) = result {
            self.queue_system_result(id, Err(error.into()));
        }
    }

    fn queue_system_result(&self, id: u64, result: Outcome) {
        self.system_files
            .ready
            .borrow_mut()
            .push_back(Event::Done(id, result));
        (self.wake)();
    }

    pub(super) fn system_handle<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        root: Root,
    ) -> rquickjs::Result<Object<'js>> {
        let state = &self.system_files;
        if state.roots.borrow().len() >= MAX_HANDLES {
            return Err(throw_file_error(
                ctx,
                "E_LIMIT",
                "too many open system directories",
            ));
        }
        let id = state.id();
        let name = root
            .path()
            .file_name()
            .unwrap_or(root.path().as_os_str())
            .to_string_lossy();
        let handle = Object::new(ctx.clone())?;
        handle.prop(
            "__brand",
            data_property(Class::instance(
                ctx.clone(),
                SystemDirBrand {
                    id,
                    generation: self.generation,
                },
            )?),
        )?;
        handle.prop("kind", data_property("system-dir"))?;
        handle.prop("name", data_property(name.as_ref()))?;
        handle.prop("mode", data_property(root.mode().as_str()))?;
        let weak = Rc::downgrade(self);
        handle.prop(
            "close",
            data_property(Func::from(move || {
                if let Some(work) = weak.upgrade() {
                    work.system_files.roots.borrow_mut().remove(&id);
                }
            })),
        )?;
        let weak = Rc::downgrade(self);
        handle.prop(
            "lstat",
            data_property(Func::from(move |ctx: Ctx<'js>, rel: Opt<Value<'js>>| {
                let work = owner(&weak, &ctx)?;
                let scope = work.system_files.scope(
                    &ctx,
                    id,
                    FileMode::Read,
                    Arc::new(work.system_files.active_roots()),
                )?;
                let entry = EntryRef::new(scope, read_relative(&ctx, rel.0)?);
                work.system_read(&ctx, move |preview| {
                    let entry = match preview {
                        Some(fs) => fs.lstat(&entry),
                        None => entry.scope.lstat(&entry.relative_path),
                    }?;
                    json(&entry)
                })
            })),
        )?;
        let weak = Rc::downgrade(self);
        handle.prop(
            "readDir",
            data_property(Func::from(
                move |ctx: Ctx<'js>, rel: Opt<Value<'js>>, opts: Opt<Option<Object<'js>>>| {
                    let work = owner(&weak, &ctx)?;
                    let scope = work.system_files.scope(
                        &ctx,
                        id,
                        FileMode::Read,
                        Arc::new(work.system_files.active_roots()),
                    )?;
                    let entry = EntryRef::new(scope, read_relative(&ctx, rel.0)?);
                    let options = read_options(&ctx, opts.0.flatten())?;
                    let locale = work.locale.clone();
                    work.system_read(&ctx, move |preview| {
                        let page = match preview {
                            Some(fs) => fs.read_dir(&entry, &options, &locale),
                            None => entry
                                .scope
                                .read_dir(&entry.relative_path, &options, &locale),
                        }?;
                        json(&page)
                    })
                },
            )),
        )?;
        state.roots.borrow_mut().insert(id, root);
        Ok(handle)
    }

    fn system_handle_id(&self, ctx: &Ctx<'_>, value: Value<'_>) -> rquickjs::Result<u64> {
        let obj = value
            .into_object()
            .ok_or_else(|| Exception::throw_type(ctx, "expected a SystemDirHandle"))?;
        let brand: Value = obj.get("__brand")?;
        let brand = Class::<SystemDirBrand>::from_value(&brand)
            .map_err(|_| Exception::throw_type(ctx, "invalid or forged SystemDirHandle"))?;
        let brand = brand.borrow();
        if brand.generation != self.generation {
            return Err(throw_file_error(
                ctx,
                "InvalidState",
                "system directory belongs to another session",
            ));
        }
        Ok(brand.id)
    }

    fn entry_ref(
        &self,
        ctx: &Ctx<'_>,
        value: &Object<'_>,
        mode: FileMode,
        roots: &Arc<Vec<Root>>,
    ) -> rquickjs::Result<EntryRef> {
        let relative = read_relative(ctx, value.get("relativePath")?)?;
        let id = self.system_handle_id(ctx, value.get("dir")?)?;
        let scope = self.system_files.scope(ctx, id, mode, roots.clone())?;
        Ok(EntryRef::new(scope, relative))
    }

    fn open_system_directory<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        path: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        self.system_files.require_mode(ctx, FileMode::Read)?;
        let path = read_bounded_utf8(ctx, path, 4096, "system directory declaration")?;
        let grant = self
            .resolved
            .system_directories()
            .iter()
            .find(|grant| grant.declaration == path || grant.path.as_os_str() == path.as_str())
            .ok_or_else(|| {
                throw_file_error(
                    ctx,
                    "E_CAPABILITY",
                    "system directory is not declared in the package",
                )
            })?;
        self.system_files.require_mode(ctx, grant.mode)?;
        let root = self
            .system_files
            .opened(&grant.path, grant.mode)
            .map_err(|error| throw(ctx, error))?;
        self.system_handle(ctx, root)
    }

    fn system_directory_dialog<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        opts: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        self.system_files.require_mode(ctx, FileMode::Read)?;
        if !self.in_user_gesture() {
            return Err(throw_file_error(
                ctx,
                "E_CAPABILITY",
                "system directory dialog requires an active user gesture",
            ));
        }
        let (options, read_only) = parse_dialog_options(ctx, opts)?;
        let mode = if read_only || !self.system_files.capabilities.write {
            FileMode::Read
        } else {
            FileMode::ReadWrite
        };
        let (id, control, request) = self.system_request(ctx, None, false)?;
        let scripted = self.system_files.preview().map(|fs| {
            fs.dialogs
                .lock()
                .expect("fixture system directory dialog queue poisoned")
                .pop_front()
                .ok_or_else(|| {
                    fault(
                        "E_FIXTURE_MISSING_SERVICE",
                        "fixture has no system directory dialog response",
                    )
                })
        });
        self.dispatch_system_read(id, control, move |preview| {
            let root = match preview {
                Some(fs) => scripted
                    .expect("fixture dialog admitted with a scripted response")?
                    .map(|path| fs.open(&path, mode))
                    .transpose()?,
                None => run_file_chooser(PortalDialogKind::OpenDirectory, &options)?
                    .map(|path| Root::open(&path, mode))
                    .transpose()?,
            };
            Ok(Completed::Directory(
                root.map(Root::with_user_authorization),
            ))
        });
        Ok(request)
    }

    fn persist_system_grant<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        value: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        self.system_files.require_mode(ctx, FileMode::Read)?;
        let id = self.system_handle_id(ctx, value)?;
        let state = &self.system_files;
        let root =
            state.roots.borrow().get(&id).cloned().ok_or_else(|| {
                throw_file_error(ctx, "InvalidState", "system directory is closed")
            })?;
        state.require_mode(ctx, root.mode())?;
        root.require_user_authorization()
            .map_err(|error| throw(ctx, error))?;
        if let Some(vfs) = self.preview_workspace_files() {
            let (id, _, request) = self.system_request(ctx, None, false)?;
            let result = vfs
                .borrow_mut()
                .persist_bookmark(
                    GrantKind::SystemDir,
                    root.path().to_str().expect("fixture paths are UTF-8"),
                    root.mode(),
                    0,
                )
                .and_then(|token| json(&token));
            self.queue_system_result(id, result);
            return Ok(request);
        }
        let app_id = state.app_id.clone();
        self.system_read(ctx, move |_| {
            let store = resolve_grant_store_path(&app_id)?;
            let record = persist_grant(
                &store,
                &app_id,
                GrantKind::SystemDir,
                root.path(),
                root.mode(),
            )?;
            json(&record.token)
        })
    }

    fn restore_system_grant<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        token: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        self.system_files.require_mode(ctx, FileMode::Read)?;
        let token = read_bounded_utf8(ctx, token, 256, "system grant token")?;
        let state = &self.system_files;
        let allow_write = state.capabilities.write;
        if let Some(vfs) = self.preview_workspace_files() {
            let (id, _, request) = self.system_request(ctx, None, false)?;
            let result = (|| {
                let vfs = vfs.borrow();
                let record = vfs
                    .list_grants()
                    .iter()
                    .find(|record| record.token == token)
                    .ok_or_else(|| fault("NotFound", "system grant is missing or revoked"))?;
                authorize_system_grant(record, allow_write)?;
                let root = state
                    .preview()
                    .expect("workspace and system files use the same fixture run")
                    .reopen(Path::new(&record.scope.path), record.mode)?
                    .with_user_authorization();
                Ok(Completed::Directory(Some(root)))
            })();
            self.queue_system_result(id, result);
            return Ok(request);
        }
        let app_id = state.app_id.clone();
        self.system_read(ctx, move |_| {
            let record =
                restore_system_grant(&resolve_grant_store_path(&app_id)?, &token, allow_write)?;
            Ok(Completed::Directory(Some(
                Root::reopen(record.scope.path.into(), record.mode)?.with_user_authorization(),
            )))
        })
    }

    fn start_file_job<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        spec: Object<'js>,
        callbacks: Option<Object<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        let state = &self.system_files;
        state.require_mode(ctx, FileMode::Read)?;
        let method =
            read_bounded_utf8(ctx, spec.get("operation")?, 32, "system file job operation")?;
        let read = FileMode::Read;
        let write = FileMode::ReadWrite;
        if matches!(method.as_str(), "trash" | "listTrash" | "restoreFromTrash") {
            require(ctx, state.capabilities.trash, "trash")?;
        }
        if method != "listTrash" {
            state.require_mode(ctx, write)?;
        }
        state.reap();
        if state.job_count() >= MAX_JOBS {
            return Err(throw_file_error(
                ctx,
                "E_LIMIT",
                "too many active file jobs",
            ));
        }
        let roots = Arc::new(state.active_roots());
        let entry = |key| self.entry_ref(ctx, &spec.get::<_, Object>(key)?, write, &roots);
        let entries = |key, mode| {
            let array: Array = spec.get(key)?;
            bounded_array(ctx, &array, |item| self.entry_ref(ctx, &item, mode, &roots))
        };
        let job = match method.as_str() {
            "copy" => Job::Copy {
                sources: entries("sources", read)?,
                destination: entry("destination")?,
            },
            "move" => Job::Move {
                sources: entries("sources", write)?,
                destination: entry("destination")?,
            },
            "trash" => Job::Trash {
                entries: entries("entries", write)?,
            },
            "listTrash" => Job::ListTrash {
                roots: (*roots).clone(),
            },
            "restoreFromTrash" => Job::RestoreFromTrash {
                ids: bounded_array(ctx, &spec.get("ids")?, |value| {
                    read_bounded_utf8(ctx, value, 256, "Trash entry ID")
                })?,
                roots: (*roots).clone(),
            },
            "removePermanent" => Job::RemovePermanent {
                entries: entries("entries", write)?,
                recursive: spec.get::<_, Option<bool>>("recursive")?.unwrap_or(false),
            },
            "mkdir" => Job::Mkdir {
                entry: entry("entry")?,
            },
            "createSymlink" => Job::CreateSymlink {
                entry: entry("entry")?,
                target: read_bounded_utf8(ctx, spec.get("target")?, 4096, "symlink target")?,
            },
            "setPermissions" => Job::SetPermissions {
                entry: entry("entry")?,
                mode: spec.get("mode")?,
            },
            _ => {
                return Err(Exception::throw_type(
                    ctx,
                    "unknown system file job operation",
                ));
            }
        };
        let (id, control, request) = self.system_request(ctx, callbacks, true)?;
        let (conflict_handler, progress_handler) = {
            let pending = state.pending.borrow();
            let pending = &pending[&id];
            (pending.on_conflict.is_some(), pending.on_progress.is_some())
        };
        let preview = state.preview().cloned();
        let deterministic = preview.is_some();
        let fixture_worker = if deterministic {
            match state.preview_worker() {
                Ok(worker) => Some(worker),
                Err(error) => {
                    self.queue_system_result(id, Err(error.into()));
                    return Ok(request);
                }
            }
        } else {
            None
        };
        let fixture_tasks = fixture_worker.map(|worker| worker.tasks.clone());
        let tx = state.tx.clone();
        let wake = self.wake.clone();
        let task = Box::new(move || {
            let conflict = |info| {
                if conflict_handler {
                    control.ask(info, id, &tx, &wake, fixture_tasks.as_ref())
                } else {
                    ConflictResolution::CANCEL
                }
            };
            let mut last = None::<Instant>;
            let progress = |progress: &fs::Progress| {
                if !progress_handler {
                    return;
                }
                if deterministic || progress.items_completed == progress.items_total {
                    if tx.send(Event::Progress(id, progress.clone())).is_ok() {
                        wake();
                    }
                } else if last.is_none_or(|sent| sent.elapsed() >= PROGRESS_INTERVAL)
                    && tx.try_send(Event::Progress(id, progress.clone())).is_ok()
                {
                    last = Some(Instant::now());
                    wake();
                }
            };
            let result = match preview {
                Some(fs) => fs.run_job(job, control.flag(), conflict, progress),
                None => fs::run_job(
                    job,
                    Trash::from_environment().as_ref(),
                    control.flag(),
                    conflict,
                    progress,
                ),
            }
            .and_then(|result| match result {
                JobResult::Done => Ok(Completed::Void),
                JobResult::Trash(entries) => json(&entries),
            });
            if tx.send(Event::Done(id, result)).is_ok() {
                wake();
            }
        });
        let result = if let Some(worker) = fixture_worker {
            worker.submit(id, true, task)
        } else {
            state.spawn(task, Some(true))
        };
        if let Err(error) = result {
            self.queue_system_result(id, Err(error.into()));
        }
        Ok(request)
    }

    fn watch_system_volumes<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        callback: Function<'js>,
    ) -> rquickjs::Result<Function<'js>> {
        self.require_creation(ctx)?;
        let state = &self.system_files;
        require(ctx, state.capabilities.volumes, "volumes")?;
        state.reap();
        if state.watches.borrow().len() >= MAX_WATCHERS
            || state
                .workers
                .borrow()
                .iter()
                .filter(|worker| !worker.job)
                .count()
                >= MAX_WATCHERS
        {
            return Err(throw_file_error(ctx, "E_LIMIT", "too many volume watches"));
        }
        let id = state.id();
        let control = Arc::new(Control::default());
        let weak = Rc::downgrade(self);
        let stop = Function::new(ctx.clone(), move || {
            if let Some(work) = weak.upgrade()
                && let Some(watch) = work.system_files.watches.borrow_mut().remove(&id)
            {
                watch.control.cancel();
            }
        })?;
        if let Some(preview) = state.preview() {
            state
                .ready
                .borrow_mut()
                .push_back(Event::Volumes(id, preview.volumes.clone()));
            state.watches.borrow_mut().insert(
                id,
                Watch {
                    callback: Persistent::save(ctx, callback),
                    control,
                },
            );
            (self.wake)();
            return Ok(stop);
        }
        let tx = state.tx.clone();
        let wake = self.wake.clone();
        let watch_control = control.clone();
        state
            .spawn(
                Box::new(move || {
                    let mut prior = None;
                    loop {
                        if watch_control.cancelled() {
                            return;
                        }
                        if let Ok(values) = fs::list_volumes()
                            && prior.as_ref() != Some(&values)
                            && tx.try_send(Event::Volumes(id, values.clone())).is_ok()
                        {
                            prior = Some(values);
                            wake();
                        }
                        if watch_control.stop_or_timeout(Duration::from_millis(750)) {
                            return;
                        }
                    }
                }),
                Some(false),
            )
            .map_err(|error| throw(ctx, error.into()))?;
        state.watches.borrow_mut().insert(
            id,
            Watch {
                callback: Persistent::save(ctx, callback),
                control,
            },
        );
        Ok(stop)
    }

    fn cached_system_thumbnail<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        options: Object<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        require(ctx, self.system_files.capabilities.thumbnails, "thumbnails")?;
        let entry = self.entry_ref(
            ctx,
            &options,
            FileMode::Read,
            &Arc::new(self.system_files.active_roots()),
        )?;
        let size = match options.get::<_, Option<String>>("size")?.as_deref() {
            None | Some("normal") => ThumbnailSize::Normal,
            Some("large") => ThumbnailSize::Large,
            Some("x-large") => ThumbnailSize::XLarge,
            Some("xx-large") => ThumbnailSize::XxLarge,
            _ => return Err(Exception::throw_type(ctx, "invalid thumbnail size")),
        };
        self.system_read(ctx, move |preview| {
            let bytes = match preview {
                Some(fs) => fs.thumbnail(&entry, size)?,
                None => match fs::cache_home() {
                    Some(cache) => {
                        fs::cached_thumbnail(&entry.scope, &entry.relative_path, size, &cache)?
                            .map(Arc::from)
                    }
                    None => None,
                },
            };
            Ok(Completed::Image(bytes))
        })
    }

    /// Only tokens minted from a trusted native gesture can replace a gesture.
    pub(crate) fn remember_external_activation(&self, token: &str) {
        let state = &self.system_files;
        if !state.capabilities.open_external || token.is_empty() || token.len() > 4096 {
            return;
        }
        let mut tokens = state.external_tokens.borrow_mut();
        if tokens.iter().any(|issued| issued == token) {
            return;
        }
        if tokens.len() == MAX_EXTERNAL_TOKENS {
            tokens.pop_front();
        }
        tokens.push_back(token.to_owned());
    }

    fn open_external<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        options: Object<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        require(
            ctx,
            self.system_files.capabilities.open_external,
            "openExternal",
        )?;
        let token = options
            .get::<_, Option<Value>>("activationToken")?
            .map(|v| read_bounded_utf8(ctx, v, 4096, "activation token"))
            .transpose()?;
        if token
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.chars().any(char::is_control))
        {
            return Err(Exception::throw_type(ctx, "invalid activation token"));
        }
        if token.is_none() && !self.in_user_gesture() {
            return Err(throw_file_error(
                ctx,
                "E_CAPABILITY",
                "openExternal requires an active gesture or activation token",
            ));
        }
        let url = options
            .get::<_, Option<Value>>("url")?
            .filter(|url| !url.is_undefined());
        let target = match url {
            Some(value) => {
                let raw = read_bounded_utf8(ctx, value, 8192, "external URL")?;
                External::Url(fs::external_url(&raw).map_err(|error| throw(ctx, error))?)
            }
            None => External::Entry(self.entry_ref(
                ctx,
                &options,
                FileMode::Read,
                &Arc::new(self.system_files.active_roots()),
            )?),
        };
        if let Some(token) = &token {
            let mut tokens = self.system_files.external_tokens.borrow_mut();
            let index = tokens
                .iter()
                .position(|issued| issued == token)
                .ok_or_else(|| {
                    throw_file_error(
                        ctx,
                        "E_CAPABILITY",
                        "activation token was not minted for this session or was already used",
                    )
                })?;
            tokens.remove(index);
        }
        self.system_read(ctx, move |preview| {
            let uri = match target {
                External::Url(url) => Ok(url),
                External::Entry(entry) => {
                    follow(preview, &entry, FileMode::Read).and_then(|path| fs::file_uri(&path))
                }
            };
            if preview.is_some() {
                return Ok(Completed::External {
                    uri,
                    token_present: token.is_some(),
                });
            }
            fs::launch_external(&uri?, token.as_deref())?;
            Ok(Completed::Void)
        })
    }

    pub(crate) fn system_drag_uris<'js>(
        &self,
        ctx: &Ctx<'js>,
        entries: Array<'js>,
        mode: FileMode,
    ) -> rquickjs::Result<Vec<u8>> {
        self.require_creation(ctx)?;
        self.system_files.require_mode(ctx, mode)?;
        let roots = Arc::new(self.system_files.active_roots());
        let uris = bounded_array(ctx, &entries, |object| {
            let entry = self.entry_ref(ctx, &object, mode, &roots)?;
            follow(self.system_files.preview().map(Arc::as_ref), &entry, mode)
                .and_then(|path| fs::file_uri(&path))
                .map_err(|error| throw(ctx, error))
        })?;
        let bytes: usize = uris.iter().map(|uri| uri.len() + 2).sum();
        let mut payload = Vec::with_capacity(bytes);
        for uri in uris {
            payload.extend_from_slice(uri.as_bytes());
            payload.extend_from_slice(b"\r\n");
        }
        Ok(payload)
    }
}

enum External {
    Url(String),
    Entry(EntryRef),
}

fn follow(
    preview: Option<&PreviewSystemFiles>,
    entry: &EntryRef,
    mode: FileMode,
) -> Result<PathBuf, ServiceError> {
    match preview {
        Some(fs) => fs.follow(entry, mode),
        None if mode.can_write() => entry.scope.follow_for_move(&entry.relative_path),
        None => entry.scope.follow(&entry.relative_path),
    }
}

fn volumes(preview: Option<&PreviewSystemFiles>) -> Result<Vec<Volume>, ServiceError> {
    preview.map_or_else(fs::list_volumes, |fs| Ok(fs.volumes.clone()))
}

fn read_relative(ctx: &Ctx<'_>, value: Option<Value<'_>>) -> rquickjs::Result<String> {
    value.filter(|value| !value.is_undefined()).map_or_else(
        || Ok(String::new()),
        |value| read_bounded_utf8(ctx, value, 4096, "relative system path"),
    )
}

fn bounded_array<'js, T, R>(
    ctx: &Ctx<'js>,
    array: &Array<'js>,
    mut item: impl FnMut(T) -> rquickjs::Result<R>,
) -> rquickjs::Result<Vec<R>>
where
    T: rquickjs::FromJs<'js>,
{
    let count: u32 = array.as_object().get("length")?;
    if !(1..=MAX_ENTRIES_PER_JOB).contains(&count) {
        return Err(Exception::throw_range(
            ctx,
            "system file operations require 1..=1024 entries",
        ));
    }
    (0..count as usize)
        .map(|index| item(array.get(index)?))
        .collect()
}

fn serialize<'js>(ctx: &Ctx<'js>, value: &impl Serialize) -> rquickjs::Result<Value<'js>> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?;
    ctx.json_parse(bytes)
}

fn decision(value: &Value<'_>) -> ConflictResolution {
    use fs::ConflictAction;
    let read = || -> rquickjs::Result<ConflictResolution> {
        let Some(value) = value.as_object() else {
            return Ok(ConflictResolution::CANCEL);
        };
        let action = match value.get::<_, String>("action")?.as_str() {
            "overwrite" => ConflictAction::Overwrite,
            "skip" => ConflictAction::Skip,
            "autoRename" => ConflictAction::AutoRename,
            _ => return Ok(ConflictResolution::CANCEL),
        };
        Ok(ConflictResolution {
            action,
            apply_to_all: value.get::<_, Option<bool>>("applyToAll")?.unwrap_or(false),
        })
    };
    match read() {
        Ok(decision) => decision,
        Err(error) => {
            if error.is_exception() {
                let _ = value.ctx().catch();
            }
            ConflictResolution::CANCEL
        }
    }
}

fn read_options(ctx: &Ctx<'_>, opts: Option<Object<'_>>) -> rquickjs::Result<ReadOptions> {
    let mut options = ReadOptions::default();
    if let Some(opts) = opts {
        options.cursor = opts
            .get::<_, Option<Value>>("cursor")?
            .map(|value| read_bounded_utf8(ctx, value, 64, "pagination cursor"))
            .transpose()?;
        options.locale = opts
            .get::<_, Option<Value>>("locale")?
            .map(|value| read_bounded_utf8(ctx, value, 128, "collation locale"))
            .transpose()?;
        if let Some(size) = opts.get::<_, Option<u32>>("pageSize")? {
            options.page_size = size as usize;
        }
        if let Some(hidden) = opts.get("includeHidden")? {
            options.include_hidden = hidden;
        }
        if let Some(first) = opts.get("directoriesFirst")? {
            options.directories_first = first;
        }
        if let Some(numeric) = opts.get("numericCollation")? {
            options.numeric_collation = numeric;
        }
        options.sort_by = match opts.get::<_, Option<String>>("sortBy")?.as_deref() {
            None | Some("name") => SortBy::Name,
            Some("size") => SortBy::Size,
            Some("modifiedMs") => SortBy::ModifiedMs,
            Some("kind") => SortBy::Kind,
            _ => {
                return Err(Exception::throw_type(
                    ctx,
                    "invalid system directory sort key",
                ));
            }
        };
        options.sort_direction = match opts.get::<_, Option<String>>("sortDirection")?.as_deref() {
            None | Some("asc") => SortDirection::Asc,
            Some("desc") => SortDirection::Desc,
            _ => {
                return Err(Exception::throw_type(
                    ctx,
                    "invalid system directory sort direction",
                ));
            }
        };
    }
    Ok(options)
}
