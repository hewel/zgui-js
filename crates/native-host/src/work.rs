//! Owner-thread roots and registrations; workers receive only owned service requests.
use std::{cell::{Cell, OnceCell, RefCell}, collections::{HashMap, VecDeque, hash_map::Entry}, fmt::Write, rc::{Rc, Weak}, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Duration};
use host_services::Completion;
use rquickjs::{Array, Ctx, Exception, Function, IntoJs, Object, Persistent, Value, function::{Func, Opt, Rest}, object::Property};
use zgui::{platform::{SurfaceId, WakeReason, Waker}, runtime::{AppTimers, ImageBlobs}, view::{HostHandle, host::{FrameRequestId, Repeat, TimerId}}, vocab::Timestamp};
use crate::{assets::{AssetError, AssetProvider, AssetSource, Delivery, MAX_SAFE_ID}, package::Package, shell::Shell};
use crate::runtime_capabilities::ResolvedCapabilities;

mod external;

const MAX_CALLBACKS: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode { Loading, Mounting, Mounted, Stopping, Disposed }
#[derive(Clone, Copy)]
enum ImageLease { Package, Socket(usize) }
#[derive(Clone, Copy)]
pub(crate) enum Arguments { Timer, Frame(f64) }
pub(crate) struct ScheduledCall { id: u64, pub callback: Persistent<Function<'static>>, pub arguments: Arguments }
enum NativeRegistration {
    Timer(TimerId),
    WindowFrame(FrameRequestId),
    ShellFrame { host: HostHandle, id: FrameRequestId },
}
struct Registration { native: NativeRegistration, callback: Persistent<Function<'static>> }
struct Pending { resolve: Persistent<Function<'static>>, reject: Persistent<Function<'static>>, cancelled: Arc<AtomicBool> }

enum Scheduler {
    Window(HostHandle),
    Shell { timers: AppTimers, frame_host: RefCell<Option<HostHandle>> },
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
    generation: u64,
    scheduler: Scheduler,
    provider: Box<dyn AssetProvider>,
    image_blobs: ImageBlobs,
    external: external::External,
    resolved: ResolvedCapabilities,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Work {
    /// Builds the owner-thread services for a primary surface.
    pub(crate) fn new(session: WorkSession, host: HostHandle, surface: SurfaceId, waker: Arc<dyn Waker>) -> Rc<Self> {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || waker.wake(WakeReason::ReactiveWork { surfaces: vec![surface].into_boxed_slice() }));
        Self::with_scheduler(session, wake, Scheduler::Window(host))
    }

    /// Uses application timing and wakes that remain valid without surfaces.
    pub(crate) fn new_shell(session: WorkSession, timers: AppTimers, waker: Arc<dyn Waker>) -> Rc<Self> {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || waker.wake(WakeReason::AppWork));
        Self::with_scheduler(session, wake, Scheduler::Shell { timers, frame_host: RefCell::new(None) })
    }

    fn with_scheduler(session: WorkSession, wake: Arc<dyn Fn() + Send + Sync>, scheduler: Scheduler) -> Rc<Self> {
        let WorkSession { package, generation, assets, resolved } = session;
        let external = external::External::new(package.clone(), &assets);
        Rc::new(Self {
            registrations: RefCell::new(HashMap::new()), ready: RefCell::new(VecDeque::new()), requests: RefCell::new(HashMap::new()),
            mode: Cell::new(Mode::Loading), cleanup_shell: OnceCell::new(), generation, scheduler,
            provider: assets.bind(package, generation, wake.clone()),
            image_blobs: ImageBlobs::new(generation),
            external,
            resolved,
            wake,
        })
    }

    /// Share the session's disposal barrier with every Work creation path.
    pub(crate) fn guard_cleanup_with(&self, shell: Rc<Shell>) {
        assert!(self.cleanup_shell.set(shell).is_ok(), "work cleanup barrier must be bound once for generation {}", self.generation);
    }

    /// Sets the shell's native frame source. Retiring a source also cancels its pending frames.
    pub(crate) fn set_frame_host(&self, host: Option<HostHandle>) {
        let Scheduler::Shell { frame_host, .. } = &self.scheduler else { return; };
        let previous = {
            let mut frame_host = frame_host.borrow_mut();
            if let (Some(old), Some(new)) = (&*frame_host, &host)
                && old.ptr_eq(new)
            {
                return;
            }
            std::mem::replace(&mut *frame_host, host)
        };
        let Some(previous) = previous else { return; };
        self.registrations.borrow_mut().retain(|_, registration| {
            if let NativeRegistration::ShellFrame { host, id } = &registration.native
                && host.ptr_eq(&previous)
            {
                host.cancel_frame_callback(*id);
                return false;
            }
            true
        });
        self.ready.borrow_mut().retain(|call| matches!(call.arguments, Arguments::Timer));
    }

