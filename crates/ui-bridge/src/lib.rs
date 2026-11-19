//! Owner-thread native tree bridge. JavaScript sees branded objects, never native IDs.
//! A structural turn is one outermost JavaScript entry; pending removals expire at its end.
#![forbid(unsafe_code)]

pub mod registry;
mod properties;
mod events;
mod dismiss;
mod focus;
mod geometry;
mod scroll;
mod canvas;

use std::{cell::{Cell, RefCell}, collections::{HashMap, VecDeque}, fmt, marker::PhantomData, rc::{Rc, Weak}, sync::Arc};
use rquickjs::{class::Trace, function::{Func, Opt}, Class, Ctx, Exception, JsLifetime, Object, Persistent, Value};
use registry::{BridgeError, Handle, NodeRegistry, Snapshot};

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct NativeNode {
    #[qjs(skip_trace)]
    handle: Handle,
}

/// Bounded capacity for deferred native notifications awaiting a host flush.
pub const MAX_QUEUED_NOTIFICATIONS: usize = 256;

/// Host-selected failure policy and motion preference, fixed for one session.
#[derive(Clone, Copy, Debug, Default)]
pub struct BridgeOptions {
    /// Preview rejects invalid style maps; live sessions retain their last valid map.
    pub strict_styles: bool,
    /// Suppresses shared SDK motion durations through ThemeProvider.
    pub reduced_motion: bool,
    /// Holds mount-generated sheets until the session admits them.
    pub defer_stylesheet_install: bool,
}

/// A rejected application styling contribution; unrelated props remain applicable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleDiagnostic {
    pub code: &'static str,
    pub message: String,
}

/// Returned by [`UiBridge::has_queued_notifications`] after queue overflow retires the bridge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueOverflow;

impl fmt::Display for QueueOverflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("nested native notification queue exhausted")
    }
}

impl std::error::Error for QueueOverflow {}

/// Outcome reported by a synchronous host dispatcher attempting immediate JS entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub enum DispatchOutcome {
    Executed,
    Faulted,
    Busy,
    Retired,
}

struct DeferredEvent {
    id: u64,
    snapshot: events::EventSnapshot,
}

enum QueuedNotification {
    Event(DeferredEvent),
    Geometry { id: u64, value: zgui::view::ObservedValue, scale: f32 },
}

enum DispatchPayload<'a> {
    LiveEvent {
        id: u64,
        snapshot: &'a mut Option<events::EventSnapshot>,
        apply: &'a mut dyn FnMut(events::DispatchControl),
    },
    QueuedEvent(DeferredEvent),
    Geometry {
        id: u64,
        slot: &'a mut Option<zgui::view::ObservedValue>,
        scale: f32,
    },
}

/// Callback handle handed to the host closure inside [`BridgeStep::run_turn`] or
/// [`UiBridge::flush_notification`].
#[must_use]
pub struct BridgeCallback<'a> {
    bridge: &'a UiBridge,
    payload: DispatchPayload<'a>,
}

impl<'a> BridgeCallback<'a> {
    /// Dispatches the notification into JavaScript inside the active structural turn.
    pub fn call<'js>(self, ctx: &Ctx<'js>) -> rquickjs::Result<()> {
        match self.payload {
            DispatchPayload::LiveEvent { id, snapshot, apply } => {
                let snapshot = snapshot.take().expect("live event snapshot consumed twice");
                self.bridge.invoke_snapshot(ctx, id, snapshot, Some(apply))
            }
            DispatchPayload::QueuedEvent(event) => {
                self.bridge.invoke_snapshot(ctx, event.id, event.snapshot, None)
            }
            DispatchPayload::Geometry { id, slot, scale } => {
                let value = slot.take().expect("geometry observation consumed twice");
                self.bridge.invoke_geometry_inner(ctx, id, value, scale)
            }
        }
    }
}

impl fmt::Debug for BridgeCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeCallback").finish_non_exhaustive()
    }
}

/// Pending synchronous native notification awaiting a host JS engine turn.
#[must_use]
pub struct BridgeStep<'a> {
    bridge: &'a UiBridge,
    payload: DispatchPayload<'a>,
}

impl fmt::Debug for BridgeStep<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeStep").finish_non_exhaustive()
    }
}

impl<'a> BridgeStep<'a> {
    /// Executes the notification inside an active bridge structural turn.
    pub fn run_turn<J, E>(
        self,
        mut js: J,
        enter: impl FnOnce(&mut J, BridgeCallback<'_>) -> Result<(), E>,
    ) -> Result<(), E> {
        let bridge = self.bridge;
        let payload = self.payload;
        bridge.execute_turn(|| enter(&mut js, BridgeCallback { bridge, payload }))
    }
}

/// Borrowed proof of an active structural turn inside [`UiBridge::run_turn`].
pub struct BridgeTurn<'turn> {
    bridge: &'turn UiBridge,
    _marker: PhantomData<&'turn mut UiBridge>,
}

impl fmt::Debug for BridgeTurn<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeTurn").finish_non_exhaustive()
    }
}

impl<'turn> BridgeTurn<'turn> {
    /// Exposes the native mount-region root wrapper within the active structural turn.
    pub fn root<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        self.bridge.root(ctx)
    }

    /// Drains one queued notification inside this active structural turn.
    /// Returns `Ok(false)` if the internal queue is empty or disposed.
    pub fn flush_notification<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<bool> {
        let item = self.bridge.pending.borrow_mut().pop_front();
        let Some(item) = item else {
            return Ok(false);
        };
        match item {
            QueuedNotification::Event(event) => {
                self.bridge.invoke_snapshot(ctx, event.id, event.snapshot, None)?;
            }
            QueuedNotification::Geometry { id, value, scale } => {
                self.bridge.invoke_geometry_inner(ctx, id, value, scale)?;
            }
        }
        Ok(true)
    }
}

