//! Owner-thread roots and registrations; workers receive only owned service requests.
use std::{cell::{Cell, RefCell}, collections::{BTreeMap, HashMap, VecDeque}, fmt::Write, rc::{Rc, Weak}, sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc::{self, Receiver, SyncSender}}, time::Duration};
use host_services::{Completion, Executor, Request};
use rquickjs::{Ctx, Exception, Function, Object, Persistent, Value, function::{Func, Opt, Rest}, object::Property};
use zgui::{platform::{SurfaceId, WakeReason, Waker}, view::{HostHandle, host::{FrameRequestId, Repeat, TimerId}}};
use crate::package::Package;

mod niri;
pub(crate) mod fixture;
use niri::NiriListener;

const MAX_SAFE_ID: u64 = 9_007_199_254_740_991;
const MAX_CALLBACKS: usize = 1024;
const MAX_REQUESTS: usize = 64;
const MAX_ASSET_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode { Loading, Mounting, Mounted, Stopping, Disposed }
#[derive(Clone, Copy)]
pub(crate) enum Arguments { Timer, Frame(f64) }
pub(crate) struct ScheduledCall { id: u64, pub callback: Persistent<Function<'static>>, pub arguments: Arguments }
enum NativeRegistration { Timer(TimerId), Frame(FrameRequestId) }
struct Registration { native: NativeRegistration, callback: Persistent<Function<'static>> }
struct Pending { resolve: Persistent<Function<'static>>, reject: Persistent<Function<'static>>, cancelled: Arc<AtomicBool> }

#[derive(Clone)]
pub(crate) enum ServicePolicy { Live, Fixture(Rc<fixture::FixtureRuntime>) }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContractFailure {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for ContractFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ContractFailure {}

pub(crate) struct Work {
    registrations: RefCell<HashMap<u64, Registration>>,
    ready: RefCell<VecDeque<ScheduledCall>>,
    requests: RefCell<HashMap<u64, Pending>>,
    subscriptions: RefCell<BTreeMap<u64, NiriListener>>,
    subscription_cursor: Cell<u64>,
    pending_completion: RefCell<Option<Completion>>,
    next_id: Cell<u64>,
    mode: Cell<Mode>,
    generation: u64,
    host: HostHandle,
    package: Option<Rc<Package>>,
    executor: Executor,
    sender: RefCell<Option<SyncSender<Completion>>>,
    inbox: RefCell<Option<Receiver<Completion>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
    service_policy: ServicePolicy,
}

impl Work {
    pub(crate) fn new(host: HostHandle, package: Option<Rc<Package>>, generation: u64, executor: Executor, waker: Arc<dyn Waker>, surface: SurfaceId, service_policy: ServicePolicy) -> Rc<Self> {
        let (sender, inbox) = mpsc::sync_channel(MAX_REQUESTS);
        Rc::new(Self {
            registrations: RefCell::new(HashMap::new()), ready: RefCell::new(VecDeque::new()), requests: RefCell::new(HashMap::new()),
            subscriptions: RefCell::new(BTreeMap::new()), subscription_cursor: Cell::new(0),
            pending_completion: RefCell::new(None), next_id: Cell::new(1), mode: Cell::new(Mode::Loading), generation, host, package, executor,
            sender: RefCell::new(Some(sender)), inbox: RefCell::new(Some(inbox)),
            wake: Arc::new(move || waker.wake(WakeReason::ReactiveWork { surfaces: vec![surface].into_boxed_slice() })),
            service_policy,
        })
    }

    pub(crate) fn contract_failure(&self) -> Option<ContractFailure> {
        self.fixtures().and_then(|runtime| runtime.contract_failure())
    }

