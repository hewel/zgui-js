//! Owner-thread roots and registrations; workers receive only owned service requests.
use std::{cell::{Cell, OnceCell, RefCell}, collections::{HashMap, VecDeque, hash_map::Entry}, fmt::Write, rc::{Rc, Weak}, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Duration};
use host_services::Completion;
use rquickjs::{Array, Ctx, Exception, Function, IntoJs, Object, Persistent, TypedArray, Value, function::{Func, Opt, Rest}, object::Property};
use zgui::{platform::{SurfaceId, WakeReason, Waker}, runtime::{AppTimers, ImageBlobs}, view::{HostHandle, host::{FrameRequestId, Repeat, TimerId}}, vocab::Timestamp};
use crate::{assets::{AssetError, AssetProvider, AssetSource, Delivery, MAX_SAFE_ID}, package::Package, shell::Shell};
use crate::runtime_capabilities::ResolvedCapabilities;

mod clipboard;
mod external;
mod files;
mod network;
mod sqlite;
mod workers;

pub(crate) use workers::WorkerStep;

const MAX_CALLBACKS: usize = 1024;
// Match the binary socket frame cap; direct images have a separate session budget.
const MAX_DIRECT_IMAGE_BYTES: u32 = 8 * 1024 * 1024;
const MAX_DIRECT_IMAGES: u32 = 64;
const MAX_DIRECT_BYTES_PER_SESSION: u32 = 64 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode { Loading, Mounting, Mounted, Stopping, Disposed }
#[derive(Clone, Copy)]
enum ImageLease { Package, Socket(usize), Direct(u32) }

impl ImageLease {
    fn charge(self, work: &Work) {
        self.update(work, true);
    }

    fn release(self, work: &Work) {
        self.update(work, false);
    }