#[derive(Clone, Default)]
enum WakeTarget {
    #[default]
    None,
    Live {
        surface: zgui::platform::SurfaceId,
        waker: Arc<dyn zgui::platform::Waker>,
    },
}

type HostDispatcher = Rc<dyn Fn(BridgeStep<'_>) -> DispatchOutcome>;

/// Host integration port supplied to [`UiBridge::new`] controlling whether idle
/// notifications dispatch synchronously or queue for the host's next flush.
#[derive(Clone, Default)]
pub struct HostPort {
    wake: WakeTarget,
    dispatch: Option<HostDispatcher>,
}

impl fmt::Debug for HostPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostPort")
            .field("synchronous", &self.dispatch.is_some())
            .finish_non_exhaustive()
    }
}

impl HostPort {
    /// Queues every notification internally until drained by [`UiBridge::flush_notification`]
    /// or [`BridgeTurn::flush_notification`].
    pub fn queued() -> Self {
        Self::default()
    }

    /// Attempts synchronous JS dispatch when idle using `enter`, and signals `waker`
    /// for `surface` whenever queued notifications or geometry watches need host work.
    pub fn live<T: 'static>(
        surface: zgui::platform::SurfaceId,
        waker: Arc<dyn zgui::platform::Waker>,
        owner: Weak<T>,
        enter: fn(&T, BridgeStep<'_>) -> DispatchOutcome,
    ) -> Self {
        Self {
            wake: WakeTarget::Live { surface, waker },
            dispatch: Some(Self::make_dispatcher(owner, enter)),
        }
    }

    /// Synchronous dispatch port without an OS waker, intended for unit harnesses.
    pub fn synchronous<T: 'static>(
        owner: Weak<T>,
        enter: fn(&T, BridgeStep<'_>) -> DispatchOutcome,
    ) -> Self {
        Self {
            wake: WakeTarget::None,
            dispatch: Some(Self::make_dispatcher(owner, enter)),
        }
    }

    fn make_dispatcher<T: 'static>(
        owner: Weak<T>,
        enter: fn(&T, BridgeStep<'_>) -> DispatchOutcome,
    ) -> Rc<dyn Fn(BridgeStep<'_>) -> DispatchOutcome> {
        Rc::new(move |step| {
            let Some(owner) = owner.upgrade() else {
                return DispatchOutcome::Retired;
            };
            enter(&owner, step)
        })
    }
}

struct TurnActiveGuard<'a>(&'a UiBridge);

impl<'a> TurnActiveGuard<'a> {
    fn enter(bridge: &'a UiBridge) -> Self {
        assert!(!bridge.active.replace(true), "nested bridge entry");
        Self(bridge)
    }
}

impl Drop for TurnActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.active.set(false);
    }
}

type EventDispatcher = Rc<events::Dispatcher>;
type GeometryDispatcher = Rc<geometry::Dispatcher>;

/// Dispose and drop this before destroying its JS engine. For locals, declare
/// the engine first; for struct fields, place this bridge before the engine.
/// Installed callbacks hold only weak references, so the context cannot own its own roots.
pub struct UiBridge {
    registry: RefCell<NodeRegistry>,
    host: zgui::view::HostHandle,
    port: HostPort,
    wrappers: RefCell<HashMap<u64, Persistent<Object<'static>>>>,
    properties: RefCell<properties::Properties>,
    options: BridgeOptions,
    style_diagnostics: RefCell<Vec<StyleDiagnostic>>,
    stylesheets: RefCell<Vec<(String, String)>>,
    stylesheet_install_deferred: Cell<bool>,
    events: RefCell<events::Events>,
    event_dispatcher: EventDispatcher,
    geometry: RefCell<geometry::Geometry>,
    geometry_dispatcher: GeometryDispatcher,
    focus_traps: RefCell<focus::FocusTraps>,
    dismissals: RefCell<dismiss::Dismissals>,
    scroll_locks: RefCell<scroll::ScrollLocks>,
    canvas_scenes: RefCell<canvas::CanvasScenes>,
    pending: RefCell<VecDeque<QueuedNotification>>,
    wakes: Cell<usize>,
    overflowed: Cell<bool>,
    active: Cell<bool>,
    stopping: Cell<bool>,
    disposed: Cell<bool>,
}

impl UiBridge {
    /// Uses the host for this registry's native document and binds internal dispatchers
    /// to `port`. Editable values always go through its editing API, never reconciled text.
    pub fn new(registry: NodeRegistry, host: zgui::view::HostHandle, port: HostPort) -> Rc<Self> {
        Self::new_with_options(registry, host, port, BridgeOptions::default())
    }

