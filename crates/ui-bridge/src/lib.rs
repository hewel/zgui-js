//! Owner-thread native tree bridge. JavaScript sees branded objects, never native IDs.
//! A structural turn is one outermost JavaScript entry; pending removals expire at its end.
#![forbid(unsafe_code)]

pub mod registry;
mod properties;
mod events;

use std::{cell::{Cell, RefCell}, collections::HashMap, rc::{Rc, Weak}};
use rquickjs::{class::Trace, function::{Func, Opt}, Class, Ctx, Exception, JsLifetime, Object, Persistent, Value};
use registry::{BridgeError, Handle, NodeRegistry};

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

/// Dispose and drop this before destroying its JS engine. For locals, declare
/// the engine first; for struct fields, place this bridge before the engine.
/// Installed callbacks hold only weak references, so the context cannot own its own roots.
pub struct UiBridge {
    registry: RefCell<NodeRegistry>,
    wrappers: RefCell<HashMap<u64, Persistent<Object<'static>>>>,
    properties: RefCell<properties::Properties>,
    events: RefCell<events::Events>,
    dispatcher: RefCell<Option<Rc<events::Dispatcher>>>,
    active: Cell<bool>,
    stopping: Cell<bool>,
    disposed: Cell<bool>,
}

impl UiBridge {
    pub fn new(registry: NodeRegistry) -> Rc<Self> {
        let events = RefCell::new(events::Events::new(registry.dom_handle()));
        Rc::new(Self { registry: RefCell::new(registry), wrappers: RefCell::new(HashMap::new()), properties: RefCell::new(properties::Properties::default()), events, dispatcher: RefCell::new(None), active: Cell::new(false), stopping: Cell::new(false), disposed: Cell::new(false) })
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
    pub fn callback_count(&self) -> usize { self.events.borrow().len() }

    pub fn set_dispatcher(&self, dispatcher: Rc<dyn Fn(u64, &mut zgui::view::EventCx<'_>)>) {
        *self.dispatcher.borrow_mut() = Some(dispatcher);
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
        let callback = self.events.borrow().callback(id);
        let Some(callback) = callback else { return Ok(()); };
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
        self.events.borrow_mut().clear();
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
            let mutation = properties::Mutation::parse(&ctx, &name, value)?;
            let (dom, native) = {
                let registry = bridge.registry.borrow();
                (registry.dom_handle(), registry.native(handle).map_err(|e| host_error(&ctx, e))?)
            };
            bridge.properties.borrow_mut().apply(&dom, native, handle.token(), mutation);
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

fn owner(weak: &Weak<UiBridge>, ctx: &Ctx<'_>) -> rquickjs::Result<Rc<UiBridge>> {
    weak.upgrade().ok_or_else(|| Exception::throw_reference(ctx, "native session no longer exists"))
}
fn host_error(ctx: &Ctx<'_>, error: BridgeError) -> rquickjs::Error {
    Exception::throw_type(ctx, &error.to_string())
}
