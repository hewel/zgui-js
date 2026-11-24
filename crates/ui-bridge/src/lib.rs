//! Owner-thread native tree bridge. JavaScript sees branded objects, never native IDs.
//! A structural turn is one outermost JavaScript entry; pending removals expire at its end.
#![forbid(unsafe_code)]

pub mod dnd;
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

/// Host-selected failure policy, motion preference, and capability state for one session.
#[derive(Clone, Copy, Debug, Default)]
pub struct BridgeOptions {
    /// Preview rejects invalid style maps; live sessions retain their last valid map.
    pub strict_styles: bool,
    /// Suppresses shared SDK motion durations through ThemeProvider.
    pub reduced_motion: bool,
    /// Live compositor background blur capability (`ext_background_effect_v1` `Capability::Blur`).
    pub background_blur_supported: bool,
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

struct GestureGuard<'a> {
    bridge: &'a UiBridge,
    group: Option<Rc<BridgeGroup>>,
}

impl Drop for GestureGuard<'_> {
    fn drop(&mut self) {
        self.bridge.user_gesture_depth.set(self.bridge.user_gesture_depth.get().saturating_sub(1));
        if let Some(group) = &self.group {
            group.user_gesture_depth.set(group.user_gesture_depth.get().saturating_sub(1));
        }
    }
}

/// The structural-turn boundary shared by every mount region in one JS engine.
///
/// The group owns its member bridges. Dispose and drop it before destroying the
/// engine, just as for a standalone [`UiBridge`].
#[derive(Default)]
pub struct BridgeGroup {
    bridges: RefCell<Vec<Rc<UiBridge>>>,
    active: Cell<bool>,
    user_gesture_depth: Cell<u32>,
}

impl fmt::Debug for BridgeGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeGroup").finish_non_exhaustive()
    }
}

impl BridgeGroup {
    /// Creates an empty turn boundary, usable before the first mount region opens.
    pub fn new() -> Self { Self::default() }

    /// Whether a user-initiated input event handler is currently executing in this group.
    pub fn in_user_gesture(&self) -> bool { self.user_gesture_depth.get() > 0 }

    /// Joins a live standalone bridge to this group's turns.
    ///
    /// # Panics
    ///
    /// Membership may change only between turns. The bridge must not already
    /// belong to a live group, be in a standalone turn, or have been disposed.
    pub fn add(self: &Rc<Self>, bridge: &Rc<UiBridge>) {
        let mut bridges = self.bridges.borrow_mut();
        assert!(!bridge.active.get() && !bridge.disposed.get(), "bridge must be live and idle");
        assert!(bridge.group.borrow().upgrade().is_none(), "bridge already belongs to a group");
        bridges.push(bridge.clone());
        *bridge.group.borrow_mut() = Rc::downgrade(self);
    }

    /// Removes a member without disposing it. Only call between turns.
    ///
    /// # Panics
    ///
    /// Panics if this group is running a turn.
    pub fn remove(&self, bridge: &UiBridge) {
        let mut bridges = self.bridges.borrow_mut();
        if let Some(index) = bridges.iter().position(|member| std::ptr::eq(member.as_ref(), bridge)) {
            *bridge.group.borrow_mut() = Weak::new();
            bridges.remove(index);
        }
    }

    /// Enters one JS turn with every member region active, even when the group is empty.
    ///
    /// # Errors
    ///
    /// Propagates `action`'s error after disposing every member bridge. Successful
    /// turns finalize pending detachments in all member regions.
    ///
    /// # Panics
    ///
    /// Panics on nested turn entry.
    pub fn run<J, T, E>(&self, mut js: J, action: impl FnOnce(&mut J) -> Result<T, E>) -> Result<T, E> {
        self.execute_turn(|| action(&mut js))
    }

    /// Prevents new tree resources from being allocated in any current member.
    pub fn begin_stop(&self) {
        for bridge in &*self.bridges.borrow() { bridge.begin_stop(); }
    }

    /// Revokes all current regions and releases their persistent JavaScript roots.
    pub fn dispose(&self) {
        for bridge in &*self.bridges.borrow() { bridge.dispose(); }
    }

    fn execute_turn<T, E>(&self, action: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        let bridges = self.bridges.borrow();
        assert!(!self.active.replace(true), "nested bridge group entry");
        let mut guard = GroupActiveGuard { active: &self.active, entered: &[] };
        for (index, bridge) in bridges.iter().enumerate() {
            assert!(!bridge.active.replace(true), "nested bridge entry");
            guard.entered = &bridges[..=index];
            if !bridge.disposed.get() {
                bridge.registry.borrow_mut().begin_turn();
            }
        }
        let result = action();
        if result.is_err() || bridges.iter().any(|bridge| bridge.overflowed.get()) {
            self.dispose();
        } else {
            for bridge in bridges.iter().filter(|bridge| !bridge.disposed.get()) {
                bridge.finish_turn();
            }
        }
        result
    }
}

impl Drop for BridgeGroup {
    fn drop(&mut self) { self.dispose(); }
}

struct GroupActiveGuard<'a> {
    active: &'a Cell<bool>,
    entered: &'a [Rc<UiBridge>],
}