    pub fn new_with_options(registry: NodeRegistry, host: zgui::view::HostHandle, port: HostPort, options: BridgeOptions) -> Rc<Self> {
        let dom = registry.dom_handle();
        let root_native = registry.native(registry.root()).expect("the session root is a live native node");
        let limit = registry.node_limit();
        let events = RefCell::new(events::Events::new(dom.clone()));
        let geometry = RefCell::new(geometry::Geometry::new(dom.clone(), root_native, host.clone(), limit.saturating_mul(4)));
        let focus_traps = RefCell::new(focus::FocusTraps::new(limit));
        let dismissals = RefCell::new(dismiss::Dismissals::new(dom.clone(), root_native, limit));
        let canvas_scenes = RefCell::new(canvas::CanvasScenes::new(dom.clone()));
        let scroll_locks = RefCell::new(scroll::ScrollLocks::new(dom, root_native, host.clone(), limit));
        Rc::new_cyclic(|weak: &Weak<Self>| {
            let event_weak = weak.clone();
            let event_dispatcher: EventDispatcher = Rc::new(move |id, event| {
                if let Some(bridge) = event_weak.upgrade() {
                    bridge.dispatch_event(id, event);
                }
            });
            let geometry_weak = weak.clone();
            let geometry_dispatcher: GeometryDispatcher = Rc::new(move |id, value, scale| {
                if let Some(bridge) = geometry_weak.upgrade() {
                    bridge.dispatch_geometry(id, value, scale);
                }
            });
            Self {
                registry: RefCell::new(registry),
                host,
                port,
                wrappers: RefCell::new(HashMap::new()),
                properties: RefCell::new(properties::Properties::default()),
                options,
                style_diagnostics: RefCell::new(Vec::new()),
                stylesheets: RefCell::new(Vec::new()),
                stylesheet_install_deferred: Cell::new(options.defer_stylesheet_install),
                events,
                event_dispatcher,
                geometry,
                geometry_dispatcher,
                focus_traps,
                dismissals,
                scroll_locks,
                canvas_scenes,
                pending: RefCell::new(VecDeque::new()),
                wakes: Cell::new(0),
                overflowed: Cell::new(false),
                active: Cell::new(false),
                stopping: Cell::new(false),
                disposed: Cell::new(false),
            }
        })
    }

    /// Drains styling diagnostics for the host's structured diagnostic stream.
    pub fn take_style_diagnostics(&self) -> Vec<StyleDiagnostic> {
        std::mem::take(&mut *self.style_diagnostics.borrow_mut())
    }

    /// Returns mount-generated stylesheet candidates for validation outside the JS entry.
    pub fn stylesheet_candidates(&self) -> Vec<(String, String)> {
        self.stylesheets.borrow().iter().map(|(name, css)| (name.clone(), css.clone())).collect()
    }

    /// Installs all admitted mount-generated sheets and enables ordinary queued updates.
    pub fn arm_stylesheets(&self) {
        if !self.stylesheet_install_deferred.get() { return; }
        for (name, css) in self.stylesheet_candidates() {
            self.host.install_stylesheet(&name, &css);
        }
        self.stylesheet_install_deferred.set(false);
    }