    fn update(self, work: &Work, charge: bool) {
        match self {
            Self::Package => {}
            Self::Socket(bytes) => {
                if charge {
                    work.external.charge_image(bytes);
                } else {
                    work.external.release_image(bytes);
                }
            }
            Self::Direct(bytes) => {
                let mut budget = work.direct_images.get();
                if charge {
                    budget.handles += 1;
                    budget.bytes += bytes;
                } else {
                    budget.handles -= 1;
                    budget.bytes -= bytes;
                }
                work.direct_images.set(budget);
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct DirectImageBudget { handles: u32, bytes: u32 }
#[derive(Clone, Copy)]
pub(crate) enum Arguments { Timer, Frame(f64) }
pub(crate) struct ScheduledCall { id: u64, pub callback: Persistent<Function<'static>>, pub arguments: Arguments }
enum NativeRegistration {
    Timer(TimerId),
    Frame { host: HostHandle, id: FrameRequestId, pinned: bool },
}
struct Registration { native: NativeRegistration, callback: Persistent<Function<'static>> }
struct Pending { resolve: Persistent<Function<'static>>, reject: Persistent<Function<'static>>, cancelled: Arc<AtomicBool> }

struct FrameHostEntry {
    surface: SurfaceId,
    host: HostHandle,
    occluded: bool,
}

enum Scheduler {
    Window(HostHandle),
    Shell(AppTimers),
}

/// Package and service authority owned by one generation of Work.
pub(crate) struct WorkSession {
    pub(crate) package: Option<Rc<Package>>,
    pub(crate) generation: u64,
    pub(crate) assets: AssetSource,
    pub(crate) resolved: ResolvedCapabilities,
}

pub(crate) struct Work {
    registrations: RefCell<HashMap<u64, Registration>>,
    ready: RefCell<VecDeque<ScheduledCall>>,
    requests: RefCell<HashMap<u64, Pending>>,
    mode: Cell<Mode>,
    cleanup_shell: OnceCell<Rc<Shell>>,
    gesture_source: OnceCell<Rc<ui_bridge::BridgeGroup>>,
    generation: u64,
    scheduler: Scheduler,
    frame_hosts: RefCell<Vec<FrameHostEntry>>,
    provider: Box<dyn AssetProvider>,
    image_blobs: ImageBlobs,
    direct_images: Cell<DirectImageBudget>,
    clipboard: clipboard::ClipboardState,
    files: files::FilesState,
    network: network::NetworkState,
    sqlite: sqlite::SqliteState,
    workers: workers::WorkersState,
    external: external::External,
    resolved: ResolvedCapabilities,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Work {
    /// Builds the owner-thread services for a primary surface.
    pub(crate) fn new(session: WorkSession, host: HostHandle, surface: SurfaceId, waker: Arc<dyn Waker>) -> Rc<Self> {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || waker.wake(WakeReason::ReactiveWork { surfaces: vec![surface].into_boxed_slice() }));
        let frame_hosts = vec![FrameHostEntry { surface, host: host.clone(), occluded: false }];
        Self::with_scheduler(session, wake, Scheduler::Window(host), frame_hosts)
    }

    /// Uses application timing and wakes that remain valid without surfaces.
    pub(crate) fn new_shell(session: WorkSession, timers: AppTimers, waker: Arc<dyn Waker>) -> Rc<Self> {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || waker.wake(WakeReason::AppWork));
        Self::with_scheduler(session, wake, Scheduler::Shell(timers), Vec::new())
    }

    fn with_scheduler(
        session: WorkSession,
        wake: Arc<dyn Fn() + Send + Sync>,
        scheduler: Scheduler,
        frame_hosts: Vec<FrameHostEntry>,
    ) -> Rc<Self> {
        let WorkSession { package, generation, assets, resolved } = session;
        let allow_clipboard_read = package
            .as_ref()
            .is_some_and(|pkg| pkg.manifest.capabilities.clipboard.read);
        let files = files::FilesState::new(package.clone(), &assets, wake.clone());
        let network = network::NetworkState::new(package.clone(), &assets);
        let sqlite = sqlite::SqliteState::new(package.clone(), &assets);
        let workers = workers::WorkersState::new(package.clone(), &assets);
        let external = external::External::new(package.clone(), &assets);
        Rc::new(Self {
            registrations: RefCell::new(HashMap::new()), ready: RefCell::new(VecDeque::new()), requests: RefCell::new(HashMap::new()),
            mode: Cell::new(Mode::Loading), cleanup_shell: OnceCell::new(), gesture_source: OnceCell::new(), generation, scheduler,
            frame_hosts: RefCell::new(frame_hosts),
            provider: assets.bind(package, generation, wake.clone()),
            image_blobs: ImageBlobs::new(generation),
            direct_images: Cell::default(),
            clipboard: clipboard::ClipboardState::new(allow_clipboard_read),
            files,
            network,
            sqlite,
            workers,
            external,
            resolved,
            wake,
        })
    }

    /// Share the session's disposal barrier with every Work creation path.
    pub(crate) fn guard_cleanup_with(&self, shell: Rc<Shell>) {
        assert!(self.cleanup_shell.set(shell).is_ok(), "work cleanup barrier must be bound once for generation {}", self.generation);
    }

    /// Share the session's bridge group so gesture-gated services can verify active user input.
    pub(crate) fn set_gesture_source(&self, bridges: Rc<ui_bridge::BridgeGroup>) {
        assert!(self.gesture_source.set(bridges).is_ok(), "work gesture source must be bound once for generation {}", self.generation);
    }

    pub(crate) fn in_user_gesture(&self) -> bool {
        self.gesture_source.get().is_some_and(|bridges| bridges.in_user_gesture())
    }

    /// Registers a live surface as an available frame source.
    pub(crate) fn add_frame_host(self: &Rc<Self>, surface: SurfaceId, host: HostHandle) {
        {
            let mut hosts = self.frame_hosts.borrow_mut();
            if hosts.iter().any(|entry| entry.host.ptr_eq(&host)) {
                return;
            }
            hosts.push(FrameHostEntry { surface, host: host.clone(), occluded: false });
        }
        self.migrate_unscoped_frames(&host, |current| !current.ptr_eq(&host) && self.is_frame_host_occluded(current));
    }

    /// Removes a retiring surface and migrates any remaining pending frame callbacks to a
    /// live non-occluded surface rather than cancelling them mid-flight.
    pub(crate) fn remove_frame_host(self: &Rc<Self>, removed: &HostHandle) {
        self.frame_hosts.borrow_mut().retain(|entry| !entry.host.ptr_eq(removed));
        let replacement = self.pick_frame_host();
        let mut migrated = false;
        let mut registrations = self.registrations.borrow_mut();
        let ids: Vec<u64> = registrations
            .iter()
            .filter_map(|(&id, reg)| match &reg.native {
                NativeRegistration::Frame { host, .. } if host.ptr_eq(removed) => Some(id),
                _ => None,
            })
            .collect();
        for id in ids {
            let Some(reg) = registrations.get_mut(&id) else { continue; };
            let NativeRegistration::Frame { host, id: old_native, .. } = &reg.native else { continue; };
            host.cancel_frame_callback(*old_native);
            if let Some(target) = &replacement {
                let new_native = self.schedule_native_frame(id, target);
                reg.native = NativeRegistration::Frame { host: target.clone(), id: new_native, pinned: false };
                migrated = true;
            } else {
                registrations.remove(&id);
            }
        }
        drop(registrations);
        if migrated {
            (self.wake)();
        }
    }

    /// Updates occlusion state for a surface and migrates unscoped callbacks off occluded hosts.
    pub(crate) fn set_frame_host_occluded(self: &Rc<Self>, surface: SurfaceId, occluded: bool) {
        let changed = {
            let mut hosts = self.frame_hosts.borrow_mut();
            let Some(entry) = hosts.iter_mut().find(|entry| entry.surface == surface) else { return; };
            if entry.occluded == occluded {
                None
            } else {
                entry.occluded = occluded;
                Some(entry.host.clone())
            }
        };
        let Some(changed_host) = changed else { return; };
        if occluded {
            if let Some(replacement) = self.pick_non_occluded_host_other_than(&changed_host) {
                self.migrate_unscoped_frames(&replacement, |current| current.ptr_eq(&changed_host));
            }
        } else {
            self.migrate_unscoped_frames(&changed_host, |current| {
                !current.ptr_eq(&changed_host) && self.is_frame_host_occluded(current)
            });
        }
    }

    fn has_frame_host(&self, target: &HostHandle) -> bool {
        self.frame_hosts.borrow().iter().any(|entry| entry.host.ptr_eq(target))
    }

    fn pick_frame_host(&self) -> Option<HostHandle> {
        let hosts = self.frame_hosts.borrow();
        hosts
            .iter()
            .find(|entry| !entry.occluded)
            .or_else(|| hosts.first())
            .map(|entry| entry.host.clone())
    }

    fn pick_non_occluded_host_other_than(&self, excluded: &HostHandle) -> Option<HostHandle> {
        self.frame_hosts
            .borrow()
            .iter()
            .find(|entry| !entry.occluded && !entry.host.ptr_eq(excluded))
            .map(|entry| entry.host.clone())
    }

    fn is_frame_host_occluded(&self, target: &HostHandle) -> bool {
        self.frame_hosts
            .borrow()
            .iter()
            .find(|entry| entry.host.ptr_eq(target))
            .is_some_and(|entry| entry.occluded)
    }

    fn schedule_native_frame(self: &Rc<Self>, id: u64, host: &HostHandle) -> FrameRequestId {
        let weak = Rc::downgrade(self);
        host.request_frame_callback(Rc::new(move |timestamp: Timestamp| {
            if let Some(owner) = weak.upgrade() {
                owner.fire(id, Arguments::Frame(timestamp.since_origin().as_secs_f64() * 1000.0));
            }
        }))
    }

    fn migrate_unscoped_frames(self: &Rc<Self>, to: &HostHandle, should_migrate: impl Fn(&HostHandle) -> bool) {
        let mut migrated = false;
        let mut registrations = self.registrations.borrow_mut();
        for (&id, reg) in registrations.iter_mut() {
            if let NativeRegistration::Frame { host, id: old_native, pinned: false } = &reg.native
                && should_migrate(host)
            {
                host.cancel_frame_callback(*old_native);
                let new_native = self.schedule_native_frame(id, to);
                reg.native = NativeRegistration::Frame { host: to.clone(), id: new_native, pinned: false };
                migrated = true;
            }
        }
        drop(registrations);
        if migrated {
            (self.wake)();
        }
    }

    /// Attaches a region-scoped `requestAnimationFrame` onto a bridge API object so springs and
    /// effects inside that surface tick on its own Wayland/native frame cadence.
    pub(crate) fn install_region_frame_api<'js>(
        self: &Rc<Self>,
        api: &Object<'js>,
        host: HostHandle,
    ) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        api.set("requestAnimationFrame", Func::from(move |ctx: Ctx<'js>, callback: Function<'js>| {
            owner(&weak, &ctx)?.register_frame(&ctx, callback, Some(host.clone()))
        }))
    }

    pub(crate) fn contract_failure(&self) -> Option<crate::assets::ContractFailure> {
        self.provider.contract_failure()
    }

    pub(crate) fn image_blobs(&self) -> &ImageBlobs { &self.image_blobs }

    /// Reclaim image storage after the retiring session has counted leaked handles.
    pub(crate) fn clear_images(&self) {
        self.image_blobs.clear();
        self.direct_images.set(DirectImageBudget::default());
    }

    /// Returns the path grants used by service admission and native images.
    #[must_use]
    pub(crate) fn resolved_capabilities(&self) -> &ResolvedCapabilities { &self.resolved }

    /// Registers host-owned encoded bytes directly in the active session.
    pub(crate) fn register_image_bytes(&self, bytes: Arc<[u8]>) -> std::io::Result<String> {
        if !self.can_create_work() {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "image registration requires an active session"));
        }
        self.image_blobs.register(bytes)
    }

    /// Provider rejection to a JavaScript exception. Type/Range map to the
    /// ordinary exception classes; Contract is the sticky fixture failure with
    /// its machine-readable code.
    fn asset_error(&self, ctx: &Ctx<'_>, error: AssetError) -> rquickjs::Error {
        match error {
            AssetError::Type(message) => Exception::throw_type(ctx, &message),
            AssetError::Range(message) => Exception::throw_range(ctx, &message),
            AssetError::Contract(failure) => match service_error(ctx, failure.code, &failure.message) {
                Ok(error) => ctx.throw(error.into_value()),
                Err(error) => error,
            },
        }
    }
    pub(crate) fn mounting(&self) { self.mode.set(Mode::Mounting); }
    pub(crate) fn mounted(&self) { self.mode.set(Mode::Mounted); }
    pub(crate) fn has_ready(&self) -> bool { !self.ready.borrow().is_empty() }
    pub(crate) fn next_call(&self) -> Option<ScheduledCall> { self.ready.borrow_mut().pop_front() }
    pub(crate) fn next_completion(&self) -> Option<Completion> {
        if self.mode.get() != Mode::Mounted { return None; }
        self.provider.next_completion()
    }
    pub(crate) fn has_completions(&self) -> bool { self.provider.has_completions() }
    pub(crate) fn pending_count(&self) -> usize {
        self.ready.borrow().len()
            + self.requests.borrow().len()
            + self.clipboard.pending_count()
            + self.sqlite.pending_count()
            + self.files.pending_count()
    }
    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        let registrations = self.registrations.borrow();
        let ready = self.ready.borrow();
        let timers = registrations.values().filter(|item| matches!(item.native, NativeRegistration::Timer(_))).count()
            + ready.iter().filter(|item| matches!(item.arguments, Arguments::Timer)).count();
        (timers, registrations.len() + ready.len() - timers, self.requests.borrow().len() + self.clipboard.pending_count() + self.sqlite.pending_count() + self.files.pending_count())
    }

