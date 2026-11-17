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

use std::{cell::{Cell, RefCell}, collections::{HashMap, HashSet}, rc::{Rc, Weak}};
use rquickjs::{class::Trace, function::{Func, Opt}, Class, Ctx, Exception, JsLifetime, Object, Persistent, Value};
use registry::{BridgeError, Handle, NodeRegistry, Snapshot};

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub struct NativeNode {
    #[qjs(skip_trace)]
    handle: Handle,
}

/// An unexpected nested notification carries data, never native cancellation authority.
pub struct DeferredEvent { id: u64, snapshot: events::EventSnapshot }
impl DeferredEvent {
    pub fn capture(id: u64, event: &zgui::view::EventCx<'_>) -> Self {
        Self { id, snapshot: events::EventSnapshot::capture(event) }
    }
}

type GeometryDispatcherSlot = Option<(Rc<geometry::Dispatcher>, Rc<dyn Fn()>)>;
pub type EventDispatcher = Rc<events::Dispatcher>;

/// Dispose and drop this before destroying its JS engine. For locals, declare
/// the engine first; for struct fields, place this bridge before the engine.
/// Installed callbacks hold only weak references, so the context cannot own its own roots.
pub struct UiBridge {
    registry: RefCell<NodeRegistry>,
    host: zgui::view::HostHandle,
    wrappers: RefCell<HashMap<u64, Persistent<Object<'static>>>>,
    properties: RefCell<properties::Properties>,
    stylesheets: RefCell<HashSet<String>>,
    events: RefCell<events::Events>,
    dispatcher: RefCell<Option<EventDispatcher>>,
    geometry: RefCell<geometry::Geometry>,
    geometry_dispatcher: RefCell<GeometryDispatcherSlot>,
    focus_traps: RefCell<focus::FocusTraps>,
    dismissals: RefCell<dismiss::Dismissals>,
    scroll_locks: RefCell<scroll::ScrollLocks>,
    active: Cell<bool>,
    stopping: Cell<bool>,
    disposed: Cell<bool>,
}

impl UiBridge {
    /// Uses the host for this registry's native document. Editable values always
    /// go through its editing API, never through reconciled text descendants.
    pub fn new(registry: NodeRegistry, host: zgui::view::HostHandle) -> Rc<Self> {
        let events = RefCell::new(events::Events::new(registry.dom_handle()));
        let geometry = RefCell::new(geometry::Geometry::new(registry.dom_handle(),
            registry.native(registry.root()).expect("the session root is a live native node"), host.clone(), registry.node_limit().saturating_mul(4)));
        let focus_traps = RefCell::new(focus::FocusTraps::new(registry.node_limit()));
        let dismissals = RefCell::new(dismiss::Dismissals::new(registry.dom_handle(),
            registry.native(registry.root()).expect("the session root is a live native node"), registry.node_limit()));
        let scroll_locks = RefCell::new(scroll::ScrollLocks::new(registry.dom_handle(),
            registry.native(registry.root()).expect("the session root is a live native node"), host.clone(), registry.node_limit()));
        Rc::new(Self { registry: RefCell::new(registry), host, wrappers: RefCell::new(HashMap::new()), properties: RefCell::new(properties::Properties::default()), stylesheets: RefCell::new(HashSet::new()), events, dispatcher: RefCell::new(None), geometry, geometry_dispatcher: RefCell::new(None), focus_traps, dismissals, scroll_locks, active: Cell::new(false), stopping: Cell::new(false), disposed: Cell::new(false) })
    }

    /// Finalizes detachments after successful entry; a failed update faults and
    /// tears down the region because prior native writes cannot be rolled back.
    pub fn run_turn<T, E>(&self, action: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        assert!(!self.active.replace(true), "nested bridge entry");
        self.registry.borrow_mut().begin_turn();
        let result = action();
        self.active.set(false);
        let invalid = if result.is_ok() { self.registry.borrow_mut().finish_turn() }
            else { self.disposed.set(true); self.registry.borrow_mut().abort_turn() };
        let released = if result.is_err() { self.dismissals.borrow_mut().clear() }
            else { self.dismissals.borrow_mut().remove_nodes(&invalid) };
        for callback in released { self.events.borrow_mut().remove_callback(callback); }
        if result.is_err() { self.geometry.borrow_mut().clear(); }
        else { self.geometry.borrow_mut().remove_nodes(&invalid); }
        if result.is_err() { self.scroll_locks.borrow_mut().clear(); }
        else { self.scroll_locks.borrow_mut().remove_nodes(&invalid); }
        let traps = if result.is_err() {
            self.focus_traps.borrow_mut().take_all()
        } else {
            self.focus_traps.borrow_mut().remove_nodes(&invalid)
        };
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

    /// Copies the attached live tree without entering JavaScript. The returned
    /// snapshot owns its records and holds no registry borrow during subsequent
    /// native queries. See [`NodeRegistry::snapshot`] for ordering and limits.
    pub fn snapshot(&self, max_nodes: usize) -> Snapshot {
        self.registry.borrow().snapshot(max_nodes)
    }

    /// Diagnostic changes need a fresh validation frame even without native DOM changes.
    pub fn metadata_revision(&self) -> u64 { self.registry.borrow().metadata_revision() }

    pub fn set_dispatcher(&self, dispatcher: EventDispatcher) {
        *self.dispatcher.borrow_mut() = Some(dispatcher);
    }

    pub fn set_geometry_dispatcher(&self, dispatcher: Rc<dyn Fn(u64, zgui::view::ObservedValue, f32)>, wake: Rc<dyn Fn()>) {
        *self.geometry_dispatcher.borrow_mut() = Some((dispatcher, wake));
    }

    pub fn invoke_geometry<'js>(&self, ctx: &Ctx<'js>, id: u64, value: zgui::view::ObservedValue, scale: f32) -> rquickjs::Result<()> {
        self.require_turn(ctx)?;
        let prepared = self.geometry.borrow_mut().prepare(ctx, id, value, scale)?;
        let Some((callback, measurement)) = prepared else { return Ok(()); };
        let object = measurement.into_js(ctx)?;
        callback.restore(ctx)?.call::<_, Value>((object,)).map(|_| ())
    }