    fn fixtures(&self) -> Option<&Rc<fixture::FixtureRuntime>> {
        match &self.service_policy { ServicePolicy::Live => None, ServicePolicy::Fixture(runtime) => Some(runtime) }
    }
    fn fixture_error(&self, ctx: &Ctx<'_>, error: ContractFailure) -> rquickjs::Error {
        match service_error(ctx, error.code, &error.message) {
            Ok(error) => ctx.throw(error.into_value()),
            Err(error) => error,
        }
    }
    fn malformed_service(&self, ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
        if let Some(runtime) = self.fixtures() { self.fixture_error(ctx, runtime.malformed(message)) }
        else { Exception::throw_type(ctx, message) }
    }
    pub(crate) fn mounting(&self) { self.mode.set(Mode::Mounting); }
    pub(crate) fn mounted(&self) { self.mode.set(Mode::Mounted); }
    pub(crate) fn has_ready(&self) -> bool { !self.ready.borrow().is_empty() }
    pub(crate) fn next_call(&self) -> Option<ScheduledCall> { self.ready.borrow_mut().pop_front() }
    pub(crate) fn next_completion(&self) -> Option<Completion> {
        if self.mode.get() != Mode::Mounted { return None; }
        if let Some(runtime) = self.fixtures() {
            return match runtime.take(true)? {
                fixture::Delivery::Asset { generation, id, result } => Some(Completion { generation, request_id: id, result }),
                fixture::Delivery::Niri { .. } => unreachable!("asset queue head"),
            };
        }
        self.pending_completion.borrow_mut().take().or_else(|| self.inbox.borrow().as_ref()?.try_recv().ok())
    }
    pub(crate) fn has_completions(&self) -> bool {
        if let Some(runtime) = self.fixtures() { return runtime.has_delivery(true); }
        if self.pending_completion.borrow().is_some() { return true; }
        let completion = self.inbox.borrow().as_ref().and_then(|inbox| inbox.try_recv().ok());
        let pending = completion.is_some();
        *self.pending_completion.borrow_mut() = completion;
        pending
    }
    pub(crate) fn counts(&self) -> (usize, usize, usize, usize) {
        let registrations = self.registrations.borrow();
        let ready = self.ready.borrow();
        let timers = registrations.values().filter(|item| matches!(item.native, NativeRegistration::Timer(_))).count()
            + ready.iter().filter(|item| matches!(item.arguments, Arguments::Timer)).count();
        (timers, registrations.len() + ready.len() - timers, self.requests.borrow().len(), self.subscriptions.borrow().len())
    }