    fn style_result<T>(&self, ctx: &Ctx<'_>, result: Result<T, StyleDiagnostic>) -> rquickjs::Result<Option<T>> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(diagnostic) => {
                let fail = self.options.strict_styles;
                let message = fail.then(|| format!("{}: {}", diagnostic.code, diagnostic.message));
                self.style_diagnostics.borrow_mut().push(diagnostic);
                if let Some(message) = message { Err(Exception::throw_type(ctx, &message)) } else { Ok(None) }
            }
        }
    }

    /// Runs `action` inside an outermost structural turn and passes a [`BridgeTurn`] token.
    /// Finalizes detachments on `Ok`; faults and fully disposes the bridge on `Err`.
    pub fn run_turn<J, T, E>(
        &self,
        mut js: J,
        action: impl for<'turn> FnOnce(&BridgeTurn<'turn>, &mut J) -> Result<T, E>,
    ) -> Result<T, E> {
        let turn = BridgeTurn { bridge: self, _marker: PhantomData };
        self.execute_turn(|| action(&turn, &mut js))
    }

    /// Pops and runs one queued notification inside an active structural turn.
    /// Returns `Ok(false)` when the queue is empty or disposed.
    pub fn flush_notification<J, E>(
        &self,
        mut js: J,
        enter: impl FnOnce(&mut J, BridgeCallback<'_>) -> Result<(), E>,
    ) -> Result<bool, E> {
        let item = self.pending.borrow_mut().pop_front();
        let Some(item) = item else {
            return Ok(false);
        };
        match item {
            QueuedNotification::Event(event) => {
                let payload = DispatchPayload::QueuedEvent(event);
                self.execute_turn(|| enter(&mut js, BridgeCallback { bridge: self, payload }))?;
            }
            QueuedNotification::Geometry { id, value, scale } => {
                let mut slot = Some(value);
                let payload = DispatchPayload::Geometry { id, slot: &mut slot, scale };
                self.execute_turn(|| enter(&mut js, BridgeCallback { bridge: self, payload }))?;
            }
        }
        Ok(true)
    }

    /// Reports whether queued notifications remain, or `Err(QueueOverflow)` if overflow retired the bridge.
    pub fn has_queued_notifications(&self) -> Result<bool, QueueOverflow> {
        if self.overflowed.get() {
            return Err(QueueOverflow);
        }
        Ok(!self.pending.borrow().is_empty())
    }

    /// Returns and clears the count of wake signals emitted since the last call.
    pub fn take_wakes(&self) -> usize {
        self.wakes.replace(0)
    }

    fn execute_turn<T, E>(&self, action: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        let guard = TurnActiveGuard::enter(self);
        self.registry.borrow_mut().begin_turn();
        let result = action();
        drop(guard);
        if result.is_err() || self.disposed.get() {
            self.dispose();
            return result;
        }
        let invalid = self.registry.borrow_mut().finish_turn();
        let (dom, targets) = {
            let registry = self.registry.borrow();
            let targets = self.properties.borrow().dirty_tokens().into_iter()
                .filter_map(|token| registry.native_token(token).ok().map(|native| (token, native))).collect::<Vec<_>>();
            (registry.dom_handle(), targets)
        };
        let diagnostics = self.properties.borrow_mut().finish_turn(&dom, targets);
        self.style_diagnostics.borrow_mut().extend(diagnostics);
        for callback in self.dismissals.borrow_mut().remove_nodes(&invalid) {
            self.events.borrow_mut().remove_callback(callback);
        }
        self.geometry.borrow_mut().remove_nodes(&invalid);
        self.scroll_locks.borrow_mut().remove_nodes(&invalid);
        self.canvas_scenes.borrow_mut().remove_nodes(&invalid);
        let traps = self.focus_traps.borrow_mut().remove_nodes(&invalid);
        for trap in traps { drop(trap); }
        let mut wrappers = self.wrappers.borrow_mut();
        for handle in invalid {
            wrappers.remove(&handle.token());
            self.properties.borrow_mut().remove(handle.token());
            self.events.borrow_mut().remove(handle.token());
        }
        result
    }

    pub fn live_count(&self) -> usize { self.registry.borrow().live_count() }
    pub fn wrapper_count(&self) -> usize { self.wrappers.borrow().len() }
    pub fn is_disposed(&self) -> bool { self.disposed.get() }
    pub fn callback_count(&self) -> usize { self.events.borrow().len() + self.geometry.borrow().len() }
    pub fn focus_trap_count(&self) -> usize { self.focus_traps.borrow().len() }
    pub fn scroll_lock_count(&self) -> usize { self.scroll_locks.borrow().len() }
    /// Returns the number of retained canvas scenes still owned by this session.
    pub fn canvas_scene_count(&self) -> usize { self.canvas_scenes.borrow().len() }

    /// Copies the attached live tree without entering JavaScript. The returned
    /// snapshot owns its records and holds no registry borrow during subsequent
    /// native queries. See [`NodeRegistry::snapshot`] for ordering and limits.
    pub fn snapshot(&self, max_nodes: usize) -> Snapshot {
        self.registry.borrow().snapshot(max_nodes)
    }

    /// Diagnostic changes need a fresh validation frame even without native DOM changes.
    pub fn metadata_revision(&self) -> u64 { self.registry.borrow().metadata_revision() }

    fn wake(&self) {
        self.wakes.set(self.wakes.get().saturating_add(1));
        if let WakeTarget::Live { surface, waker } = &self.port.wake {
            waker.wake(zgui::platform::WakeReason::ReactiveWork {
                surfaces: vec![*surface].into_boxed_slice(),
            });
        }
    }

    fn can_dispatch_synchronously(&self) -> bool {
        self.port.dispatch.is_some()
            && !self.active.get()
            && !self.disposed.get()
            && !self.overflowed.get()
            && self.pending.borrow().is_empty()
    }

    fn enqueue(&self, item: QueuedNotification) {
        if self.disposed.get() {
            return;
        }
        if self.pending.borrow().len() >= MAX_QUEUED_NOTIFICATIONS {
            self.overflowed.set(true);
            self.dispose();
            return;
        }
        self.pending.borrow_mut().push_back(item);
        self.wake();
    }

    fn try_dispatch_synchronously<'a>(&'a self, payload: DispatchPayload<'a>) -> bool {
        let Some(dispatch) = self.port.dispatch.as_ref().filter(|_| self.can_dispatch_synchronously()) else {
            return false;
        };
        match dispatch(BridgeStep { bridge: self, payload }) {
            DispatchOutcome::Executed | DispatchOutcome::Retired => true,
            DispatchOutcome::Faulted => {
                self.dispose();
                true
            }
            DispatchOutcome::Busy => false,
        }
    }

    fn dispatch_event(&self, id: u64, event: &mut zgui::view::EventCx<'_>) {
        if self.disposed.get() {
            return;
        }
        let mut snapshot_slot = Some(events::EventSnapshot::capture(event));
        let mut native_slot = Some(event);
        let mut apply = |control: events::DispatchControl| {
            if let Some(native) = native_slot.take() {
                control.expire_and_apply(native);
            }
        };
        let payload = DispatchPayload::LiveEvent {
            id,
            snapshot: &mut snapshot_slot,
            apply: &mut apply,
        };
        if !self.try_dispatch_synchronously(payload)
            && let Some(snapshot) = snapshot_slot.take()
        {
            self.enqueue(QueuedNotification::Event(DeferredEvent { id, snapshot }));
        }
    }

    fn dispatch_geometry(&self, id: u64, value: zgui::view::ObservedValue, scale: f32) {
        if self.disposed.get() {
            return;
        }
        let mut value_slot = Some(value);
        let payload = DispatchPayload::Geometry { id, slot: &mut value_slot, scale };
        if !self.try_dispatch_synchronously(payload)
            && let Some(value) = value_slot.take()
        {
            self.enqueue(QueuedNotification::Geometry { id, value, scale });
        }
    }

    fn invoke_geometry_inner<'js>(&self, ctx: &Ctx<'js>, id: u64, value: zgui::view::ObservedValue, scale: f32) -> rquickjs::Result<()> {
        self.require_turn(ctx)?;
        let prepared = self.geometry.borrow_mut().prepare(ctx, id, value, scale)?;
        let Some((callback, measurement)) = prepared else { return Ok(()); };
        let object = measurement.into_js(ctx)?;
        callback.restore(ctx)?.call::<_, Value>((object,)).map(|_| ())
    }

    fn invoke_snapshot<'js>(
        &self,
        ctx: &Ctx<'js>,
        id: u64,
        snapshot: events::EventSnapshot,
        apply: Option<&mut dyn FnMut(events::DispatchControl)>,
    ) -> rquickjs::Result<()> {
        self.require_turn(ctx)?;
        let (callback, dismiss) = {
            let events = self.events.borrow();
            (events.callback(id), events.is_dismiss(id))
        };
        let Some(callback) = callback else { return Ok(()); };
        if dismiss {
            if let Some(reason) = snapshot.dismiss_reason() {
                callback.restore(ctx)?.call::<_, Value>((reason.name(),))?;
            }
            return Ok(());
        }
        let (target, current) = {
            let registry = self.registry.borrow();
            (registry.event_handle(snapshot.target), registry.event_handle(snapshot.current_target))
        };
        let (Some(target), Some(current)) = (target, current) else { return Ok(()); };
        let target = self.wrap(ctx, target)?;
        let current = self.wrap(ctx, current)?;
        let (object, control) = snapshot.to_js(ctx, &target, &current)?;
        let callback = callback.restore(ctx)?;
        if let Some(apply) = apply {
            let result = callback.call::<_, Value>((object,));
            apply(control);
            result.map(|_| ())
        } else {
            drop(control);
            callback.call::<_, Value>((object,)).map(|_| ())
        }
    }

    /// Retiring owners may remove resources, but cannot allocate or register new work.
    pub fn begin_stop(&self) { self.stopping.set(true); }

    pub fn dispose(&self) {
        let stylesheets: Vec<String> = self.stylesheets.borrow_mut().drain(..).map(|(name, _)| name).collect();
        for name in stylesheets { self.host.remove_stylesheet(&name); }
        for callback in self.dismissals.borrow_mut().clear() {
            self.events.borrow_mut().remove_callback(callback);
        }
        self.events.borrow_mut().clear();
        self.geometry.borrow_mut().clear();
        self.scroll_locks.borrow_mut().clear();
        self.pending.borrow_mut().clear();
        let traps = self.focus_traps.borrow_mut().take_all();
        for trap in traps { drop(trap); }
        if !self.disposed.replace(true) { self.registry.borrow_mut().dispose(); }
        self.canvas_scenes.borrow_mut().clear();
        self.wrappers.borrow_mut().clear();
        self.properties.borrow_mut().clear();
    }

    fn require_turn(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        if self.disposed.get() { return Err(Exception::throw_reference(ctx, "native session is disposed")); }
        if !self.active.get() { return Err(Exception::throw_type(ctx, "native mutation outside a structural turn")); }
        Ok(())
    }

    fn require_creation(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        self.require_turn(ctx)?;
        if self.stopping.get() { return Err(Exception::throw_reference(ctx, "native session is stopping")); }
        Ok(())
    }

    fn wrap<'js>(&self, ctx: &Ctx<'js>, handle: Handle) -> rquickjs::Result<Object<'js>> {
        self.registry.borrow().native(handle).map_err(|error| host_error(ctx, error))?;
        let existing = self.wrappers.borrow().get(&handle.token()).cloned();
        if let Some(wrapper) = existing { return wrapper.restore(ctx); }
        let object = Class::instance(ctx.clone(), NativeNode { handle })?.into_inner();
        self.wrappers.borrow_mut().insert(handle.token(), Persistent::save(ctx, object.clone()));
        Ok(object)
    }

    pub fn root<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        let handle = self.registry.borrow().root();
        self.wrap(ctx, handle)
    }

    /// Install the private host contract before evaluating the bundled frontend.
    /// No constructor is exported for NativeNode, and cloned/fabricated objects
    /// fail Class conversion before any native operation is reached.
    pub fn install<'js>(self: &Rc<Self>, ctx: &Ctx<'js>) -> rquickjs::Result<()> {
        let api = Object::new(ctx.clone())?;
        canvas::install(self, &api)?;
        let weak = Rc::downgrade(self);
        api.set("setProperty", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>, name: String, value: Value<'js>, _previous: Opt<Value<'js>>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let handle = node.borrow().handle;
            if name == zgui::vocab::prop::drawing::CANVAS {
                return canvas::bind(&bridge, &ctx, handle, value);
            }
            if name.starts_with("on") {
                if !value.is_null() && !value.is_undefined() { bridge.require_creation(&ctx)?; }
                let native = bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
                return bridge.events.borrow_mut().set(&ctx, handle, native, &name, value, bridge.event_dispatcher.clone());
            }
            if let Some(relation) = properties::Relation::parse(&name) {
                if matches!(relation, properties::Relation::PopupFor) && value.is_array() {
                    return Err(Exception::throw_type(&ctx, "ariaPopupFor requires a native node, null, or undefined"));
                }
                let limit = bridge.registry.borrow().node_limit();
                let handles = relation_handles(&ctx, value, limit)?;
                let (dom, native, mutation) = {
                    let registry = bridge.registry.borrow();
                    let mut targets = Vec::with_capacity(handles.len());
                    for target in handles {
                        let native = registry.native(target).map_err(|e| host_error(&ctx, e))?;
                        targets.push(zgui::vocab::NodeId(native.as_u64()));
                    }
                    (registry.dom_handle(), registry.native(handle).map_err(|e| host_error(&ctx, e))?,
                        properties::Mutation::Relation(relation, targets))
                };
                let result = bridge.properties.borrow_mut().apply(&dom, native, handle.token(), mutation);
                bridge.style_result(&ctx, result)?;
                return Ok(());
            }
            let mutation = match properties::Mutation::parse(&ctx, &name, value) {
                Ok(mutation) => mutation,
                Err(error) if name == "style" || name == "__zguiStyle" || name == "__zguiInvariantStyle" || name.starts_with("--") => {
                    if !error.is_exception() { return Err(error); }
                    let exception = ctx.catch();
                    let message = exception.as_object().and_then(|object| object.get::<_, String>("message").ok()).unwrap_or_else(|| "invalid style value".into());
                    if !message.starts_with("STYLE_") {
                        return Err(ctx.throw(exception));
                    }
                    bridge.style_result::<()>(&ctx, Err(StyleDiagnostic { code: "STYLE_VALUE_INVALID", message }))?;
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            if matches!(&mutation, properties::Mutation::Attribute("src", Some(_))) {
                bridge.require_creation(&ctx)?;
            }
            if let properties::Mutation::TestId(value) = mutation {
                return bridge.registry.borrow_mut().set_test_id(handle, value).map_err(|error| host_error(&ctx, error));
            }
            let (dom, native) = {
                let registry = bridge.registry.borrow();
                if mutation.requires_editable() {
                    registry.require_editable(handle).map_err(|e| host_error(&ctx, e))?;
                }
                if matches!(mutation, properties::Mutation::Attribute("src", _)) {
                    registry.require_image(handle).map_err(|e| host_error(&ctx, e))?;
                }
                if matches!(mutation, properties::Mutation::Attribute("secure", _)) {
                    registry.require_field(handle).map_err(|e| host_error(&ctx, e))?;
                }
                if matches!(mutation, properties::Mutation::Drawing(_, _)) {
                    registry.require_vector(handle).map_err(|e| host_error(&ctx, e))?;
                }
                (registry.dom_handle(), registry.native(handle).map_err(|e| host_error(&ctx, e))?)
            };
            let result = bridge.properties.borrow_mut().apply(&dom, native, handle.token(), mutation);
            let value = bridge.style_result(&ctx, result)?.flatten();
            // Native editing can notify listeners; release all registry/property
            // borrows before entering the host.
            if let Some(value) = value { bridge.host.set_value(native, &value); }
            Ok::<(), rquickjs::Error>(())
        }))?;
        let reduced_motion = self.options.reduced_motion;
        api.set("getReducedMotion", Func::from(move || reduced_motion))?;
        let weak = Rc::downgrade(self);
        api.set("createElement", Func::from(move |ctx: Ctx<'js>, tag: String| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            let handle = bridge.registry.borrow_mut().create_element(&tag).map_err(|e| host_error(&ctx, e))?;
            bridge.wrap(&ctx, handle)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("createPortal", Func::from(move |ctx: Ctx<'js>, layer: String| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            let layer = overlay_layer(&ctx, &layer)?;
            let handle = bridge.registry.borrow_mut().create_portal(layer).map_err(|e| host_error(&ctx, e))?;
            bridge.wrap(&ctx, handle)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("createTextNode", Func::from(move |ctx: Ctx<'js>, text: String| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            let handle = bridge.registry.borrow_mut().create_text(&text).map_err(|e| host_error(&ctx, e))?;
            bridge.wrap(&ctx, handle)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("replaceText", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>, text: String| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let handle = node.borrow().handle;
            let result = bridge.registry.borrow_mut().replace_text(handle, &text);
            result.map_err(|e| host_error(&ctx, e))
        }))?;
        let weak = Rc::downgrade(self);
        api.set("isTextNode", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>| {
            let bridge = owner(&weak, &ctx)?;
            let handle = node.borrow().handle;
            let result = bridge.registry.borrow().is_text(handle);
            result.map_err(|e| host_error(&ctx, e))
        }))?;
        let weak = Rc::downgrade(self);
        api.set("insertNode", Func::from(move |ctx: Ctx<'js>, parent: Class<'js, NativeNode>, node: Class<'js, NativeNode>, anchor: Option<Class<'js, NativeNode>>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            let parent = parent.borrow().handle;
            let node = node.borrow().handle;
            let anchor = anchor.map(|node| node.borrow().handle);
            let result = bridge.registry.borrow_mut().insert(parent, node, anchor);
            result.map_err(|e| host_error(&ctx, e))
        }))?;
        let weak = Rc::downgrade(self);
        api.set("removeNode", Func::from(move |ctx: Ctx<'js>, parent: Class<'js, NativeNode>, node: Class<'js, NativeNode>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let parent = parent.borrow().handle;
            let node = node.borrow().handle;
            let result = bridge.registry.borrow_mut().remove(parent, node);
            result.map_err(|e| host_error(&ctx, e))
        }))?;
        let weak = Rc::downgrade(self);
        api.set("installStylesheet", Func::from(move |ctx: Ctx<'js>, name: String, css: String| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            let name = stylesheet_name(&ctx, &bridge.registry.borrow().stylesheet_prefix(), &name)?;
            {
                let mut stylesheets = bridge.stylesheets.borrow_mut();
                if let Some((_, installed)) = stylesheets.iter_mut().find(|(candidate, _)| candidate == &name) {
                    *installed = css.clone();
                } else {
                    stylesheets.push((name.clone(), css.clone()));
                }
            }
            if !bridge.stylesheet_install_deferred.get() {
                bridge.host.install_stylesheet(&name, &css);
            }
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("removeStylesheet", Func::from(move |ctx: Ctx<'js>, name: String| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let name = stylesheet_name(&ctx, &bridge.registry.borrow().stylesheet_prefix(), &name)?;
            let index = { bridge.stylesheets.borrow().iter().position(|(candidate, _)| candidate == &name) };
            if let Some(index) = index {
                bridge.stylesheets.borrow_mut().remove(index);
                bridge.host.remove_stylesheet(&name);
            }
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("focus", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>, reveal: Opt<bool>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let handle = node.borrow().handle;
            let native = bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
            bridge.host.focus(native);
            if reveal.0.unwrap_or(false) {
                bridge.host.scroll_to(native, zgui::view::ScrollTarget::IntoView, zgui::view::ScrollBehavior::Instant);
            }
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("trapFocus", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>, wrap: bool, auto_focus: bool, restore: bool| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            let handle = node.borrow().handle;
            let native = bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
            let id = bridge.focus_traps.borrow_mut().reserve(&ctx)?;
            let cancel = {
                let weak = Rc::downgrade(&bridge);
                rquickjs::Function::new(ctx.clone(), move || {
                    if let Some(bridge) = weak.upgrade() {
                        let trap = bridge.focus_traps.borrow_mut().take(id);
                        drop(trap);
                    }
                })?
            };
            let mut options = zgui::view::FocusTrapOptions::default();
            options.wrap = wrap;
            options.auto_focus = auto_focus;
            options.restore = restore;
            let native_id = bridge.host.push_focus_trap(native, options);
            let trap = zgui::view::FocusTrap::new(bridge.host.clone(), native, native_id);
            bridge.focus_traps.borrow_mut().add(id, handle, trap);
            Ok::<rquickjs::Function<'js>, rquickjs::Error>(cancel)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("lockScroll", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            let handle = node.borrow().handle;
            bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
            let id = bridge.scroll_locks.borrow_mut().reserve(&ctx)?;
            let cancel = {
                let weak = Rc::downgrade(&bridge);
                rquickjs::Function::new(ctx.clone(), move || {
                    if let Some(bridge) = weak.upgrade() { bridge.scroll_locks.borrow_mut().take(id); }
                })?
            };
            bridge.scroll_locks.borrow_mut().add(id, handle);
            Ok::<rquickjs::Function<'js>, rquickjs::Error>(cancel)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("registerDismissLayer", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>, callback: rquickjs::Function<'js>, options: Object<'js>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            let handle = node.borrow().handle;
            let options = dismiss_options(&ctx, &bridge, options)?;
            let native = bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
            let id = bridge.dismissals.borrow_mut().reserve(&ctx)?;
            let callback_id = bridge.events.borrow_mut().register_dismiss(&ctx, callback)?;
            bridge.dismissals.borrow_mut().add(id, handle, native, options, callback_id, bridge.event_dispatcher.clone());
            let update = {
                let weak = Rc::downgrade(&bridge);
                Func::from(move |ctx: Ctx<'js>, options: Object<'js>| {
                    let bridge = owner(&weak, &ctx)?;
                    bridge.require_creation(&ctx)?;
                    let options = dismiss_options(&ctx, &bridge, options)?;
                    bridge.dismissals.borrow_mut().update(&ctx, id, options)
                })
            };
            let dispose = {
                let weak = Rc::downgrade(&bridge);
                Func::from(move || {
                    if let Some(bridge) = weak.upgrade()
                        && let Some(callback) = bridge.dismissals.borrow_mut().take(id)
                    {
                        bridge.events.borrow_mut().remove_callback(callback);
                    }
                    Ok::<(), rquickjs::Error>(())
                })
            };
            let lease = Object::new(ctx.clone())?;
            lease.set("update", update)?;
            lease.set("dispose", dispose)?;
            Ok::<Object<'js>, rquickjs::Error>(lease)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("getWindowRect", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>| -> rquickjs::Result<Value<'js>> {
            let bridge = owner(&weak, &ctx)?;
            let handle = node.borrow().handle;
            let native = bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
            let Some(bounds) = bridge.host.window_box(native) else {
                return Ok(Value::new_null(ctx.clone()));
            };
            let scale = f64::from(bridge.host.scale());
            if !scale.is_finite() || scale <= 0.0 {
                return Err(Exception::throw_range(&ctx, "native window scale must be finite and positive"));
            }
            let values = [
                ("x", f64::from(bounds.origin.x.0) / scale),
                ("y", f64::from(bounds.origin.y.0) / scale),
                ("width", f64::from(bounds.size.width.0) / scale),
                ("height", f64::from(bounds.size.height.0) / scale),
            ];
            if values.iter().any(|(_, value)| !value.is_finite()) {
                return Err(Exception::throw_range(&ctx, "native window bounds must be finite"));
            }
            let object = Object::new(ctx.clone())?;
            for (name, value) in values { object.set(name, value)?; }
            Ok(object.into_value())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("getTextContent", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>| {
            let bridge = owner(&weak, &ctx)?;
            let handle = node.borrow().handle;
            let (dom, native) = {
                let registry = bridge.registry.borrow();
                let native = registry.native(handle).map_err(|e| host_error(&ctx, e))?;
                (registry.dom_handle(), native)
            };
            Ok::<String, rquickjs::Error>(dom.text_content(native))
        }))?;
        let weak = Rc::downgrade(self);
        api.set("isAnimating", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>| {
            let bridge = owner(&weak, &ctx)?;
            let handle = node.borrow().handle;
            let native = bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
            Ok::<bool, rquickjs::Error>(bridge.host.running_animations(native) != 0)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("precedes", Func::from(move |ctx: Ctx<'js>, first: Class<'js, NativeNode>, second: Class<'js, NativeNode>| {
            let bridge = owner(&weak, &ctx)?;
            let first = bridge.registry.borrow().native(first.borrow().handle).map_err(|e| host_error(&ctx, e))?;
            let second = bridge.registry.borrow().native(second.borrow().handle).map_err(|e| host_error(&ctx, e))?;
            Ok::<bool, rquickjs::Error>(bridge.host.precedes(first, second))
        }))?;
        let weak = Rc::downgrade(self);
        api.set("clearRoot", Func::from(move |ctx: Ctx<'js>, root: Class<'js, NativeNode>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let root = root.borrow().handle;
            let mut registry = bridge.registry.borrow_mut();
            if root != registry.root() { return Err(Exception::throw_type(&ctx, "clearRoot requires the mount root")); }
            while let Some(child) = registry.first_child(root).map_err(|e| host_error(&ctx, e))? {
                registry.remove(root, child).map_err(|e| host_error(&ctx, e))?;
            }
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("observeGeometry", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>, name: String, callback: rquickjs::Function<'js>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            let handle = node.borrow().handle;
            let native = bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
            let what = geometry::Geometry::kind(&ctx, &name)?;
            let id = bridge.geometry.borrow_mut().reserve(&ctx)?;
            let cancel = {
                let weak = Rc::downgrade(&bridge);
                rquickjs::Function::new(ctx.clone(), move || {
                    if let Some(bridge) = weak.upgrade() { bridge.geometry.borrow_mut().cancel(id); }
                })?
            };
            bridge.geometry.borrow_mut().add(&ctx, id, handle, native, what, callback, bridge.geometry_dispatcher.clone());
            bridge.wake();
            Ok::<rquickjs::Function<'js>, rquickjs::Error>(cancel)
        }))?;
        for (name, query) in [
            ("getParentNode", NodeRegistry::parent as fn(&NodeRegistry, Handle) -> Result<Option<Handle>, BridgeError>),
            ("getFirstChild", NodeRegistry::first_child),
            ("getNextSibling", NodeRegistry::next_sibling),
        ] {
            let weak = Rc::downgrade(self);
            api.set(name, Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>| {
                let bridge = owner(&weak, &ctx)?;
                let handle = node.borrow().handle;
                let result = query(&bridge.registry.borrow(), handle).map_err(|e| host_error(&ctx, e))?;
                result.map(|handle| bridge.wrap(&ctx, handle)).transpose()
            }))?;
        }
        ctx.globals().set("__zgui", api)
    }
}

impl Drop for UiBridge {
    fn drop(&mut self) { self.dispose(); }
}

fn stylesheet_name(ctx: &Ctx<'_>, prefix: &str, key: &str) -> rquickjs::Result<String> {
    let valid = !key.is_empty() && key.len() <= 128 && key.bytes().all(|byte|
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'));
    if !valid { return Err(Exception::throw_type(ctx, "stylesheet names use 1-128 ASCII bytes from [a-zA-Z0-9_.:-]")); }
    Ok(format!("{prefix}{key}"))
}

// Read a bounded unsigned JS array length directly instead of rquickjs::Array::len/iter,
// whose signed-integer assumption overflows or rejects valid arrays near u32 bounds.
fn bounded_array_len<'js>(ctx: &Ctx<'js>, array: &rquickjs::Array<'js>, limit: usize, error_msg: &str) -> rquickjs::Result<usize> {
    let length = array.as_object().get::<_, u32>("length")? as usize;
    if length > limit {
        return Err(Exception::throw_range(ctx, error_msg));
    }
    Ok(length)
}

fn relation_handles<'js>(ctx: &Ctx<'js>, value: Value<'js>, limit: usize) -> rquickjs::Result<Vec<Handle>> {
    let node = |ctx: &Ctx<'js>, value: Value<'js>| -> rquickjs::Result<Handle> {
        value.as_object()
            .and_then(|object| object.as_class::<NativeNode>())
            .map(|class| class.borrow().handle)
            .ok_or_else(|| Exception::throw_type(ctx, "accessibility relations accept only native nodes or arrays of them"))
    };
    if value.is_null() || value.is_undefined() { return Ok(Vec::new()); }
    if value.is_array() {
        let array = value.as_array().expect("checked array");
        let length = bounded_array_len(ctx, array, limit, "accessibility relations exceed the session node limit")?;
        let mut handles = Vec::with_capacity(length);
        for index in 0..length { handles.push(node(ctx, array.get::<Value>(index)?)?); }
        return Ok(handles);
    }
    Ok(vec![node(ctx, value)?])
}

fn owner(weak: &Weak<UiBridge>, ctx: &Ctx<'_>) -> rquickjs::Result<Rc<UiBridge>> {
    weak.upgrade().ok_or_else(|| Exception::throw_reference(ctx, "native session no longer exists"))
}
fn dismiss_options<'js>(ctx: &Ctx<'js>, bridge: &UiBridge, value: Object<'js>) -> rquickjs::Result<dismiss::Options> {
    let layer: String = value.get("layer")?;
    let outside: bool = value.get("dismissOnOutsidePress")?;
    let escape: bool = value.get("dismissOnEscape")?;
    let exclude: Option<Class<'js, NativeNode>> = value.get("exclude")?;
    let leaving: bool = value.get("leaving")?;
    bridge.require_creation(ctx)?;
    let band = overlay_layer(ctx, &layer)?;
    let exclude = exclude.map(|node| {
        let handle = node.borrow().handle;
        bridge.registry.borrow().native(handle).map(|native| (handle, native)).map_err(|e| host_error(ctx, e))
    }).transpose()?;
    Ok(dismiss::Options { band, outside, escape, exclude, leaving })
}

fn overlay_layer(ctx: &Ctx<'_>, layer: &str) -> rquickjs::Result<zgui::view::OverlayLayer> {
    match layer {
        "content" => Ok(zgui::view::OverlayLayer::Content),
        "popover" => Ok(zgui::view::OverlayLayer::Popover),
        "modal" => Ok(zgui::view::OverlayLayer::Modal),
        "toast" => Ok(zgui::view::OverlayLayer::Toast),
        _ => Err(Exception::throw_type(ctx, "unsupported native overlay layer")),
    }
}

fn host_error(ctx: &Ctx<'_>, error: BridgeError) -> rquickjs::Error {
    Exception::throw_type(ctx, &error.to_string())
}