    /// Called inside the owning engine entry, with no registry borrow held.
    pub fn invoke_event<'js>(&self, ctx: &Ctx<'js>, id: u64, event: &mut zgui::view::EventCx<'_>) -> rquickjs::Result<()> {
        self.invoke_snapshot(ctx, id, events::EventSnapshot::capture(event), Some(event))
    }

    pub fn invoke_deferred<'js>(&self, ctx: &Ctx<'js>, event: DeferredEvent) -> rquickjs::Result<()> {
        self.invoke_snapshot(ctx, event.id, event.snapshot, None)
    }

    fn invoke_snapshot<'js>(&self, ctx: &Ctx<'js>, id: u64, snapshot: events::EventSnapshot, native: Option<&mut zgui::view::EventCx<'_>>) -> rquickjs::Result<()> {
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
        if let Some(native) = native {
            let result = callback.call::<_, Value>((object,));
            control.expire_and_apply(native);
            result.map(|_| ())
        } else {
            drop(control);
            callback.call::<_, Value>((object,)).map(|_| ())
        }
    }

    /// Retiring owners may remove resources, but cannot allocate or register new work.
    pub fn begin_stop(&self) { self.stopping.set(true); }

    pub fn dispose(&self) {
        let stylesheets: Vec<String> = self.stylesheets.borrow_mut().drain().collect();
        for name in stylesheets { self.host.remove_stylesheet(&name); }
        for callback in self.dismissals.borrow_mut().clear() {
            self.events.borrow_mut().remove_callback(callback);
        }
        self.events.borrow_mut().clear();
        self.geometry.borrow_mut().clear();
        self.scroll_locks.borrow_mut().clear();
        self.geometry_dispatcher.borrow_mut().take();
        let traps = self.focus_traps.borrow_mut().take_all();
        for trap in traps { drop(trap); }
        if !self.disposed.replace(true) { self.registry.borrow_mut().dispose(); }
        self.wrappers.borrow_mut().clear();
        self.properties.borrow_mut().clear();
        self.dispatcher.borrow_mut().take();
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
        let weak = Rc::downgrade(self);
        api.set("setProperty", Func::from(move |ctx: Ctx<'js>, node: Class<'js, NativeNode>, name: String, value: Value<'js>, _previous: Opt<Value<'js>>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let handle = node.borrow().handle;
            if name.starts_with("on") {
                if !value.is_null() && !value.is_undefined() { bridge.require_creation(&ctx)?; }
                let native = bridge.registry.borrow().native(handle).map_err(|e| host_error(&ctx, e))?;
                let dispatcher = bridge.dispatcher.borrow().clone().ok_or_else(|| Exception::throw_type(&ctx, "native event dispatcher not installed"))?;
                return bridge.events.borrow_mut().set(&ctx, handle, native, &name, value, dispatcher);
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
                bridge.properties.borrow_mut().apply(&dom, native, handle.token(), mutation);
                return Ok(());
            }
            let mutation = properties::Mutation::parse(&ctx, &name, value)?;
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
            let value = bridge.properties.borrow_mut().apply(&dom, native, handle.token(), mutation);
            // Native editing can notify listeners; release all registry/property
            // borrows before entering the host.
            if let Some(value) = value { bridge.host.set_value(native, &value); }
            Ok::<(), rquickjs::Error>(())
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
            bridge.stylesheets.borrow_mut().insert(name.clone());
            bridge.host.install_stylesheet(&name, &css);
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("removeStylesheet", Func::from(move |ctx: Ctx<'js>, name: String| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_turn(&ctx)?;
            let name = stylesheet_name(&ctx, &bridge.registry.borrow().stylesheet_prefix(), &name)?;
            if bridge.stylesheets.borrow_mut().remove(&name) {
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
            let dispatcher = bridge.dispatcher.borrow().clone().ok_or_else(|| Exception::throw_type(&ctx, "native event dispatcher not installed"))?;
            let id = bridge.dismissals.borrow_mut().reserve(&ctx)?;
            let callback_id = bridge.events.borrow_mut().register_dismiss(&ctx, callback)?;
            bridge.dismissals.borrow_mut().add(id, handle, native, options, callback_id, dispatcher);
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
            let (dispatcher, wake) = bridge.geometry_dispatcher.borrow().clone()
                .ok_or_else(|| Exception::throw_type(&ctx, "native geometry dispatcher not installed"))?;
            let id = bridge.geometry.borrow_mut().reserve(&ctx)?;
            let cancel = {
                let weak = Rc::downgrade(&bridge);
                rquickjs::Function::new(ctx.clone(), move || {
                    if let Some(bridge) = weak.upgrade() { bridge.geometry.borrow_mut().cancel(id); }
                })?
            };
            bridge.geometry.borrow_mut().add(&ctx, id, handle, native, what, callback, dispatcher);
            wake();
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