    fn require_creation(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        if !matches!(self.mode.get(), Mode::Mounting | Mode::Mounted) {
            return Err(Exception::throw_type(ctx, "host work is unavailable during preflight or disposal"));
        }
        Ok(())
    }
    fn id(&self, ctx: &Ctx<'_>) -> rquickjs::Result<u64> {
        if let Some(runtime) = self.fixtures() {
            return runtime.registration_id().ok_or_else(||Exception::throw_range(ctx, "host registration identifiers exhausted"));
        }
        let id = self.next_id.get();
        if id > MAX_SAFE_ID { return Err(Exception::throw_range(ctx, "host registration identifiers exhausted")); }
        self.next_id.set(id + 1);
        Ok(id)
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
            if owner.fixtures().is_some() && (args.0.len() != 1 || !args.0[0].is_string()) {
                return Err(owner.malformed_service(&ctx, "readTextAsset requires exactly one string path"));
            }
            let value = args.0.into_iter().next().unwrap_or_else(|| Value::new_undefined(ctx.clone()));
            let path: String = rquickjs::FromJs::from_js(&ctx, value)?;
            owner.read_asset(&ctx, path)
        }))?;
        let weak = Rc::downgrade(self);
        services.set("watchNiri", Func::from(move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
            let owner = owner(&weak, &ctx)?;
            if owner.fixtures().is_some() && (args.0.len() != 2 || args.0.iter().any(|value| !value.is_function())) {
                return Err(owner.malformed_service(&ctx, "watchNiri requires exactly two callback functions"));
            }
            let mut args = args.0.into_iter();
            let snapshot: Function = rquickjs::FromJs::from_js(&ctx, args.next().unwrap_or_else(|| Value::new_undefined(ctx.clone())))?;
            let error: Function = rquickjs::FromJs::from_js(&ctx, args.next().unwrap_or_else(|| Value::new_undefined(ctx.clone())))?;
            owner.watch_niri(&ctx, snapshot, error)
        }))?;
        ctx.globals().set("__zguiServices", services)?;
        Ok(())
    }

    fn register<'js>(self: &Rc<Self>, ctx: &Ctx<'js>, callback: Function<'js>, delay: Option<Duration>) -> rquickjs::Result<f64> {
        self.require_creation(ctx)?;
        if self.registrations.borrow().len() + self.ready.borrow().len() >= MAX_CALLBACKS {
            return Err(Exception::throw_range(ctx, "native scheduled callback limit reached"));
        }
        let id = self.id(ctx)?;
        let weak = Rc::downgrade(self);
        let native = if let Some(delay) = delay {
            NativeRegistration::Timer(self.host.schedule(delay, Repeat::Once, Rc::new(move || {
                if let Some(owner) = weak.upgrade() { owner.fire(id, Arguments::Timer); }
            })))
        } else {
            NativeRegistration::Frame(self.host.request_frame_callback(Rc::new(move |timestamp| {
                if let Some(owner) = weak.upgrade() { owner.fire(id, Arguments::Frame(timestamp.since_origin().as_secs_f64() * 1000.0)); }
            })))
        };
        self.registrations.borrow_mut().insert(id, Registration { native, callback: Persistent::save(ctx, callback) });
        Ok(id as f64)
    }

    fn fire(&self, id: u64, arguments: Arguments) {
        let registration = self.registrations.borrow_mut().remove(&id);
        let Some(registration) = registration else { return; };
        if self.mode.get() != Mode::Mounted { return; }
        // Native callbacks run before layout. One binding checkpoint drains this
        // batch with a shared budget, rather than resetting a budget per timer.
        self.ready.borrow_mut().push_back(ScheduledCall { id, callback: registration.callback, arguments });
    }
    fn cancel_registration(&self, id: u64, frame: bool) {
        let mut ready = self.ready.borrow_mut();
        if let Some(index) = ready.iter().position(|call| call.id == id && matches!(call.arguments, Arguments::Frame(_)) == frame) {
            ready.remove(index);
            return;
        }
        drop(ready);
        let mut registrations = self.registrations.borrow_mut();
        if registrations.get(&id).is_none_or(|registration| matches!(registration.native, NativeRegistration::Frame(_)) != frame) { return; }
        let registration = registrations.remove(&id).unwrap();
        drop(registrations);
        self.cancel_native(registration.native);
    }
    fn cancel_native(&self, native: NativeRegistration) {
        match native { NativeRegistration::Timer(id) => self.host.cancel_timer(id), NativeRegistration::Frame(id) => self.host.cancel_frame_callback(id) }
    }

    fn read_asset<'js>(self: &Rc<Self>, ctx: &Ctx<'js>, path: String) -> rquickjs::Result<Object<'js>> {
        if let Some(runtime) = self.fixtures() { runtime.require_assets().map_err(|error| self.fixture_error(ctx, error))?; }
        if let Err(error) = self.require_creation(ctx) {
            if self.fixtures().is_some() { return Err(self.malformed_service(ctx, "readTextAsset is unavailable during preflight or disposal")); }
            return Err(error);
        }
        if self.fixtures().is_none() {
            let package = self.package.as_ref().ok_or_else(|| Exception::throw_type(ctx, "asset requests require a loaded package"))?;
            if !package.manifest.assets.contains(&path) { return Err(Exception::throw_range(ctx, "asset is not declared in this package")); }
        }
        if self.requests.borrow().len() >= MAX_REQUESTS {
            return Err(if self.fixtures().is_some() { self.malformed_service(ctx, "outstanding request limit reached") }
                else { Exception::throw_range(ctx, "outstanding request limit reached") });
        }
        let id = self.id(ctx)?;
        if let Some(runtime) = self.fixtures() {
            runtime.read_asset(self.generation, id, &path, self.package.as_deref()).map_err(|error| self.fixture_error(ctx, error))?;
        }
        let (promise, resolve, reject) = ctx.promise()?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let object = Object::new(ctx.clone())?;
        object.set("promise", promise)?;
        let weak = Rc::downgrade(self);
        object.set("cancel", Func::from(move |ctx: Ctx<'js>| owner(&weak, &ctx)?.cancel_request(&ctx, id)))?;
        self.requests.borrow_mut().insert(id, Pending { resolve: Persistent::save(ctx, resolve), reject: Persistent::save(ctx, reject), cancelled: cancelled.clone() });
        if self.fixtures().is_some() { return Ok(object); }
        let package = self.package.as_ref().expect("live package validated before request creation");
        let reply = self.sender.borrow().as_ref().expect("creation mode retains completion sender").clone();
        let request = Request { generation: self.generation, request_id: id, root: package.directory.clone(), path, max_bytes: MAX_ASSET_BYTES, cancelled, reply, wake: self.wake.clone() };
        if let Err(error) = self.executor.submit(request) {
            let pending = self.requests.borrow_mut().remove(&id).unwrap();
            let code = match error { host_services::SubmitError::QueueFull => "Busy", host_services::SubmitError::Stopped => "Stopped" };
            pending.reject.restore(ctx)?.call::<_, ()>((service_error(ctx, code, &error.to_string())?,))?;
        }
        Ok(object)
    }

    fn cancel_request<'js>(&self, ctx: &Ctx<'js>, id: u64) -> rquickjs::Result<()> {
        let pending = self.requests.borrow_mut().remove(&id);
        let Some(pending) = pending else { return Ok(()); };
        pending.cancelled.store(true, Ordering::Release);
        if let Some(runtime) = self.fixtures() { runtime.cancel(id); }
        pending.reject.restore(ctx)?.call::<_, ()>((service_error(ctx, "Cancelled", "asset request cancelled")?,))
    }

    pub(crate) fn settle<'js>(&self, ctx: &Ctx<'js>, completion: Completion) -> rquickjs::Result<()> {
        if completion.generation != self.generation || self.mode.get() != Mode::Mounted {
            if let Some(runtime) = self.fixtures() { runtime.ignored(completion.request_id, "stale generation or inactive session"); }
            return Ok(());
        }
        let pending = self.requests.borrow_mut().remove(&completion.request_id);
        let Some(pending) = pending else {
            if let Some(runtime) = self.fixtures() { runtime.ignored(completion.request_id, "request already settled or cancelled"); }
            return Ok(());
        };
        if let Some(runtime) = self.fixtures() { runtime.delivered(completion.request_id, completion.result.is_err()); }
        match completion.result {
            Ok(text) => pending.resolve.restore(ctx)?.call::<_, ()>((text,)),
            Err(error) => pending.reject.restore(ctx)?.call::<_, ()>((service_error(ctx, error.code, &error.message)?,)),
        }
    }

    pub(crate) fn begin_stop(&self) {
        if matches!(self.mode.get(), Mode::Stopping | Mode::Disposed) { return; }
        self.mode.set(Mode::Stopping);
        if let Some(runtime) = self.fixtures() { runtime.cancel_generation(self.generation); }
        self.subscriptions.borrow_mut().clear();
        for pending in self.requests.borrow_mut().drain().map(|(_, pending)| pending) { pending.cancelled.store(true, Ordering::Release); }
        // Retired workers must not block the shared executor on a full old inbox.
        self.inbox.borrow_mut().take();
        self.sender.borrow_mut().take();
        self.pending_completion.borrow_mut().take();
    }
    pub(crate) fn finish_stop(&self) {
        self.begin_stop();
        for registration in self.registrations.borrow_mut().drain().map(|(_, registration)| registration) { self.cancel_native(registration.native); }
        self.ready.borrow_mut().clear();
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
fn service_error<'js>(ctx: &Ctx<'js>, code: &str, message: &str) -> rquickjs::Result<Object<'js>> {
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