impl Drop for GroupActiveGuard<'_> {
    fn drop(&mut self) {
        for bridge in self.entered { bridge.active.set(false); }
        self.active.set(false);
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
    reduced_motion: Cell<bool>,
    reduced_motion_subscribers: RefCell<HashMap<u64, Persistent<rquickjs::Function<'static>>>>,
    next_reduced_motion_id: Cell<u64>,
    background_blur_supported: Cell<bool>,
    background_blur_subscribers: RefCell<HashMap<u64, Persistent<rquickjs::Function<'static>>>>,
    next_background_blur_id: Cell<u64>,
    style_diagnostics: RefCell<Vec<StyleDiagnostic>>,
    stylesheets: RefCell<Vec<(String, String)>>,
    stylesheet_install_deferred: Cell<bool>,
    events: RefCell<events::Events>,
    dnd: RefCell<dnd::DndListeners>,
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
    group: RefCell<Weak<BridgeGroup>>,
    active: Cell<bool>,
    user_gesture_depth: Cell<u32>,
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
                reduced_motion: Cell::new(options.reduced_motion),
                reduced_motion_subscribers: RefCell::new(HashMap::new()),
                next_reduced_motion_id: Cell::new(1),
                background_blur_supported: Cell::new(options.background_blur_supported),
                background_blur_subscribers: RefCell::new(HashMap::new()),
                next_background_blur_id: Cell::new(1),
                style_diagnostics: RefCell::new(Vec::new()),
                stylesheets: RefCell::new(Vec::new()),
                stylesheet_install_deferred: Cell::new(options.defer_stylesheet_install),
                events,
                dnd: RefCell::new(dnd::DndListeners::default()),
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
                group: RefCell::new(Weak::new()),
                active: Cell::new(false),
                user_gesture_depth: Cell::new(0),
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
    /// Finalizes detachments on `Ok` and disposes on `Err`, across the entire group
    /// if this bridge belongs to one.
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
        let group = self.group.borrow().upgrade();
        if let Some(group) = group {
            return group.execute_turn(action);
        }
        let guard = TurnActiveGuard::enter(self);
        self.registry.borrow_mut().begin_turn();
        let result = action();
        drop(guard);
        if result.is_err() || self.disposed.get() {
            self.dispose();
        } else {
            self.finish_turn();
        }
        result
    }

    fn finish_turn(&self) {
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
            self.dnd.borrow_mut().remove_owner(handle.token());
        }
    }

    pub fn live_count(&self) -> usize { self.registry.borrow().live_count() }
    pub fn wrapper_count(&self) -> usize { self.wrappers.borrow().len() }
    pub fn is_disposed(&self) -> bool { self.disposed.get() }
    pub fn callback_count(&self) -> usize {
        self.events.borrow().len() + self.geometry.borrow().len() + self.dnd.borrow().len()
    }

    /// Resolves a branded `NativeNode` owned by this bridge into its native `NodeId`.
    pub fn resolve_native_node(&self, ctx: &Ctx<'_>, node: &Class<'_, NativeNode>) -> rquickjs::Result<zgui::view::NodeId> {
        let handle = node.borrow().handle;
        self.registry.borrow().native(handle).map_err(|e| host_error(ctx, e))
    }

    /// Resolves a branded `NativeNode` owned by this bridge without throwing into QuickJS,
    /// returning a human-readable ownership/liveness failure reason on mismatch.
    pub fn resolve_owned_native_node(&self, node: &Class<'_, NativeNode>) -> Result<zgui::view::NodeId, String> {
        if self.disposed.get() {
            return Err(BridgeError::Disposed.to_string());
        }
        let handle = node.borrow().handle;
        self.registry.borrow().native(handle).map_err(|e| e.to_string())
    }

    /// Returns whether `native` is still live and effectively inert (itself or any native ancestor
    /// carries `UiState::INERT`).
    #[must_use]
    pub fn is_native_node_effectively_inert(&self, native: zgui::view::NodeId) -> bool {
        if self.disposed.get() {
            return false;
        }
        let registry = self.registry.borrow();
        let dom = registry.dom_handle();
        dom.is_effectively_inert(native)
    }

    /// Returns whether `native` is still live and attached under this bridge's root.
    #[must_use]
    pub fn is_native_node_attached(&self, native: zgui::view::NodeId) -> bool {
        if self.disposed.get() {
            return false;
        }
        let registry = self.registry.borrow();
        let dom = registry.dom_handle();
        if !dom.is_live(native) {
            return false;
        }
        let Ok(root_native) = registry.native(registry.root()) else {
            return false;
        };
        let mut cursor = Some(native);
        let mut hops = 0usize;
        while let Some(id) = cursor {
            if id == root_native {
                return true;
            }
            hops += 1;
            if hops > 4096 {
                return false;
            }
            cursor = dom.parent(id);
        }
        false
    }

    /// Returns the registered JS DnD callback for `(native, kind)`, if any.
    #[must_use]
    pub fn dnd_callback(
        &self,
        native: zgui::view::NodeId,
        kind: dnd::DndEventKind,
    ) -> Option<Persistent<rquickjs::Function<'static>>> {
        self.dnd.borrow().callback(native, kind)
    }
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

    fn sync_geometry_attachment<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<()> {
        let changes = self.geometry.borrow().attachment_changes(&self.registry.borrow());
        for id in changes {
            // Earlier callbacks may cancel a lease or mutate the tree before this one runs.
            let change = self.geometry.borrow_mut().prepare_attachment(id, &self.registry.borrow());
            if let Some((callback, attached)) = change {
                callback.restore(ctx)?.call::<_, Value>((attached,))?;
            }
        }
        Ok(())
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
        let _gesture = snapshot.is_user_gesture().then(|| self.enter_user_gesture());
        if let Some(apply) = apply {
            let result = callback.call::<_, Value>((object,));
            apply(control);
            result.map(|_| ())
        } else {
            drop(control);
            callback.call::<_, Value>((object,)).map(|_| ())
        }
    }

    /// Whether a user-initiated input event handler is currently executing on this bridge.
    pub fn in_user_gesture(&self) -> bool {
        self.user_gesture_depth.get() > 0
            || self.group.borrow().upgrade().is_some_and(|group| group.in_user_gesture())
    }

    fn enter_user_gesture(&self) -> GestureGuard<'_> {
        let group = self.group.borrow().upgrade();
        self.user_gesture_depth.set(self.user_gesture_depth.get().saturating_add(1));
        if let Some(group) = &group {
            group.user_gesture_depth.set(group.user_gesture_depth.get().saturating_add(1));
        }
        GestureGuard { bridge: self, group }
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
        self.dnd.borrow_mut().clear();
        self.geometry.borrow_mut().clear();
        self.scroll_locks.borrow_mut().clear();
        self.pending.borrow_mut().clear();
        let traps = self.focus_traps.borrow_mut().take_all();
        for trap in traps { drop(trap); }
        if !self.disposed.replace(true) { self.registry.borrow_mut().dispose(); }
        self.canvas_scenes.borrow_mut().clear();
        self.reduced_motion_subscribers.borrow_mut().clear();
        self.background_blur_subscribers.borrow_mut().clear();
        self.wrappers.borrow_mut().clear();
        self.properties.borrow_mut().clear();
    }

    /// Sets the initial reduced-motion preference before JS mounts (no subscriber dispatch).
    pub fn seed_reduced_motion(&self, value: bool) {
        self.reduced_motion.set(value);
    }

    /// Sets the initial background-blur capability before JS mounts (no subscriber dispatch).
    pub fn seed_background_blur_supported(&self, value: bool) {
        self.background_blur_supported.set(value);
    }

    /// Updates the bridge's live reduced-motion preference and notifies JS subscribers.
    ///
    /// # Errors
    ///
    /// Propagates any JavaScript exception thrown by a subscriber callback.
    pub fn set_reduced_motion(&self, ctx: &Ctx<'_>, value: bool) -> rquickjs::Result<bool> {
        if self.reduced_motion.get() == value {
            return Ok(false);
        }
        self.reduced_motion.set(value);
        let ids: Vec<u64> = self.reduced_motion_subscribers.borrow().keys().copied().collect();
        for id in ids {
            let callback = self.reduced_motion_subscribers.borrow().get(&id).cloned();
            if let Some(callback) = callback {
                callback.restore(ctx)?.call::<_, ()>((value,))?;
            }
        }
        Ok(true)
    }

    /// Updates the bridge's live background-blur capability and notifies JS subscribers.
    ///
    /// # Errors
    ///
    /// Propagates any JavaScript exception thrown by a subscriber callback.
    pub fn set_background_blur_supported(&self, ctx: &Ctx<'_>, value: bool) -> rquickjs::Result<bool> {
        if self.background_blur_supported.get() == value {
            return Ok(false);
        }
        self.background_blur_supported.set(value);
        let mut ids: Vec<u64> = self.background_blur_subscribers.borrow().keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let callback = self.background_blur_subscribers.borrow().get(&id).cloned();
            if let Some(callback) = callback {
                callback.restore(ctx)?.call::<_, ()>((value,))?;
            }
        }
        Ok(true)
    }

    /// Number of currently active background-blur capability subscribers on this bridge.
    #[must_use]
    pub fn background_blur_subscriber_count(&self) -> usize {
        self.background_blur_subscribers.borrow().len()
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

    /// Installs the private host contract globally for a single-region frontend.
    pub fn install<'js>(self: &Rc<Self>, ctx: &Ctx<'js>) -> rquickjs::Result<()> {
        ctx.globals().set("__zgui", self.api(ctx)?)
    }

    /// Mount layer or lock content against this document without changing the primary bridge.
    ///
    /// # Errors
    ///
    /// Returns an error outside a bridge turn, if mounting throws, or if no disposer is returned.
    pub fn mount_shell_region<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        attach: &rquickjs::Function<'js>,
    ) -> rquickjs::Result<rquickjs::Function<'js>> {
        let api = self.api(ctx)?;
        self.mount_shell_region_with_api(ctx, attach, api)
    }

    /// Mount layer, popup, window, or lock content using a host-augmented region API object.
    ///
    /// # Errors
    ///
    /// Returns an error outside a bridge turn, if mounting throws, or if no disposer is returned.
    pub fn mount_shell_region_with_api<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        attach: &rquickjs::Function<'js>,
        api: Object<'js>,
    ) -> rquickjs::Result<rquickjs::Function<'js>> {
        self.require_turn(ctx)?;
        attach.call((self.root(ctx)?, api))
    }

    /// Builds the region's private host contract without changing any global binding.
    /// Node objects are branded and can only be used with their owning document.
    pub fn api<'js>(self: &Rc<Self>, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
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
                if let Some(dnd_kind) = dnd::DndEventKind::parse(&name) {
                    return bridge.dnd.borrow_mut().set(&ctx, handle, native, dnd_kind, value);
                }
                return bridge.events.borrow_mut().set(&ctx, handle, native, &name, value, bridge.event_dispatcher.clone());
            }
            if let Some(relation) = properties::ScalarRelation::parse(&name) {
                let target_handle = scalar_relation_handle(&ctx, &name, value)?;
                let (dom, native, target_native) = {
                    let registry = bridge.registry.borrow();
                    let source_native = registry.native(handle).map_err(|e| host_error(&ctx, e))?;
                    if registry.is_text(handle).map_err(|e| host_error(&ctx, e))? {
                        return Err(Exception::throw_type(&ctx, &format!("{name} cannot be set on a raw text node")));
                    }
                    let target_native = match target_handle {
                        Some(target) => {
                            let native = registry.native(target).map_err(|e| host_error(&ctx, e))?;
                            if registry.is_text(target).map_err(|e| host_error(&ctx, e))? {
                                return Err(Exception::throw_type(&ctx, &format!("{name} target cannot be a raw text node")));
                            }
                            Some(zgui::vocab::NodeId(native.as_u64()))
                        }
                        None => None,
                    };
                    (registry.dom_handle(), source_native, target_native)
                };
                let mutation = properties::Mutation::ScalarRelation(relation, target_native);
                let result = bridge.properties.borrow_mut().apply(&dom, native, handle.token(), mutation);
                bridge.style_result(&ctx, result)?;
                return Ok(());
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
        let weak = Rc::downgrade(self);
        api.set("getReducedMotion", Func::from(move |ctx: Ctx<'js>| -> rquickjs::Result<bool> {
            Ok(owner(&weak, &ctx)?.reduced_motion.get())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("subscribeReducedMotion", Func::from(move |ctx: Ctx<'js>, callback: rquickjs::Function<'js>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            let mut subs = bridge.reduced_motion_subscribers.borrow_mut();
            if subs.len() >= 128 {
                return Err(Exception::throw_range(&ctx, "reduced-motion subscription limit reached"));
            }
            let id = bridge.next_reduced_motion_id.get();
            bridge.next_reduced_motion_id.set(id.checked_add(1).ok_or_else(|| Exception::throw_range(&ctx, "reduced-motion subscriber IDs exhausted"))?);
            subs.insert(id, Persistent::save(&ctx, callback));
            drop(subs);
            let unsub_weak = weak.clone();
            rquickjs::Function::new(ctx, move || {
                if let Some(bridge) = unsub_weak.upgrade() {
                    bridge.reduced_motion_subscribers.borrow_mut().remove(&id);
                }
            })
        }))?;
        let weak = Rc::downgrade(self);
        api.set("getBackgroundBlurSupported", Func::from(move |ctx: Ctx<'js>| -> rquickjs::Result<bool> {
            Ok(owner(&weak, &ctx)?.background_blur_supported.get())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("subscribeBackgroundBlurSupported", Func::from(move |ctx: Ctx<'js>, callback: rquickjs::Function<'js>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            let mut subs = bridge.background_blur_subscribers.borrow_mut();
            if subs.len() >= 128 {
                return Err(Exception::throw_range(&ctx, "background-blur subscription limit reached"));
            }
            let id = bridge.next_background_blur_id.get();
            bridge.next_background_blur_id.set(id.checked_add(1).ok_or_else(|| Exception::throw_range(&ctx, "background-blur subscriber IDs exhausted"))?);
            subs.insert(id, Persistent::save(&ctx, callback));
            drop(subs);
            let unsub_weak = weak.clone();
            rquickjs::Function::new(ctx, move || {
                if let Some(bridge) = unsub_weak.upgrade() {
                    bridge.background_blur_subscribers.borrow_mut().remove(&id);
                }
            })
        }))?;
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
            result.map_err(|e| host_error(&ctx, e))?;
            bridge.sync_geometry_attachment(&ctx)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("removeNode", Func::from(move |ctx: Ctx<'js>, parent: Class<'js, NativeNode>, node: Class<'js, NativeNode>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let parent = parent.borrow().handle;
            let node = node.borrow().handle;
            let result = bridge.registry.borrow_mut().remove(parent, node);
            result.map_err(|e| host_error(&ctx, e))?;
            bridge.sync_geometry_attachment(&ctx)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("installStylesheet", Func::from(move |ctx: Ctx<'js>, name: String, css: String, target: Opt<Value<'js>>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            if let Some(target) = target.0.filter(|value| !value.is_undefined()) {
                let handle = target.as_object().and_then(|object| object.as_class::<NativeNode>())
                    .ok_or_else(|| Exception::throw_type(&ctx, "invalid native stylesheet target"))?
                    .borrow().handle;
                let registry = bridge.registry.borrow();
                registry.native(handle).map_err(|error| host_error(&ctx, error))?;
                if !registry.attached(handle.token()) {
                    return Err(Exception::throw_reference(&ctx, "native stylesheet target is detached"));
                }
            }
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
            let Some(bridge) = weak.upgrade() else { return Ok(()); };
            if bridge.disposed.get() { return Ok(()); }
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
            bridge.host.focus_with_reveal(native, reveal.0.unwrap_or(false));
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
        api.set("scrollTo", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>, left: Option<f64>, top: Option<f64>, behavior: Opt<String>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let handle = node.borrow().handle;
            let native = bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
            let scale = valid_host_scale(&ctx, bridge.host.scale())?;
            let behavior = parse_scroll_behavior(&ctx, behavior.0.as_deref())?;
            let current = if left.is_none() || top.is_none() {
                Some(bridge.host.scroll_position(native).offset)
            } else {
                None
            };
            let x = match left {
                Some(v) => zgui::geom::DevicePx(css_to_device(&ctx, v, scale, "left")?),
                None => current.expect("current offset sampled").x,
            };
            let y = match top {
                Some(v) => zgui::geom::DevicePx(css_to_device(&ctx, v, scale, "top")?),
                None => current.expect("current offset sampled").y,
            };
            bridge.host.scroll_to(
                native,
                zgui::view::ScrollTarget::Offset(zgui::geom::Point::new(x, y)),
                behavior,
            );
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("scrollBy", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>, left: Option<f64>, top: Option<f64>, behavior: Opt<String>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let handle = node.borrow().handle;
            let native = bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
            let scale = valid_host_scale(&ctx, bridge.host.scale())?;
            let behavior = parse_scroll_behavior(&ctx, behavior.0.as_deref())?;
            let dx = css_to_device(&ctx, left.unwrap_or(0.0), scale, "left")?;
            let dy = css_to_device(&ctx, top.unwrap_or(0.0), scale, "top")?;
            bridge.host.scroll_to(
                native,
                zgui::view::ScrollTarget::By(zgui::geom::Point::new(
                    zgui::geom::DevicePx(dx),
                    zgui::geom::DevicePx(dy),
                )),
                behavior,
            );
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("scrollIntoView", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>, block: Opt<String>, behavior: Opt<String>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let handle = node.borrow().handle;
            let native = bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
            let target = match block.0.as_deref().unwrap_or("nearest") {
                "nearest" => zgui::view::ScrollTarget::IntoView,
                "start" => zgui::view::ScrollTarget::IntoViewStart,
                "center" => zgui::view::ScrollTarget::IntoViewCenter,
                "end" => zgui::view::ScrollTarget::IntoViewEnd,
                other => return Err(Exception::throw_type(&ctx, &format!("unsupported scrollIntoView block: {other}"))),
            };
            let behavior = parse_scroll_behavior(&ctx, behavior.0.as_deref())?;
            bridge.host.scroll_to(native, target, behavior);
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("getSelection", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>| -> rquickjs::Result<Value<'js>> {
            let bridge = owner(&weak, &ctx)?;
            let handle = node.borrow().handle;
            let native = {
                let registry = bridge.registry.borrow();
                registry.require_editable(handle).map_err(|e| host_error(&ctx, e))?;
                registry.native(handle).map_err(|e| host_error(&ctx, e))?
            };
            let Some(range) = bridge.host.selection(native) else {
                return Ok(Value::new_null(ctx.clone()));
            };
            let text = bridge.host.editable_text(native).unwrap_or_default();
            let object = Object::new(ctx.clone())?;
            object.set("start", events::utf8_byte_to_utf16(&text, range.start))?;
            object.set("end", events::utf8_byte_to_utf16(&text, range.end))?;
            Ok(object.into_value())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("setSelection", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>, start: Value<'js>, end: Opt<Value<'js>>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let handle = node.borrow().handle;
            let native = {
                let registry = bridge.registry.borrow();
                registry.require_editable(handle).map_err(|e| host_error(&ctx, e))?;
                registry.native(handle).map_err(|e| host_error(&ctx, e))?
            };
            let start_u16 = parse_selection_offset(&ctx, &start, "start")?;
            let end_u16 = match end.0 {
                None => start_u16,
                Some(ref val) if val.is_undefined() => start_u16,
                Some(ref val) => parse_selection_offset(&ctx, val, "end")?,
            }
            .max(start_u16);
            let text = bridge.host.editable_text(native).unwrap_or_default();
            let start_byte = events::utf16_to_utf8_byte(&text, start_u16);
            let end_byte = events::utf16_to_utf8_byte(&text, end_u16).max(start_byte);
            bridge.host.set_selection(native, start_byte..end_byte);
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("selectAll", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let handle = node.borrow().handle;
            let native = {
                let registry = bridge.registry.borrow();
                registry.require_editable(handle).map_err(|e| host_error(&ctx, e))?;
                registry.native(handle).map_err(|e| host_error(&ctx, e))?
            };
            bridge.host.select_all(native);
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("getDocumentSelection", Func::from(move |ctx: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            let bridge = owner(&weak, &ctx)?;
            let Some(snapshot) = bridge.host.document_selection() else {
                return Ok(Value::new_null(ctx.clone()));
            };
            let handles = {
                let registry = bridge.registry.borrow();
                let Some(container) = registry.handle_for_native(snapshot.container) else {
                    return Ok(Value::new_null(ctx.clone()));
                };
                let Some(anchor) = registry.handle_for_native(snapshot.anchor_node) else {
                    return Ok(Value::new_null(ctx.clone()));
                };
                let Some(focus) = registry.handle_for_native(snapshot.focus_node) else {
                    return Ok(Value::new_null(ctx.clone()));
                };
                (container, anchor, focus)
            };
            let anchor_text = bridge.host.node_text(snapshot.anchor_node).unwrap_or_default();
            let focus_text = bridge.host.node_text(snapshot.focus_node).unwrap_or_default();
            let anchor_obj = Object::new(ctx.clone())?;
            anchor_obj.set("node", bridge.wrap(&ctx, handles.1)?)?;
            anchor_obj.set("offset", events::utf8_byte_to_utf16(&anchor_text, snapshot.anchor_offset))?;
            let focus_obj = Object::new(ctx.clone())?;
            focus_obj.set("node", bridge.wrap(&ctx, handles.2)?)?;
            focus_obj.set("offset", events::utf8_byte_to_utf16(&focus_text, snapshot.focus_offset))?;
            let out = Object::new(ctx.clone())?;
            out.set("container", bridge.wrap(&ctx, handles.0)?)?;
            out.set("anchor", anchor_obj)?;
            out.set("focus", focus_obj)?;
            out.set("text", snapshot.text)?;
            out.set("isCollapsed", snapshot.is_collapsed)?;
            Ok(out.into_value())
        }))?;
        let weak = Rc::downgrade(self);
        api.set(
            "setDocumentSelection",
            Func::from(move |
                ctx: Ctx<'js>,
                container: Class<'js, NativeNode>,
                anchor_node: Class<'js, NativeNode>,
                anchor_offset: Value<'js>,
                focus_node: Opt<Class<'js, NativeNode>>,
                focus_offset: Opt<Value<'js>>,
            | {
                let bridge = owner(&weak, &ctx)?;
                bridge.require_turn(&ctx)?;
                let container_handle = container.borrow().handle;
                let anchor_handle = anchor_node.borrow().handle;
                let focus_handle = focus_node
                    .0
                    .as_ref()
                    .map(|node| node.borrow().handle)
                    .unwrap_or(anchor_handle);
                let (container_native, anchor_native, focus_native) = {
                    let registry = bridge.registry.borrow();
                    (
                        registry.native(container_handle).map_err(|e| host_error(&ctx, e))?,
                        registry.native(anchor_handle).map_err(|e| host_error(&ctx, e))?,
                        registry.native(focus_handle).map_err(|e| host_error(&ctx, e))?,
                    )
                };
                let anchor_u16 = parse_selection_offset(&ctx, &anchor_offset, "anchor.offset")?;
                let focus_u16 = match focus_offset.0 {
                    None => anchor_u16,
                    Some(ref val) if val.is_undefined() => anchor_u16,
                    Some(ref val) => parse_selection_offset(&ctx, val, "focus.offset")?,
                };
                let anchor_text = bridge.host.node_text(anchor_native).unwrap_or_default();
                let focus_text = bridge.host.node_text(focus_native).unwrap_or_default();
                let anchor_byte = events::utf16_to_utf8_byte(&anchor_text, anchor_u16);
                let focus_byte = events::utf16_to_utf8_byte(&focus_text, focus_u16);
                bridge.host.set_document_selection(
                    container_native,
                    (anchor_native, anchor_byte),
                    (focus_native, focus_byte),
                );
                Ok::<(), rquickjs::Error>(())
            }),
        )?;
        let weak = Rc::downgrade(self);
        api.set("clearDocumentSelection", Func::from(move |ctx: Ctx<'js>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            bridge.host.clear_document_selection();
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("selectDocumentAll", Func::from(move |ctx: Ctx<'js>, container: Class<'js, NativeNode>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let handle = container.borrow().handle;
            let native = bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
            bridge.host.select_document_all(native);
            Ok::<(), rquickjs::Error>(())
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
            drop(registry);
            bridge.sync_geometry_attachment(&ctx)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("observeGeometry", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>, name: String, callback: rquickjs::Function<'js>, on_attachment: Opt<rquickjs::Function<'js>>| {
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
            if let Some(on_attachment) = on_attachment.0 {
                let attached = bridge.registry.borrow().attached(handle.token());
                bridge.geometry.borrow_mut().track_attachment(&ctx, id, handle, attached, on_attachment.clone());
                if let Err(error) = on_attachment.call::<_, Value>((attached,)) {
                    bridge.geometry.borrow_mut().cancel(id);
                    return Err(error);
                }
            }
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
        Ok(api)
    }
}

impl Drop for UiBridge {
    fn drop(&mut self) { self.dispose(); }
}

fn valid_host_scale(ctx: &Ctx<'_>, scale: f32) -> rquickjs::Result<f64> {
    let scale = f64::from(scale);
    if !scale.is_finite() || scale <= 0.0 {
        return Err(Exception::throw_range(ctx, "native window scale must be finite and positive"));
    }
    Ok(scale)
}

fn css_to_device(ctx: &Ctx<'_>, css_px: f64, scale: f64, name: &str) -> rquickjs::Result<f32> {
    if !css_px.is_finite() {
        return Err(Exception::throw_range(ctx, &format!("scroll {name} offset must be finite")));
    }
    let device = css_px * scale;
    if !device.is_finite() || device.abs() > f64::from(f32::MAX) {
        return Err(Exception::throw_range(ctx, &format!("scroll {name} offset exceeds device range")));
    }
    Ok(device as f32)
}

fn parse_scroll_behavior(ctx: &Ctx<'_>, value: Option<&str>) -> rquickjs::Result<zgui::view::ScrollBehavior> {
    match value.unwrap_or("instant") {
        "instant" => Ok(zgui::view::ScrollBehavior::Instant),
        "smooth" => Ok(zgui::view::ScrollBehavior::Smooth),
        other => Err(Exception::throw_type(ctx, &format!("unsupported scroll behavior: {other}"))),
    }
}

fn stylesheet_name(ctx: &Ctx<'_>, prefix: &str, key: &str) -> rquickjs::Result<String> {
    validate_stylesheet_name(ctx, key)?;
    Ok(format!("{prefix}{key}"))
}

/// Validates the logical name shared by region and shell stylesheet operations.
///
/// # Errors
///
/// Throws a JavaScript type error for a name outside the allowed ASCII alphabet or length.
pub fn validate_stylesheet_name(ctx: &Ctx<'_>, name: &str) -> rquickjs::Result<()> {
    let valid = !name.is_empty() && name.len() <= 128 && name.bytes().all(|byte|
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'));
    if !valid { return Err(Exception::throw_type(ctx, "stylesheet names use 1-128 ASCII bytes from [a-zA-Z0-9_.:-]")); }
    Ok(())
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

fn scalar_relation_handle<'js>(ctx: &Ctx<'js>, name: &str, value: Value<'js>) -> rquickjs::Result<Option<Handle>> {
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    if value.is_array() {
        return Err(Exception::throw_type(
            ctx,
            &format!("{name} requires a native element node, null, or undefined"),
        ));
    }
    let handle = value
        .as_object()
        .and_then(|object| object.as_class::<NativeNode>())
        .map(|class| class.borrow().handle)
        .ok_or_else(|| {
            Exception::throw_type(
                ctx,
                &format!("{name} requires a native element node, null, or undefined"),
            )
        })?;
    Ok(Some(handle))
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

fn parse_selection_offset(ctx: &Ctx<'_>, value: &Value<'_>, name: &str) -> rquickjs::Result<usize> {
    let Some(number) = value.as_number() else {
        return Err(Exception::throw_type(ctx, &format!("selection {name} offset must be a finite number")));
    };
    if !number.is_finite() {
        return Err(Exception::throw_range(ctx, &format!("selection {name} offset must be a finite number")));
    }
    if number <= 0.0 {
        return Ok(0);
    }
    Ok(number.trunc().min(usize::MAX as f64) as usize)
}

fn host_error(ctx: &Ctx<'_>, error: BridgeError) -> rquickjs::Error {
    Exception::throw_type(ctx, &error.to_string())
}

#[cfg(test)]
mod group_tests {
    use super::*;
    use js_runtime::{JsRuntime, RuntimeLimits};
    use zgui::{
        geom::{DevicePx, Size},
        view::{DomHandle, ElementName, HostHandle, ObservedValue, stub::StubHost},
    };
    use zgui_dom::Document;
    use zgui_view_dom::DocumentDom;

    #[test]
    fn notification_turn_updates_two_regions_and_a_fault_revokes_both() {
        let mut engine = JsRuntime::new(RuntimeLimits::default()).unwrap();
        let group = Rc::new(BridgeGroup::new());
        group.run(engine.turn(), |js| js.evaluate_script("globalThis.__zgui = 'legacy'")).unwrap();

        let mut region = |id, api, root_key| {
            let document = Document::with_id(zgui_dom::DocumentId::new(id).expect("distinct fixture document"));
            let backend = Rc::new(DocumentDom::new(Rc::new(RefCell::new(document))));
            let dom = DomHandle::from_rc(backend.clone());
            let root = dom.create_element(ElementName::new("column"));
            dom.insert(backend.root_node(), root, None);
            let bridge = UiBridge::new(
                NodeRegistry::new(dom, root, 1, 1, 100),
                HostHandle::new(StubHost::default()),
                HostPort::queued(),
            );
            group.add(&bridge);
            engine.with_context(|ctx| {
                ctx.globals().set(api, bridge.api(&ctx)?)?;
                ctx.globals().set(root_key, bridge.root(&ctx)?)
            }).unwrap();
            (bridge, backend, root)
        };
        let (left, left_backend, left_root) = region(0, "left", "leftRoot");
        let (right, _, _) = region(1, "right", "rightRoot");
        group.run(engine.turn(), |js| js.evaluate_script(r#"
            if (__zgui !== 'legacy') throw Error('region API changed the global');
            const a = left.createTextNode('a');
            const b = right.createTextNode('b');
            let rejected = false;
            try { right.insertNode(rightRoot, a, null); }
            catch (error) { rejected = error.message.includes('another native document'); }
            if (!rejected) throw Error('accepted a node from a different region');
            left.insertNode(leftRoot, a, null);
            right.insertNode(rightRoot, b, null);
            left.observeGeometry(leftRoot, 'borderSize', () => {
                left.removeNode(leftRoot, a);
                right.removeNode(rightRoot, b);
            });
            'mounted';
        "#)).unwrap();
        let text = engine.evaluate_script("left.getTextContent(leftRoot) + ':' + right.getTextContent(rightRoot)").expect("read both mounted trees");
        assert_eq!(text, "a:b", "each region mounted its own text");

        left_backend.deliver(left_root, ObservedValue::BorderSize(Size::new(DevicePx(20.0), DevicePx(20.0))));
        assert!(left.flush_notification(engine.turn(), |js, cb| {
            js.with_context(|ctx| cb.call(&ctx))
        }).expect("deliver the left region's observation"), "the left observation was queued");
        engine.evaluate_script(r#"
            for (const [bridge, root, removed] of [[left, leftRoot, a], [right, rightRoot, b]]) {
                if (bridge.getTextContent(root) !== '') throw Error('region retained removed text');
                let stale = false;
                try { bridge.getTextContent(removed); } catch (error) { stale = error.message.includes('stale'); }
                if (!stale) throw Error('removed handle survived the shared turn');
            }
            'detached';
        "#).expect("both registries finalized their detachments");

        let err = right.run_turn(engine.turn(), |_, js| js.evaluate_script(r#"
            left.createTextNode('unattached');
            right.insertNode(rightRoot, right.createTextNode('attached'), null);
            throw Error('region turn failed');
        "#)).unwrap_err();
        assert!(err.message.contains("region turn failed"), "the original failure survived: {err}");
        assert!(left.is_disposed() && right.is_disposed(), "a single failing turn retired both bridges");
        assert_eq!((left.live_count(), right.live_count()), (0, 0), "fault retained native nodes");
        assert_eq!((left.wrapper_count(), right.wrapper_count()), (0, 0), "fault retained JS wrappers");
        assert_eq!(left.callback_count(), 0, "fault retained geometry callbacks");
    }

    #[test]
    fn programmatic_scroll_converts_css_to_device_px_and_validates_handles() {
        let mut engine = JsRuntime::new(RuntimeLimits::default()).expect("create JS engine");
        let document = Rc::new(RefCell::new(Document::new()));
        let backend = Rc::new(DocumentDom::new(document.clone()));
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let stub_host = Rc::new(StubHost::default());
        stub_host.set_scale(2.0);
        let bridge = UiBridge::new(
            NodeRegistry::new(dom, root, 1, 1, 100),
            HostHandle::from_rc(stub_host.clone()),
            HostPort::queued(),
        );
        engine.with_context(|ctx| {
            ctx.globals().set("__zgui", bridge.api(&ctx)?)?;
            ctx.globals().set("root", bridge.root(&ctx)?)
        }).expect("install bridge");

        let scroller_id = bridge.run_turn(engine.turn(), |_, js| js.evaluate_script(r#"
            const scroller = __zgui.createElement('scroll');
            const child = __zgui.createElement('box');
            __zgui.insertNode(scroller, child, null);
            __zgui.insertNode(root, scroller, null);
            globalThis.__scroller = scroller;
            globalThis.__child = child;
            'mounted';
        "#)).expect("mount scroller");
        assert_eq!(scroller_id, "mounted");

        let (scroller_native, child_native) = {
            let registry = bridge.registry.borrow();
            let scroller_handle = registry.first_child(registry.root()).expect("root first child").expect("scroller mounted");
            let child_handle = registry.first_child(scroller_handle).expect("scroller first child").expect("child mounted");
            (
                registry.native(scroller_handle).expect("scroller native id"),
                registry.native(child_handle).expect("child native id"),
            )
        };
        stub_host.set_scroll_position(scroller_native, zgui::view::ScrollPosition {
            offset: zgui::geom::Point::new(DevicePx(30.0), DevicePx(80.0)),
            content_size: Size::new(DevicePx(1000.0), DevicePx(2000.0)),
            scrollport: Size::new(DevicePx(400.0), DevicePx(300.0)),
        });

        bridge.run_turn(engine.turn(), |_, js| js.evaluate_script(r#"
            __zgui.scrollTo(__scroller, null, 120, 'instant');
            __zgui.scrollTo(__scroller, 25, 50, 'smooth');
            __zgui.scrollBy(__scroller, 10, -15, 'instant');
            __zgui.scrollIntoView(__child, 'nearest', 'instant');
            __zgui.scrollIntoView(__child, 'start', 'smooth');
            __zgui.scrollIntoView(__child, 'center', 'instant');
            __zgui.scrollIntoView(__child, 'end', 'instant');
            for (const bad of [
                () => __zgui.scrollTo(__scroller, NaN, 0, 'instant'),
                () => __zgui.scrollBy(__scroller, 0, Infinity, 'instant'),
                () => __zgui.scrollTo(__scroller, 0, 0, 'bouncy'),
                () => __zgui.scrollIntoView(__child, 'bogus', 'instant'),
            ]) {
                let threw = false;
                try { bad(); } catch { threw = true; }
                if (!threw) throw Error('expected invalid scroll arguments to throw');
            }
            __zgui.removeNode(root, __scroller);
            'ok';
        "#)).expect("issue scroll commands");

        let commands = stub_host.scroll_commands();
        assert_eq!(commands, vec![
            (scroller_native, zgui::view::ScrollTarget::Offset(zgui::geom::Point::new(DevicePx(30.0), DevicePx(240.0))), zgui::view::ScrollBehavior::Instant),
            (scroller_native, zgui::view::ScrollTarget::Offset(zgui::geom::Point::new(DevicePx(50.0), DevicePx(100.0))), zgui::view::ScrollBehavior::Smooth),
            (scroller_native, zgui::view::ScrollTarget::By(zgui::geom::Point::new(DevicePx(20.0), DevicePx(-30.0))), zgui::view::ScrollBehavior::Instant),
            (child_native, zgui::view::ScrollTarget::IntoView, zgui::view::ScrollBehavior::Instant),
            (child_native, zgui::view::ScrollTarget::IntoViewStart, zgui::view::ScrollBehavior::Smooth),
            (child_native, zgui::view::ScrollTarget::IntoViewCenter, zgui::view::ScrollBehavior::Instant),
            (child_native, zgui::view::ScrollTarget::IntoViewEnd, zgui::view::ScrollBehavior::Instant),
        ]);

        // Detached handle throws cleanly on subsequent turn.
        bridge.run_turn(engine.turn(), |_, js| js.evaluate_script(r#"
            let threw = false;
            try { __zgui.scrollTo(__scroller, 0, 0, 'instant'); } catch { threw = true; }
            if (!threw) throw Error('detached node scrollTo must throw');
            'ok';
        "#)).expect("detached scrollTo rejected");
    }

    #[test]
    fn background_blur_capability_seeds_notifies_unsubscribes_and_disposes_cleanly() {
        let mut engine = JsRuntime::new(RuntimeLimits::default()).expect("create JS engine");
        let document = Rc::new(RefCell::new(Document::new()));
        let backend = Rc::new(DocumentDom::new(document.clone()));
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let stub_host = Rc::new(StubHost::default());
        let bridge = UiBridge::new_with_options(
            NodeRegistry::new(dom, root, 1, 1, 32),
            HostHandle::from_rc(stub_host),
            HostPort::queued(),
            BridgeOptions { background_blur_supported: false, ..BridgeOptions::default() },
        );
        bridge.seed_background_blur_supported(true);
        engine
            .with_context(|ctx| {
                ctx.globals().set("__zgui", bridge.api(&ctx)?)?;
                ctx.globals().set("root", bridge.root(&ctx)?)
            })
            .expect("install bridge");

        bridge.run_turn(engine.turn(), |_, js| js.evaluate_script(r#"
            if (__zgui.getBackgroundBlurSupported() !== true) {
                throw Error('expected seeded true');
            }
            globalThis.__blurEvents = [];
            globalThis.__unsub1 = __zgui.subscribeBackgroundBlurSupported(v => globalThis.__blurEvents.push(['a', v]));
            globalThis.__unsub2 = __zgui.subscribeBackgroundBlurSupported(v => globalThis.__blurEvents.push(['b', v]));
            'ok';
        "#)).expect("subscribe blur");
        assert_eq!(bridge.background_blur_subscriber_count(), 2);

        // Dedup: setting same value (true) does not dispatch.
        let changed = bridge.run_turn(engine.turn(), |_, js| {
            js.with_context(|ctx| bridge.set_background_blur_supported(&ctx, true))
        }).expect("dedup set true");
        assert!(!changed);

        // Transition true -> false dispatches to both subscribers.
        let changed = bridge.run_turn(engine.turn(), |_, js| {
            js.with_context(|ctx| bridge.set_background_blur_supported(&ctx, false))
        }).expect("set false");
        assert!(changed);

        // Unsubscribe first subscriber, then transition false -> true.
        bridge.run_turn(engine.turn(), |_, js| js.evaluate_script(r#"
            globalThis.__unsub1();
            'ok';
        "#)).expect("unsubscribe first");
        assert_eq!(bridge.background_blur_subscriber_count(), 1);

        let changed = bridge.run_turn(engine.turn(), |_, js| {
            js.with_context(|ctx| bridge.set_background_blur_supported(&ctx, true))
        }).expect("set true");
        assert!(changed);

        let summary = bridge.run_turn(engine.turn(), |_, js| js.evaluate_script(r#"
            if (__zgui.getBackgroundBlurSupported() !== true) throw Error('expected live true');
            JSON.stringify(globalThis.__blurEvents);
        "#)).expect("read blur events");
        assert_eq!(summary, r#"[["a",false],["b",false],["b",true]]"#);

        // Disposal clears remaining subscribers with zero leaks.
        bridge.dispose();
        assert_eq!(bridge.background_blur_subscriber_count(), 0);
    }

    #[test]
    fn aria_active_descendant_validates_atomically_and_rejects_invalid_targets() {
        let mut engine = JsRuntime::new(RuntimeLimits::default()).expect("create JS engine");
        let document = Rc::new(RefCell::new(Document::new()));
        let backend = Rc::new(DocumentDom::new(document.clone()));
        let dom = DomHandle::from_rc(backend.clone());
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(backend.root_node(), root, None);
        let bridge = UiBridge::new(
            NodeRegistry::new(dom, root, 1, 1, 64),
            HostHandle::from_rc(Rc::new(StubHost::default())),
            HostPort::queued(),
        );

        // Second document / session for cross-document & cross-session rejection.
        let other_doc = Rc::new(RefCell::new(Document::new()));
        let other_backend = Rc::new(DocumentDom::new(other_doc));
        let other_dom = DomHandle::from_rc(other_backend.clone());
        let other_root = other_dom.create_element(ElementName::new("column"));
        other_dom.insert(other_backend.root_node(), other_root, None);
        let other_bridge = UiBridge::new(
            NodeRegistry::new(other_dom, other_root, 2, 1, 16),
            HostHandle::from_rc(Rc::new(StubHost::default())),
            HostPort::queued(),
        );

        engine
            .with_context(|ctx| {
                ctx.globals().set("__zgui", bridge.api(&ctx)?)?;
                ctx.globals().set("root", bridge.root(&ctx)?)?;
                ctx.globals().set("__other", other_bridge.api(&ctx)?)?;
                ctx.globals().set("otherRoot", other_bridge.root(&ctx)?)
            })
            .expect("install bridges");

        other_bridge
            .run_turn(engine.turn(), |_, js| {
                js.evaluate_script(
                    r#"
                    globalThis.__foreignOpt = __other.createElement('control');
                    __other.insertNode(otherRoot, globalThis.__foreignOpt, null);
                    'ok';
                "#,
                )
            })
            .expect("create foreign node");

        bridge
            .run_turn(engine.turn(), |_, js| {
                js.evaluate_script(
                    r#"
                    const combo = __zgui.createElement('field');
                    const list = __zgui.createElement('column');
                    const opt1 = __zgui.createElement('control');
                    const textEl = __zgui.createElement('text');
                    const rawText = __zgui.createTextNode('hello');
                    const deadOpt = __zgui.createElement('control');
                    __zgui.insertNode(list, opt1, null);
                    __zgui.insertNode(list, textEl, null);
                    __zgui.insertNode(list, deadOpt, null);
                    __zgui.insertNode(root, combo, null);
                    __zgui.insertNode(root, list, null);
                    __zgui.insertNode(textEl, rawText, null);
                    // Remove deadOpt so finish_turn reclaims it.
                    __zgui.removeNode(list, deadOpt);

                    globalThis.__combo = combo;
                    globalThis.__opt1 = opt1;
                    globalThis.__textEl = textEl;
                    globalThis.__rawText = rawText;
                    globalThis.__deadOpt = deadOpt;

                    // Sibling option (no physical parent-child ancestry, no layout yet) is valid.
                    __zgui.setProperty(combo, 'ariaActiveDescendant', opt1);
                    'ok';
                "#,
                )
            })
            .expect("set initial valid ariaActiveDescendant");

        let read_active_descendant = |doc: &Rc<RefCell<Document>>, native: zgui::view::NodeId| {
            let key = zgui_view_dom::id::to_document(native).expect("document node key");
            doc.borrow()
                .store()
                .columns()
                .semantics
                .get(key)
                .and_then(|slot| slot.as_deref())
                .and_then(|semantics| semantics.relations.active_descendant)
        };

        let (combo_native, opt1_native, text_el_native) = {
            let registry = bridge.registry.borrow();
            let combo_h = registry.first_child(registry.root()).unwrap().unwrap();
            let list_h = registry.next_sibling(combo_h).unwrap().unwrap();
            let opt1_h = registry.first_child(list_h).unwrap().unwrap();
            let text_el_h = registry.next_sibling(opt1_h).unwrap().unwrap();
            (
                registry.native(combo_h).unwrap(),
                registry.native(opt1_h).unwrap(),
                registry.native(text_el_h).unwrap(),
            )
        };
        assert_eq!(
            read_active_descendant(&document, combo_native),
            Some(zgui::vocab::NodeId(opt1_native.as_u64())),
            "valid sibling target must be stored on semantics.relations.active_descendant"
        );

        // Every invalid target must throw TypeError atomically and leave opt1 intact!
        bridge
            .run_turn(engine.turn(), |_, js| {
                js.evaluate_script(
                    r#"
                    const invalidTargets = [
                        [],
                        [__opt1],
                        'option-1',
                        123,
                        true,
                        {},
                        { __native: 1 },
                        __rawText,
                        __deadOpt,
                        __foreignOpt,
                    ];
                    for (const bad of invalidTargets) {
                        let caught = null;
                        try {
                            __zgui.setProperty(__combo, 'ariaActiveDescendant', bad);
                        } catch (error) {
                            caught = error;
                        }
                        if (!(caught instanceof TypeError)) {
                            throw Error('expected TypeError for invalid target, got: ' + caught);
                        }
                    }
                    // Setting ariaActiveDescendant on a raw text source node must also throw TypeError.
                    let sourceErr = null;
                    try {
                        __zgui.setProperty(__rawText, 'ariaActiveDescendant', __opt1);
                    } catch (error) {
                        sourceErr = error;
                    }
                    if (!(sourceErr instanceof TypeError)) {
                        throw Error('expected TypeError for raw text source, got: ' + sourceErr);
                    }
                    'ok';
                "#,
                )
            })
            .expect("invalid targets rejected with TypeError");

        assert_eq!(
            read_active_descendant(&document, combo_native),
            Some(zgui::vocab::NodeId(opt1_native.as_u64())),
            "failed assignments must preserve previous active_descendant atomically"
        );

        // `<text>` element (Kind::Element, not raw text) IS valid.
        bridge
            .run_turn(engine.turn(), |_, js| {
                js.evaluate_script(
                    r#"
                    __zgui.setProperty(__combo, 'ariaActiveDescendant', __textEl);
                    'ok';
                "#,
                )
            })
            .expect("<text> element is a valid target");
        assert_eq!(
            read_active_descendant(&document, combo_native),
            Some(zgui::vocab::NodeId(text_el_native.as_u64()))
        );

        // null clears, then set again and undefined clears.
        bridge
            .run_turn(engine.turn(), |_, js| {
                js.evaluate_script(
                    r#"
                    __zgui.setProperty(__combo, 'ariaActiveDescendant', null);
                    'ok';
                "#,
                )
            })
            .expect("null clears ariaActiveDescendant");
        assert_eq!(read_active_descendant(&document, combo_native), None);

        bridge
            .run_turn(engine.turn(), |_, js| {
                js.evaluate_script(
                    r#"
                    __zgui.setProperty(__combo, 'ariaActiveDescendant', __opt1);
                    __zgui.setProperty(__combo, 'ariaActiveDescendant', undefined);
                    'ok';
                "#,
                )
            })
            .expect("undefined clears ariaActiveDescendant");
        assert_eq!(read_active_descendant(&document, combo_native), None);

        bridge.dispose();
        other_bridge.dispose();
    }

    #[test]
    fn key_event_is_composing_reaches_js_and_guards_capture_escape_dismiss() {
        let mut engine = JsRuntime::new(RuntimeLimits::default()).expect("create JS engine");
        let document = Rc::new(RefCell::new(Document::new()));
        let backend = Rc::new(DocumentDom::new(document.clone()));
        let dom = DomHandle::from_rc(backend.clone());
        let doc_root = backend.root_node();
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(doc_root, root, None);
        let bridge = UiBridge::new(
            NodeRegistry::new(dom, root, 1, 1, 32),
            HostHandle::from_rc(Rc::new(StubHost::default())),
            HostPort::queued(),
        );
        engine
            .with_context(|ctx| {
                ctx.globals().set("__zgui", bridge.api(&ctx)?)?;
                ctx.globals().set("root", bridge.root(&ctx)?)
            })
            .expect("install bridge");

        bridge
            .run_turn(engine.turn(), |_, js| {
                js.evaluate_script(
                    r#"
                    const overlay = __zgui.createElement('box');
                    const field = __zgui.createElement('field');
                    __zgui.insertNode(overlay, field, null);
                    __zgui.insertNode(root, overlay, null);
                    globalThis.__events = [];
                    globalThis.__dismisses = [];
                    __zgui.setProperty(field, 'onKeyDown', e => {
                        globalThis.__events.push(['down', e.key, e.isComposing]);
                    });
                    __zgui.setProperty(field, 'onKeyUp', e => {
                        globalThis.__events.push(['up', e.key, e.isComposing]);
                    });
                    __zgui.registerDismissLayer(
                        overlay,
                        reason => globalThis.__dismisses.push(reason),
                        { layer: 'modal', dismissOnOutsidePress: false, dismissOnEscape: true, exclude: null, leaving: false }
                    );
                    'ok';
                "#,
                )
            })
            .expect("mount overlay + field + dismiss layer");

        let field_native = {
            let registry = bridge.registry.borrow();
            let overlay_h = registry.first_child(registry.root()).unwrap().unwrap();
            let field_h = registry.first_child(overlay_h).unwrap().unwrap();
            registry.native(field_h).unwrap()
        };

        let listeners_for = |node: zgui::view::NodeId, kind: zgui::vocab::EventKind, capture: bool| {
            let key = zgui_view_dom::id::to_document(node).expect("document key");
            let doc = document.borrow();
            let Some(table) = doc.store().columns().listeners.get(key) else {
                return Vec::new();
            };
            table
                .iter()
                .filter(|listener| listener.kind == kind && listener.options.capture == capture)
                .filter_map(|listener| backend.handler(listener.id))
                .collect::<Vec<_>>()
        };

        let dispatch_key = |kind: zgui::vocab::EventKind, key: zgui::vocab::NamedKey, code: zgui::vocab::KeyCode, composing: bool| {
            let payload = zgui::vocab::Payload::Key(zgui::vocab::KeyEvent::named(
                key,
                zgui::vocab::PhysicalKey::Code(code),
            ));
            let control = zgui::view::EventControl::new();
            let mut sink = zgui::view::DiscardCommands;
            // Capture phase on document root (where Dismissals attaches its window listener).
            for listener in listeners_for(doc_root, kind, true) {
                let mut cx = zgui::view::EventCx::<zgui::view::AnyEvent>::new(
                    kind,
                    field_native,
                    doc_root,
                    zgui::vocab::Phase::Capture,
                    zgui::vocab::Modifiers::NONE,
                    zgui::vocab::Timestamp::ORIGIN,
                    &payload,
                    &control,
                    &mut sink,
                )
                .with_composing(composing);
                listener(&mut cx);
            }
            if control.propagation().continues_to_next_element() {
                for listener in listeners_for(field_native, kind, false) {
                    let mut cx = zgui::view::EventCx::<zgui::view::AnyEvent>::new(
                        kind,
                        field_native,
                        field_native,
                        zgui::vocab::Phase::Target,
                        zgui::vocab::Modifiers::NONE,
                        zgui::vocab::Timestamp::ORIGIN,
                        &payload,
                        &control,
                        &mut sink,
                    )
                    .with_composing(composing);
                    listener(&mut cx);
                }
            }
            control.propagation()
        };

        let flush_all = |engine: &mut JsRuntime| {
            while bridge.has_queued_notifications().expect("no queue overflow") {
                bridge
                    .flush_notification(engine.turn(), |js, cb| js.with_context(|ctx| cb.call(&ctx)))
                    .expect("flush queued event");
            }
        };

        // Escape during IME composition must NOT dismiss or stop propagation, and reaches JS with isComposing === true.
        let prop = dispatch_key(zgui::vocab::EventKind::KeyDown, zgui::vocab::NamedKey::Escape, zgui::vocab::KeyCode::Escape, true);
        assert!(prop.continues_to_next_element(), "composing Escape must not be captured by dismiss layer");
        dispatch_key(zgui::vocab::EventKind::KeyUp, zgui::vocab::NamedKey::Escape, zgui::vocab::KeyCode::Escape, true);
        flush_all(&mut engine);

        let state = bridge
            .run_turn(engine.turn(), |_, js| {
                js.evaluate_script("JSON.stringify({ events: globalThis.__events, dismisses: globalThis.__dismisses })")
            })
            .expect("read state after composing escape");
        assert_eq!(
            state,
            r#"{"events":[["down","Escape",true],["up","Escape",true]],"dismisses":[]}"#
        );

        // Ordinary Escape (isComposing === false) triggers capture-phase dismiss and stops propagation!
        let prop = dispatch_key(zgui::vocab::EventKind::KeyDown, zgui::vocab::NamedKey::Escape, zgui::vocab::KeyCode::Escape, false);
        assert!(!prop.continues_to_next_element(), "non-composing Escape must be captured by dismiss layer");
        flush_all(&mut engine);

        let state = bridge
            .run_turn(engine.turn(), |_, js| {
                js.evaluate_script("JSON.stringify({ events: globalThis.__events, dismisses: globalThis.__dismisses })")
            })
            .expect("read state after ordinary escape");
        assert_eq!(
            state,
            r#"{"events":[["down","Escape",true],["up","Escape",true]],"dismisses":["escape"]}"#
        );

        bridge.dispose();
    }

    #[test]
    fn inert_property_sets_ui_state_independently_and_rejects_non_booleans() {
        let mut engine = JsRuntime::new(RuntimeLimits::default()).expect("create JS engine");
        let document = Rc::new(RefCell::new(Document::new()));
        let backend = Rc::new(DocumentDom::new(document.clone()));
        let dom = DomHandle::from_rc(backend.clone());
        let doc_root = backend.root_node();
        let root = dom.create_element(ElementName::new("column"));
        dom.insert(doc_root, root, None);
        let bridge = UiBridge::new(
            NodeRegistry::new(dom, root, 1, 1, 32),
            HostHandle::from_rc(Rc::new(StubHost::default())),
            HostPort::queued(),
        );
        engine
            .with_context(|ctx| {
                ctx.globals().set("__zgui", bridge.api(&ctx)?)?;
                ctx.globals().set("root", bridge.root(&ctx)?)
            })
            .expect("install bridge");

        let report = bridge
            .run_turn(engine.turn(), |_, js| {
                js.evaluate_script(
                    r#"
                    const panel = __zgui.createElement("box");
                    const child = __zgui.createElement("control");
                    __zgui.insertNode(panel, child, null);
                    __zgui.insertNode(root, panel, null);
                    __zgui.setProperty(child, "ariaHidden", true);
                    __zgui.setProperty(child, "disabled", false);
                    __zgui.setProperty(panel, "inert", true);
                    globalThis.__panel = panel;
                    globalThis.__child = child;
                    let threw = false;
                    try {
                      __zgui.setProperty(panel, "inert", "yes");
                    } catch {
                      threw = true;
                    }
                    JSON.stringify({ threw });
                    "#,
                )
            })
            .expect("evaluate inert mutations");
        assert_eq!(report, r#"{"threw":true}"#);

        let child_native = engine
            .with_context(|ctx| {
                let child: Class<NativeNode> = ctx.globals().get("__child")?;
                bridge.resolve_native_node(&ctx, &child)
            })
            .expect("resolve child native node");
        assert!(bridge.is_native_node_effectively_inert(child_native));
        {
            let idx = backend.live_index_of(child_native).expect("child live");
            let doc = document.borrow();
            let key = doc.store().key_of(idx);
            assert!(doc.store().is_effectively_inert(key));
            assert!(!doc.store().core(idx).ui_state().contains(zgui::vocab::UiState::DISABLED));
            let sem = doc
                .store()
                .columns()
                .semantics
                .get(key)
                .and_then(|slot| slot.as_deref())
                .expect("child semantics");
            assert!(sem.flags.contains(zgui::vocab::SemanticFlags::HIDDEN));
        }

        bridge
            .run_turn(engine.turn(), |_, js| {
                js.evaluate_script(
                    r#"
                    __zgui.setProperty(__panel, "inert", false);
                    __zgui.setProperty(__panel, "inert", true);
                    __zgui.setProperty(__panel, "inert", null);
                    __zgui.setProperty(__panel, "inert", true);
                    __zgui.setProperty(__panel, "inert", undefined);
                    'ok';
                    "#,
                )
            })
            .expect("clear inert with false/null/undefined");
        assert!(!bridge.is_native_node_effectively_inert(child_native));
        bridge.dispose();
    }
}