    pub(crate) fn contract_failure(&self) -> Option<crate::assets::ContractFailure> {
        self.provider.contract_failure()
    }

    pub(crate) fn image_blobs(&self) -> &ImageBlobs { &self.image_blobs }

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
    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        let registrations = self.registrations.borrow();
        let ready = self.ready.borrow();
        let timers = registrations.values().filter(|item| matches!(item.native, NativeRegistration::Timer(_))).count()
            + ready.iter().filter(|item| matches!(item.arguments, Arguments::Timer)).count();
        (timers, registrations.len() + ready.len() - timers, self.requests.borrow().len())
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
            owner.register(&ctx, callback, Some(Duration::from_secs_f64(delay / 1000.0)))
        }))?;
        let weak = Rc::downgrade(self);
        ctx.globals().set("requestAnimationFrame", Func::from(move |ctx: Ctx<'js>, callback: Function<'js>| owner(&weak, &ctx)?.register(&ctx, callback, None)))?;
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
        services.prop("getResolvedCapabilities", data_property(Func::from(move |ctx: Ctx<'js>| {
            owner(&weak, &ctx)?.capability_snapshot(&ctx)
        })))?;
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

    fn register<'js>(self: &Rc<Self>, ctx: &Ctx<'js>, callback: Function<'js>, delay: Option<Duration>) -> rquickjs::Result<f64> {
        self.require_creation(ctx)?;
        if self.registrations.borrow().len() + self.ready.borrow().len() >= MAX_CALLBACKS {
            return Err(Exception::throw_range(ctx, "native scheduled callback limit reached"));
        }
        let id = self.id(ctx)?;
        let weak = Rc::downgrade(self);
        let native = if let Some(delay) = delay {
            let callback = Rc::new(move || {
                if let Some(owner) = weak.upgrade() { owner.fire(id, Arguments::Timer); }
            });
            let timer = match &self.scheduler {
                Scheduler::Window(host) => host.schedule(delay, Repeat::Once, callback),
                Scheduler::Shell { timers, .. } => timers.schedule(delay, callback)
                    .ok_or_else(|| Exception::throw_reference(ctx, "application timer scheduler is unavailable"))?,
            };
            NativeRegistration::Timer(timer)
        } else {
            let callback = move |timestamp: Timestamp| {
                if let Some(owner) = weak.upgrade() { owner.fire(id, Arguments::Frame(timestamp.since_origin().as_secs_f64() * 1000.0)); }
            };
            match &self.scheduler {
                Scheduler::Window(host) => NativeRegistration::WindowFrame(host.request_frame_callback(Rc::new(callback))),
                Scheduler::Shell { frame_host, .. } => {
                    let host = frame_host.borrow().clone()
                        .ok_or_else(|| Exception::throw_reference(ctx, "requestAnimationFrame requires an open surface"))?;
                    let id = host.request_frame_callback(Rc::new(callback));
                    NativeRegistration::ShellFrame { host, id }
                }
            }
        };
        self.registrations.borrow_mut().insert(id, Registration { native, callback: Persistent::save(ctx, callback) });
        Ok(id as f64)
    }

    fn fire(&self, id: u64, arguments: Arguments) {
        let registration = self.registrations.borrow_mut().remove(&id);
        let Some(registration) = registration else { return; };
        if self.mode.get() != Mode::Mounted { return; }
        // The owner checkpoint follows native delivery. Drain the batch with
        // a shared budget, rather than resetting a budget per timer or frame.
        self.ready.borrow_mut().push_back(ScheduledCall { id, callback: registration.callback, arguments });
        if matches!(self.scheduler, Scheduler::Shell { .. }) && matches!(arguments, Arguments::Timer) {
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
                Scheduler::Shell { timers, .. } => timers.cancel(id),
            },
            NativeRegistration::WindowFrame(id) => {
                let Scheduler::Window(host) = &self.scheduler else { unreachable!("window registration in a shell scheduler"); };
                host.cancel_frame_callback(id);
            }
            NativeRegistration::ShellFrame { host, id } => host.cancel_frame_callback(id),
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
                    if let ImageLease::Socket(bytes) = lease { owner.external.release_image(bytes); }
                    if owner.mode.get() == Mode::Mounted { (owner.wake)(); }
                }
            })))
        })();
        if let Err(error) = initialized {
            self.image_blobs.release(&src);
            return Err(error);
        }
        if let ImageLease::Socket(bytes) = lease { self.external.charge_image(bytes); }
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
        self.provider.stop();
    }
    pub(crate) fn finish_stop(&self) {
        if self.mode.get() == Mode::Disposed { return; }
        self.begin_stop();
        for registration in self.registrations.borrow_mut().drain().map(|(_, registration)| registration) { self.cancel_native(registration.native); }
        self.ready.borrow_mut().clear();
        if let Scheduler::Shell { frame_host, .. } = &self.scheduler {
            frame_host.borrow_mut().take();
        }
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