    fn can_create_work(&self) -> bool {
        matches!(self.mode.get(), Mode::Mounting | Mode::Mounted)
            && self.cleanup_shell.get().is_none_or(|shell| !shell.cleanup_in_progress())
    }

    fn require_creation(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        if !self.can_create_work() {
            return Err(Exception::throw_type(ctx, "host work is unavailable during preflight or disposal"));
        }
        Ok(())
    }
    fn id(&self, ctx: &Ctx<'_>) -> rquickjs::Result<u64> {
        self.provider.registration_id().ok_or_else(||Exception::throw_range(ctx, "host registration identifiers exhausted"))
    }

    pub(crate) fn install<'js>(self: &Rc<Self>, ctx: &Ctx<'js>) -> rquickjs::Result<()> {
        let console = Object::new(ctx.clone())?;
        for level in ["log", "info", "warn", "error", "debug"] {
            console.set(level, Func::from(move |values: Rest<Value<'js>>| -> rquickjs::Result<()> {
                let mut line = String::with_capacity(256);
                for value in values.0.iter().take(32) {
                    if line.len() >= 4096 { break; }
                    if !line.is_empty() { line.push(' '); }
                    if let Some(string) = value.as_string() {
                        let string = string.clone().to_cstring()?;
                        let text: &str = string.as_ref();
                        let mut end = text.len().min(4096 - line.len());
                        while !text.is_char_boundary(end) { end -= 1; }
                        line.push_str(&text[..end]);
                    } else if let Some(number) = value.as_number() { let _ = write!(line, "{number}"); }
                    else if let Some(boolean) = value.as_bool() { let _ = write!(line, "{boolean}"); }
                    else { let _ = write!(line, "<{}>", value.type_of().as_str()); }
                }
                eprintln!("js[{level}]: {line}");
                Ok(())
            }))?;
        }
        ctx.globals().set("console", console)?;
        let weak = Rc::downgrade(self);
        ctx.globals().set("setTimeout", Func::from(move |ctx: Ctx<'js>, callback: Function<'js>, delay: Opt<f64>| {
            let owner = owner(&weak, &ctx)?;
            let delay = delay.0.unwrap_or(0.0);
            if !delay.is_finite() || !(0.0..=u32::MAX as f64).contains(&delay) { return Err(Exception::throw_range(&ctx, "timeout delay must be finite nonnegative milliseconds within u32 range")); }
            owner.register_timer(&ctx, callback, Duration::from_secs_f64(delay / 1000.0))
        }))?;
        let weak = Rc::downgrade(self);
        ctx.globals().set("requestAnimationFrame", Func::from(move |ctx: Ctx<'js>, callback: Function<'js>| owner(&weak, &ctx)?.register_frame(&ctx, callback, None)))?;
        for (name, frame) in [("clearTimeout", false), ("cancelAnimationFrame", true)] {
            let weak = Rc::downgrade(self);
            ctx.globals().set(name, Func::from(move |ctx: Ctx<'js>, value: f64| {
                let id = registration_id(&ctx, value)?;
                owner(&weak, &ctx)?.cancel_registration(id, frame);
                Ok::<(), rquickjs::Error>(())
            }))?;
        }
        let services = Object::new(ctx.clone())?;
        let weak = Rc::downgrade(self);
        services.set("readTextAsset", Func::from(move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            let owner = owner(&weak, &ctx)?;
            owner.provider.validate_arguments(args.0.len(), args.0.first().is_some_and(|value| value.is_string()))
                .map_err(|error| owner.asset_error(&ctx, error))?;
            let value = args.0.into_iter().next().unwrap_or_else(|| Value::new_undefined(ctx.clone()));
            let path: String = rquickjs::FromJs::from_js(&ctx, value)?;
            owner.read_asset(&ctx, path)
        }))?;
        let weak = Rc::downgrade(self);
        services.set("registerImageAsset", Func::from(move |ctx: Ctx<'js>, path: Opt<Value<'js>>, extra: Opt<Value<'js>>| {
            let owner = owner(&weak, &ctx)?;
            owner.require_creation(&ctx)?;
            let (Some(path), None) = (path.0, extra.0) else {
                return Err(Exception::throw_type(&ctx, "registerImageAsset requires exactly one string path"));
            };
            let path: String = rquickjs::FromJs::from_js(&ctx, path)?;
            let bytes = owner.provider.image_asset(&path).map_err(|error| owner.asset_error(&ctx, error))?;
            owner.image_handle(&ctx, bytes, ImageLease::Package)
        }))?;
        let weak = Rc::downgrade(self);
        services.set("registerImageBytes", Func::from(move |ctx: Ctx<'js>, bytes: Opt<Value<'js>>, extra: Opt<Value<'js>>| {
            let owner = owner(&weak, &ctx)?;
            owner.require_creation(&ctx)?;
            let (Some(bytes), None) = (bytes.0, extra.0) else {
                return Err(Exception::throw_type(&ctx, "registerImageBytes requires exactly one Uint8Array"));
            };
            owner.direct_image_handle(&ctx, bytes)
        }))?;
        let weak = Rc::downgrade(self);
        services.prop("getResolvedCapabilities", data_property(Func::from(move |ctx: Ctx<'js>| {
            owner(&weak, &ctx)?.capability_snapshot(&ctx)
        })))?;
        self.install_clipboard(&services)?;
        self.install_files(&services)?;
        self.install_network(&services)?;
        self.install_sqlite(&services)?;
        self.install_workers(&services)?;
        self.install_external(&services)?;
        ctx.globals().set("__zguiServices", services)?;
        Ok(())
    }

    fn capability_snapshot<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        let paths = |paths: &[String]| -> rquickjs::Result<Array<'js>> {
            let array = Array::new(ctx.clone())?;
            for (index, path) in paths.iter().enumerate() {
                let index = u32::try_from(index).map_err(|_| Exception::throw_range(ctx, "too many capability paths"))?;
                array.as_object().prop(index, data_property(path.as_str()))?;
            }
            Ok(array)
        };
        let snapshot = Object::new(ctx.clone())?;
        let named = Object::new(ctx.clone())?;
        named.set_prototype(None)?;
        for (name, endpoint) in self.resolved.named_sockets() {
            named.prop(name.as_str(), data_property(endpoint_value(ctx, endpoint.as_deref())?))?;
        }
        snapshot.prop("imagePrefixes", data_property(paths(self.resolved.image_prefixes())?))?;
        snapshot.prop("unixSockets", data_property(paths(self.resolved.unix_sockets())?))?;
        snapshot.prop("namedSockets", data_property(named))?;
        Ok(snapshot)
    }

    fn reserve_callback_id(&self, ctx: &Ctx<'_>) -> rquickjs::Result<u64> {
        self.require_creation(ctx)?;
        if self.registrations.borrow().len() + self.ready.borrow().len() >= MAX_CALLBACKS {
            return Err(Exception::throw_range(ctx, "native scheduled callback limit reached"));
        }
        self.id(ctx)
    }

    fn register_timer<'js>(self: &Rc<Self>, ctx: &Ctx<'js>, callback: Function<'js>, delay: Duration) -> rquickjs::Result<f64> {
        let id = self.reserve_callback_id(ctx)?;
        let weak = Rc::downgrade(self);
        let native_cb = Rc::new(move || {
            if let Some(owner) = weak.upgrade() { owner.fire(id, Arguments::Timer); }
        });
        let timer = match &self.scheduler {
            Scheduler::Window(host) => host.schedule(delay, Repeat::Once, native_cb),
            Scheduler::Shell(timers) => timers.schedule(delay, native_cb)
                .ok_or_else(|| Exception::throw_reference(ctx, "application timer scheduler is unavailable"))?,
        };
        self.registrations.borrow_mut().insert(id, Registration {
            native: NativeRegistration::Timer(timer),
            callback: Persistent::save(ctx, callback),
        });
        Ok(id as f64)
    }

    fn register_frame<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        callback: Function<'js>,
        explicit_host: Option<HostHandle>,
    ) -> rquickjs::Result<f64> {
        let id = self.reserve_callback_id(ctx)?;
        let (host, pinned) = match explicit_host {
            Some(host) if self.has_frame_host(&host) => (host, true),
            _ => {
                let fallback = self.pick_frame_host()
                    .ok_or_else(|| Exception::throw_reference(ctx, "requestAnimationFrame requires an open surface"))?;
                (fallback, false)
            }
        };
        let native_id = self.schedule_native_frame(id, &host);
        self.registrations.borrow_mut().insert(id, Registration {
            native: NativeRegistration::Frame { host, id: native_id, pinned },
            callback: Persistent::save(ctx, callback),
        });
        Ok(id as f64)
    }

    fn fire(&self, id: u64, arguments: Arguments) {
        let registration = self.registrations.borrow_mut().remove(&id);
        let Some(registration) = registration else { return; };
        if self.mode.get() != Mode::Mounted { return; }
        // The owner checkpoint follows native delivery. Drain the batch with
        // a shared budget, rather than resetting a budget per timer or frame.
        self.ready.borrow_mut().push_back(ScheduledCall { id, callback: registration.callback, arguments });
        if matches!(self.scheduler, Scheduler::Shell(_)) && matches!(arguments, Arguments::Timer) {
            (self.wake)();
        }
    }
    fn cancel_registration(&self, id: u64, frame: bool) {
        let mut ready = self.ready.borrow_mut();
        if let Some(index) = ready.iter().position(|call| call.id == id && matches!(call.arguments, Arguments::Frame(_)) == frame) {
            ready.remove(index);
            return;
        }
        drop(ready);
        let mut registrations = self.registrations.borrow_mut();
        let Entry::Occupied(entry) = registrations.entry(id) else { return; };
        if matches!(entry.get().native, NativeRegistration::Timer(_)) == frame { return; }
        let registration = entry.remove();
        drop(registrations);
        self.cancel_native(registration.native);
    }
    fn cancel_native(&self, native: NativeRegistration) {
        match native {
            NativeRegistration::Timer(id) => match &self.scheduler {
                Scheduler::Window(host) => host.cancel_timer(id),
                Scheduler::Shell(timers) => timers.cancel(id),
            },
            NativeRegistration::Frame { host, id, .. } => host.cancel_frame_callback(id),
        }
    }

    fn read_asset<'js>(self: &Rc<Self>, ctx: &Ctx<'js>, path: String) -> rquickjs::Result<Object<'js>> {
        self.provider.validate_request(&path, self.can_create_work(), self.requests.borrow().len())
            .map_err(|error| self.asset_error(ctx, error))?;
        let id = self.id(ctx)?;
        let (promise, resolve, reject) = ctx.promise()?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let object = Object::new(ctx.clone())?;
        object.set("promise", promise)?;
        let weak = Rc::downgrade(self);
        object.set("cancel", Func::from(move |ctx: Ctx<'js>| owner(&weak, &ctx)?.cancel_request(&ctx, id)))?;
        // The owner cannot drain a completion until this JS turn returns, so
        // submitting before the Pending entry exists cannot lose a result.
        match self.provider.submit(id, path, cancelled.clone()).map_err(|error| self.asset_error(ctx, error))? {
            Some(error) => { reject.call::<_, ()>((service_error(ctx, error.code, &error.message)?,))?; }
            None => { self.requests.borrow_mut().insert(id, Pending { resolve: Persistent::save(ctx, resolve), reject: Persistent::save(ctx, reject), cancelled }); }
        }
        Ok(object)
    }

    fn direct_image_handle<'js>(self: &Rc<Self>, ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Object<'js>> {
        let view = TypedArray::<u8>::from_value(value)
            .map_err(|_| Exception::throw_type(ctx, "image bytes must be a Uint8Array"))?;
        // Use the intrinsic view length: TypedArray::len reads a shadowable JS property.
        let raw = view.as_raw().ok_or_else(|| Exception::throw_type(ctx, "image bytes view is detached or out of bounds"))?;
        let len = match u32::try_from(raw.len()) {
            Ok(len) if (1..=MAX_DIRECT_IMAGE_BYTES).contains(&len) => len,
            _ => return Err(Exception::throw_range(ctx, "image bytes must contain between 1 byte and 8 MiB")),
        };
        let budget = self.direct_images.get();
        if budget.handles >= MAX_DIRECT_IMAGES || len > MAX_DIRECT_BYTES_PER_SESSION - budget.bytes {
            return Err(Exception::throw_range(ctx, "session direct image budget exceeded"));
        }
        // SAFETY: the view keeps the backing buffer alive. Arc::from copies the
        // slice before any JS can run and mutate, resize, or detach the buffer.
        let bytes = Arc::from(unsafe { raw.as_ref() });
        self.image_handle(ctx, bytes, ImageLease::Direct(len))
    }

    fn image_handle<'js>(self: &Rc<Self>, ctx: &Ctx<'js>, bytes: Arc<[u8]>, lease: ImageLease) -> rquickjs::Result<Object<'js>> {
        let object = Object::new(ctx.clone())?;
        let src = self.register_image_bytes(bytes)
            .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?;
        let initialized = (|| {
            object.prop("src", data_property(src.as_str()))?;
            let id = src.clone();
            let weak = Rc::downgrade(self);
            object.prop("release", data_property(Func::from(move || {
                if let Some(owner) = weak.upgrade() && owner.image_blobs.release(&id) {
                    lease.release(&owner);
                    if owner.mode.get() == Mode::Mounted { (owner.wake)(); }
                }
            })))
        })();
        if let Err(error) = initialized {
            self.image_blobs.release(&src);
            return Err(error);
        }
        lease.charge(self);
        Ok(object)
    }

    fn cancel_request<'js>(&self, ctx: &Ctx<'js>, id: u64) -> rquickjs::Result<()> {
        let pending = self.requests.borrow_mut().remove(&id);
        let Some(pending) = pending else { return Ok(()); };
        pending.cancelled.store(true, Ordering::Release);
        self.provider.cancel(id);
        pending.reject.restore(ctx)?.call::<_, ()>((service_error(ctx, "Cancelled", "asset request cancelled")?,))
    }

    pub(crate) fn settle<'js>(&self, ctx: &Ctx<'js>, completion: Completion) -> rquickjs::Result<()> {
        if completion.generation != self.generation || self.mode.get() != Mode::Mounted {
            self.provider.settled(completion.request_id, Delivery::Ignored("stale generation or inactive session"));
            return Ok(());
        }
        let pending = self.requests.borrow_mut().remove(&completion.request_id);
        let Some(pending) = pending else {
            self.provider.settled(completion.request_id, Delivery::Ignored("request already settled or cancelled"));
            return Ok(());
        };
        self.provider.settled(completion.request_id, Delivery::Delivered { failed: completion.result.is_err() });
        match completion.result {
            Ok(text) => pending.resolve.restore(ctx)?.call::<_, ()>((text,)),
            Err(error) => pending.reject.restore(ctx)?.call::<_, ()>((service_error(ctx, error.code, &error.message)?,)),
        }
    }

    pub(crate) fn begin_stop(&self) {
        if matches!(self.mode.get(), Mode::Stopping | Mode::Disposed) { return; }
        self.mode.set(Mode::Stopping);
        for pending in self.requests.borrow_mut().drain().map(|(_, pending)| pending) { pending.cancelled.store(true, Ordering::Release); }
        self.clipboard.clear();
        self.workers.stop_all();
        self.provider.stop();
    }
    pub(crate) fn finish_stop(&self) {
        if self.mode.get() == Mode::Disposed { return; }
        self.begin_stop();
        for registration in self.registrations.borrow_mut().drain().map(|(_, registration)| registration) { self.cancel_native(registration.native); }
        self.ready.borrow_mut().clear();
        self.frame_hosts.borrow_mut().clear();
        self.clipboard.clear();
        self.files.clear();
        self.network.clear(self.generation);
        self.sqlite.clear();
        self.workers.clear();
        self.external.clear(self.generation);
        self.mode.set(Mode::Disposed);
    }
}
impl Drop for Work { fn drop(&mut self) { self.finish_stop(); } }

fn owner(weak: &Weak<Work>, ctx: &Ctx<'_>) -> rquickjs::Result<Rc<Work>> {
    weak.upgrade().ok_or_else(|| Exception::throw_reference(ctx, "host work session is disposed"))
}
fn registration_id(ctx: &Ctx<'_>, value: f64) -> rquickjs::Result<u64> {
    if !value.is_finite() || value.fract() != 0.0 || !(1.0..=MAX_SAFE_ID as f64).contains(&value) {
        return Err(Exception::throw_range(ctx, "registration ID must be a positive safe integer"));
    }
    Ok(value as u64)
}
fn endpoint_value<'js>(ctx: &Ctx<'js>, endpoint: Option<&str>) -> rquickjs::Result<Value<'js>> {
    match endpoint {
        Some(path) => path.into_js(ctx),
        None => Ok(Value::new_null(ctx.clone())),
    }
}

pub(crate) fn service_error<'js>(ctx: &Ctx<'js>, code: &str, message: &str) -> rquickjs::Result<Object<'js>> {
    let error = Exception::from_message(ctx.clone(), message)?.into_object();
    error.prop("code", data_property(code))?;
    Ok(error)
}

// Define own data properties without invoking application-installed prototype setters.
fn data_property<T>(value: T) -> Property<T> {
    Property::from(value).writable().enumerable().configurable()
}

#[cfg(test)]
mod tests;
