//! Generation-owned shell requests and monitor subscriptions. No platform work runs inside JS.
use std::{cell::{Cell, RefCell}, collections::BTreeMap, rc::{Rc, Weak}, sync::Arc};

use rquickjs::{Array, Ctx, Exception, Function, Object, Persistent, Value, function::Func};
use serde::{Deserialize, Serialize};
use zgui::{geom::{Css, CssPx, Edges, Size}, platform::{InputRect, InputRegion, KeyboardInteractivity, Layer, LayerPlacement, MonitorInfo, SessionLockState, WakeReason, Waker}};

use crate::{assets::ContractFailure, work::service_error};

pub(crate) const MAX_LAYERS: usize = 32;
const MAX_SUBSCRIPTIONS: usize = 64;
const MAX_INPUT_RECTS: u32 = 256;

/// One named output, with physical desktop geometry and the advertised scale.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Monitor {
    pub id: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    pub refresh_rate_milli_hz: Option<u32>,
}

impl Monitor {
    /// Only named outputs can be targeted by layer-shell.
    pub(crate) fn snapshot(monitors: &[MonitorInfo]) -> Vec<Self> {
        let mut result: Vec<_> = monitors.iter().filter_map(|monitor| {
            let name = monitor.name.as_ref()?.to_string();
            Some(Self {
                id: name.clone(), name,
                x: f64::from(monitor.position.x.0), y: f64::from(monitor.position.y.0),
                width: f64::from(monitor.size.width.0), height: f64::from(monitor.size.height.0),
                scale: monitor.scale_factor, refresh_rate_milli_hz: monitor.refresh_rate_millihertz,
            })
        }).collect();
        result.sort_unstable_by(|a, b| a.id.cmp(&b.id));
        result
    }
}

/// Desired native configuration. The output of a live role cannot change.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayerSpec {
    pub name: String,
    pub placement: LayerPlacement,
    pub size: Size<CssPx, Css>,
    pub input: InputRegion,
}

struct WantedLayer {
    spec: LayerSpec,
    attach: Persistent<Function<'static>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LockSpec {
    pub name: String,
    pub monitor: String,
}

struct WantedLock {
    spec: LockSpec,
    attach: Persistent<Function<'static>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LockCommand { Request, Unlock }


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Authority { Denied, Allowed }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContractPolicy { Throw, Sticky }

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lifecycle { Loading, Active, Stopping }

/// Holds only generation-local requests, never a native surface or registry.
pub(crate) struct Shell {
    authority: Authority,
    contract: ContractPolicy,
    failure: RefCell<Option<ContractFailure>>,
    life: Cell<Lifecycle>,
    monitors: RefCell<Vec<Monitor>>,
    subscriptions: RefCell<BTreeMap<u32, Persistent<Function<'static>>>>,
    layers: RefCell<BTreeMap<u32, WantedLayer>>,
    next_id: Cell<u32>,
    lock_authority: Authority,
    lock_fenced: Cell<bool>,
    lock_state: Cell<SessionLockState>,
    lock_command: Cell<Option<LockCommand>>,
    lock_subscriptions: RefCell<BTreeMap<u32, Persistent<Function<'static>>>>,
    lock_surfaces: RefCell<BTreeMap<u32, WantedLock>>,
    in_cleanup: Cell<bool>,
    cleanup_jobs: Cell<bool>,
    lock_dirty: Cell<bool>,
    dirty: Cell<bool>,
    waker: Arc<dyn Waker>,
}

impl Shell {
    /// Fixes generation authority before preflight; creation opens only for mount.
    pub(crate) fn new(authority: Authority, lock_authority: Authority, lock_state: SessionLockState, contract: ContractPolicy, waker: Arc<dyn Waker>) -> Rc<Self> {
        Rc::new(Self { authority, contract, failure: RefCell::new(None), life: Cell::new(Lifecycle::Loading),
            monitors: RefCell::new(Vec::new()), subscriptions: RefCell::new(BTreeMap::new()), layers: RefCell::new(BTreeMap::new()),
            lock_authority, lock_fenced: Cell::new(false), lock_state: Cell::new(lock_state),
            lock_command: Cell::new(None), lock_subscriptions: RefCell::new(BTreeMap::new()),
            lock_dirty: Cell::new(false),
            lock_surfaces: RefCell::new(BTreeMap::new()), in_cleanup: Cell::new(false),
            cleanup_jobs: Cell::new(false),
            next_id: Cell::new(1), dirty: Cell::new(false), waker })
    }

    pub(crate) fn activate(&self) { self.life.set(Lifecycle::Active); }
    pub(crate) fn clear(&self) {
        self.layers.borrow_mut().clear();
        self.subscriptions.borrow_mut().clear();
        self.lock_surfaces.borrow_mut().clear();
        self.lock_subscriptions.borrow_mut().clear();
        self.lock_command.take();
    }
    pub(crate) fn begin_stop(&self) { self.life.set(Lifecycle::Stopping); }
    pub(crate) fn failure(&self) -> Option<ContractFailure> { self.failure.borrow().clone() }
    pub(crate) fn callback_count(&self) -> usize {
        self.layers.borrow().len() + self.subscriptions.borrow().len()
            + self.lock_surfaces.borrow().len() + self.lock_subscriptions.borrow().len()
    }
    pub(crate) fn is_dirty(&self) -> bool { self.dirty.get() }
    pub(crate) fn locks_are_dirty(&self) -> bool { self.lock_dirty.get() }
    pub(crate) fn take_lock_dirty(&self) -> bool { self.lock_dirty.replace(false) }
    pub(crate) fn has_lock_command(&self) -> bool { self.lock_command.get().is_some() }

    /// Revoke this generation's lock commands and creation without blocking teardown.
    pub(crate) fn fence_lock_authority(&self) {
        self.lock_fenced.set(true);
        self.lock_command.take();
    }

    /// Retention is determined by the compositor, independent of generation authority.
    pub(crate) fn lock_is_retained(&self) -> bool { self.lock_state.get().is_retained() }

    pub(crate) fn take_lock_command(&self) -> Option<LockCommand> { self.lock_command.take() }

    pub(crate) fn with_cleanup<T>(&self, cleanup: impl FnOnce() -> T) -> T {
        let previous = self.in_cleanup.replace(true);
        let result = cleanup();
        self.in_cleanup.set(previous);
        result
    }

    /// Keep teardown restrictions in effect while disposer continuations can still run.
    pub(crate) fn mark_cleanup_jobs_pending(&self) { self.cleanup_jobs.set(true); }

    /// Release the teardown barrier once the engine's job queue is empty.
    pub(crate) fn clear_cleanup_jobs_pending(&self) { self.cleanup_jobs.set(false); }

    /// Block new registrations both inside the disposer and its pending jobs.
    pub(crate) fn cleanup_in_progress(&self) -> bool { self.in_cleanup.get() || self.cleanup_jobs.get() }

    pub(crate) fn update_lock_state(&self, state: SessionLockState) -> bool {
        if self.lock_state.replace(state) == state { return false; }
        self.lock_dirty.set(true);
        if state != SessionLockState::Locked && self.lock_command.get() == Some(LockCommand::Unlock) {
            self.lock_command.take();
        }
        !self.lock_fenced.get() && !self.lock_subscriptions.borrow().is_empty()
    }

    pub(crate) fn dispatch_lock_state(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        let ids: Vec<_> = self.lock_subscriptions.borrow().keys().copied().collect();
        let state = self.lock_state.get().as_str();
        for id in ids {
            let callback = self.lock_subscriptions.borrow().get(&id).cloned();
            if let Some(callback) = callback { callback.restore(ctx)?.call::<_, ()>((state,))?; }
        }
        Ok(())
    }

    pub(crate) fn lock_specs(&self) -> BTreeMap<u32, LockSpec> {
        self.lock_surfaces.borrow().iter().map(|(&id, wanted)| (id, wanted.spec.clone())).collect()
    }

    pub(crate) fn owns_lock(&self, id: u32) -> bool { self.lock_surfaces.borrow().contains_key(&id) }
    pub(crate) fn take_dirty(&self) -> bool { self.dirty.replace(false) }

    /// Snapshot the bounded set only when requests or outputs change.
    pub(crate) fn specs(&self) -> BTreeMap<u32, LayerSpec> {
        self.layers.borrow().iter().map(|(&id, layer)| (id, layer.spec.clone())).collect()
    }

    pub(crate) fn attach(&self, id: u32) -> Option<Persistent<Function<'static>>> {
        self.layers.borrow().get(&id).map(|layer| layer.attach.clone())
            .or_else(|| self.lock_surfaces.borrow().get(&id).map(|wanted| wanted.attach.clone()))
    }

    pub(crate) fn owns(&self, id: u32) -> bool { self.layers.borrow().contains_key(&id) }

    pub(crate) fn updated_input(&self, id: u32, current: &InputRegion) -> Option<InputRegion> {
        let layers = self.layers.borrow();
        let input = &layers.get(&id)?.spec.input;
        (input != current).then(|| input.clone())
    }

    /// Store the new inventory before dispatch; subscriptions added in dispatch start current.
    pub(crate) fn set_monitors(&self, monitors: &[Monitor]) -> bool {
        let mut current = self.monitors.borrow_mut();
        if current.as_slice() == monitors { return false; }
        current.clear();
        current.extend_from_slice(monitors);
        self.dirty.set(true);
        self.lock_dirty.set(true);
        !self.subscriptions.borrow().is_empty()
    }

    pub(crate) fn dispatch_monitors(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        let ids: Vec<_> = self.subscriptions.borrow().keys().copied().collect();
        for id in ids {
            let callback = self.subscriptions.borrow().get(&id).cloned();
            if let Some(callback) = callback { callback.restore(ctx)?.call::<_, ()>((self.monitor_array(ctx)?,))?; }
        }
        Ok(())
    }

    fn monitor_array<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let json = serde_json::to_vec(&*self.monitors.borrow()).expect("monitor snapshots serialize");
        ctx.json_parse(json)
    }

    fn require_authority(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        if self.authority == Authority::Allowed { return Ok(()); }
        self.capability_error(ctx, "layer surfaces require capabilities.shellSurfaces")
    }

    fn require_lock_authority(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        if self.lock_authority == Authority::Allowed { return Ok(()); }
        self.capability_error(ctx, "session lock requires capabilities.sessionLock and backend support")
    }

    fn capability_error(&self, ctx: &Ctx<'_>, message: &str) -> rquickjs::Result<()> {
        if self.contract == ContractPolicy::Sticky {
            self.failure.borrow_mut().get_or_insert_with(|| ContractFailure { code: "E_CAPABILITY", message: message.into() });
        }
        Err(ctx.throw(service_error(ctx, "E_CAPABILITY", message)?.into_value()))
    }

    fn require_active(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        if self.life.get() == Lifecycle::Active { Ok(()) }
        else { Err(Exception::throw_type(ctx, "shell work is unavailable during preflight or disposal")) }
    }

    fn require_creation(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        self.require_active(ctx)?;
        if self.cleanup_in_progress() { return Err(Exception::throw_type(ctx, "shell work is unavailable during disposal")); }
        Ok(())
    }

    fn require_lock_command(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        self.require_lock_authority(ctx)?;
        if self.lock_fenced.get() { return self.capability_error(ctx, "session lock authority has been revoked"); }
        self.require_creation(ctx)
    }

    fn id(&self, ctx: &Ctx<'_>) -> rquickjs::Result<u32> {
        let id = self.next_id.get();
        self.next_id.set(id.checked_add(1).ok_or_else(|| Exception::throw_range(ctx, "shell identifiers exhausted"))?);
        Ok(id)
    }

    fn changed(&self) {
        if !self.dirty.replace(true) { self.waker.wake(WakeReason::AppWork); }
    }

    /// Install the session-level bridge independently of all native documents.
    pub(crate) fn install<'js>(self: &Rc<Self>, ctx: &Ctx<'js>) -> rquickjs::Result<()> {
        let api = Object::new(ctx.clone())?;
        let weak = Rc::downgrade(self);
        api.set("subscribeMonitors", Func::from(move |ctx: Ctx<'js>, callback: Function<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_creation(&ctx)?;
            if shell.subscriptions.borrow().len() >= MAX_SUBSCRIPTIONS {
                return Err(Exception::throw_range(&ctx, "monitor subscription limit reached"));
            }
            let id = shell.id(&ctx)?;
            let initial = shell.monitor_array(&ctx)?;
            shell.subscriptions.borrow_mut().insert(id, Persistent::save(&ctx, callback.clone()));
            if let Err(error) = callback.call::<_, ()>((initial,)) {
                shell.subscriptions.borrow_mut().remove(&id);
                return Err(error);
            }
            let weak = weak.clone();
            Function::new(ctx, move || { if let Some(shell) = weak.upgrade() { shell.subscriptions.borrow_mut().remove(&id); } })
        }))?;
        let weak = Rc::downgrade(self);
        api.set("openLayer", Func::from(move |ctx: Ctx<'js>, name: Value<'js>, data: Value<'js>, attach: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_authority(&ctx)?;
            shell.require_creation(&ctx)?;
            let name = string(&ctx, &name, "shell surface name")?;
            if name == "main" || !valid_name(&name) { return Err(Exception::throw_type(&ctx, "invalid or reserved shell surface name")); }
            let (placement, size) = placement(&ctx, data)?;
            let attach = attach.into_function().ok_or_else(|| Exception::throw_type(&ctx, "shell surface content must be a function"))?;
            let mut layers = shell.layers.borrow_mut();
            if layers.len() >= MAX_LAYERS { return Err(Exception::throw_range(&ctx, "shell surface limit reached")); }
            if layers.values().any(|layer| layer.spec.name == name) { return Err(Exception::throw_type(&ctx, "duplicate shell surface name")); }
            let id = shell.id(&ctx)?;
            if shell.lock_surfaces.borrow().values().any(|wanted| wanted.spec.name == name) {
                return Err(Exception::throw_type(&ctx, "duplicate shell surface name"));
            }
            layers.insert(id, WantedLayer { spec: LayerSpec { name, placement, size, input: InputRegion::Full }, attach: Persistent::save(&ctx, attach) });
            shell.changed();
            Ok(id)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("updateLayer", Func::from(move |ctx: Ctx<'js>, id: Value<'js>, data: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_authority(&ctx)?;
            shell.require_active(&ctx)?;
            let id = layer_id(&ctx, &id)?;
            let (placement, size) = placement(&ctx, data)?;
            let mut layers = shell.layers.borrow_mut();
            let layer = layers.get_mut(&id).ok_or_else(|| Exception::throw_reference(&ctx, "shell surface is no longer owned"))?;
            if layer.spec.placement.monitor != placement.monitor { return Err(Exception::throw_type(&ctx, "changing an output requires a new shell surface")); }
            if layer.spec.placement != placement || layer.spec.size != size {
                layer.spec.placement = placement;
                layer.spec.size = size;
                shell.changed();
            }
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("closeLayer", Func::from(move |ctx: Ctx<'js>, id: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_authority(&ctx)?;
            if shell.life.get() != Lifecycle::Stopping { shell.require_active(&ctx)?; }
            if shell.layers.borrow_mut().remove(&layer_id(&ctx, &id)?).is_some() { shell.changed(); }
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("setInputRegion", Func::from(move |ctx: Ctx<'js>, id: Value<'js>, data: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_authority(&ctx)?;
            if shell.life.get() == Lifecycle::Stopping { return Ok(()); }
            shell.require_active(&ctx)?;
            let id = layer_id(&ctx, &id)?;
            let input = input_region(&ctx, data)?;
            let mut layers = shell.layers.borrow_mut();
            let Some(layer) = layers.get_mut(&id) else { return Ok(()); };
            if layer.spec.input != input { layer.spec.input = input; shell.changed(); }
            Ok::<(), rquickjs::Error>(())
        }))?;
        self.install_lock(&api)?;
        ctx.globals().set("__zguiShell", api)
    }

    fn install_lock<'js>(self: &Rc<Self>, api: &Object<'js>) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        api.set("requestSessionLock", Func::from(move |ctx: Ctx<'js>, callback: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_lock_command(&ctx)?;
            let callback = callback.into_function().ok_or_else(|| Exception::throw_type(&ctx, "session lock observer must be a function"))?;
            if shell.lock_subscriptions.borrow().len() >= MAX_SUBSCRIPTIONS {
                return Err(Exception::throw_range(&ctx, "session lock subscription limit reached"));
            }
            let id = shell.id(&ctx)?;
            let handle = Object::new(ctx.clone())?;
            let unlock = weak.clone();
            handle.set("unlock", Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
                let session = owner(&unlock, &ctx)?;
                session.require_lock_command(&ctx)?;
                if !session.lock_subscriptions.borrow().contains_key(&id) {
                    return Err(Exception::throw_reference(&ctx, "session lock handle was unsubscribed"));
                }
                if session.lock_state.get() != SessionLockState::Locked {
                    return Err(Exception::throw_type(&ctx, "session is not locked"));
                }
                session.lock_command.set(Some(LockCommand::Unlock));
                session.waker.wake(WakeReason::AppWork);
                Ok::<(), rquickjs::Error>(())
            })?)?;
            let unsubscribe = weak.clone();
            handle.set("unsubscribe", Function::new(ctx.clone(), move || {
                if let Some(shell) = unsubscribe.upgrade() { shell.lock_subscriptions.borrow_mut().remove(&id); }
            })?)?;
            shell.lock_subscriptions.borrow_mut().insert(id, Persistent::save(&ctx, callback.clone()));
            if let Err(error) = callback.call::<_, ()>((shell.lock_state.get().as_str(),)) {
                shell.lock_subscriptions.borrow_mut().remove(&id);
                return Err(error);
            }
            if shell.lock_command.get().is_none() { shell.lock_command.set(Some(LockCommand::Request)); }
            shell.waker.wake(WakeReason::AppWork);
            Ok(handle)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("openLock", Func::from(move |ctx: Ctx<'js>, name: Value<'js>, monitor: Value<'js>, attach: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_lock_command(&ctx)?;
            let name = string(&ctx, &name, "lock surface name")?;
            if name == "main" || !valid_name(&name) { return Err(Exception::throw_type(&ctx, "invalid or reserved shell surface name")); }
            let monitor = string(&ctx, &monitor, "output name")?;
            if monitor.is_empty() || monitor.len() > 256 { return Err(Exception::throw_type(&ctx, "invalid output name")); }
            let attach = attach.into_function().ok_or_else(|| Exception::throw_type(&ctx, "lock surface content must be a function"))?;
            let mut surfaces = shell.lock_surfaces.borrow_mut();
            if surfaces.len() >= MAX_LAYERS { return Err(Exception::throw_range(&ctx, "lock surface limit reached")); }
            if surfaces.values().any(|wanted| wanted.spec.monitor == monitor) {
                return Err(Exception::throw_type(&ctx, "output already owns a lock surface"));
            }
            if surfaces.values().any(|wanted| wanted.spec.name == name)
                || shell.layers.borrow().values().any(|wanted| wanted.spec.name == name)
            {
                return Err(Exception::throw_type(&ctx, "duplicate shell surface name"));
            }
            let id = shell.id(&ctx)?;
            surfaces.insert(id, WantedLock { spec: LockSpec { name, monitor }, attach: Persistent::save(&ctx, attach) });
            if !shell.lock_dirty.replace(true) { shell.waker.wake(WakeReason::AppWork); }
            Ok(id)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("closeLock", Func::from(move |ctx: Ctx<'js>, id: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_lock_authority(&ctx)?;
            if shell.life.get() != Lifecycle::Stopping { shell.require_active(&ctx)?; }
            if shell.lock_surfaces.borrow_mut().remove(&layer_id(&ctx, &id)?).is_some()
                && !shell.lock_dirty.replace(true)
            {
                shell.waker.wake(WakeReason::AppWork);
            }
            Ok::<(), rquickjs::Error>(())
        }))?;
        Ok(())
    }
}

fn owner(weak: &Weak<Shell>, ctx: &Ctx<'_>) -> rquickjs::Result<Rc<Shell>> {
    weak.upgrade().ok_or_else(|| Exception::throw_reference(ctx, "shell session is disposed"))
}


/// A bounded identifier usable as a single capture path component.
pub(crate) fn valid_name(name: &str) -> bool {
    name.len() <= 128 && name.as_bytes().first().is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && name.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.'))
}

fn layer_id(ctx: &Ctx<'_>, value: &Value<'_>) -> rquickjs::Result<u32> {
    let id = number(ctx, value, "shell surface ID")?;
    if id.fract() != 0.0 || !(1.0..=f64::from(u32::MAX)).contains(&id) {
        return Err(Exception::throw_range(ctx, "invalid shell surface ID"));
    }
    Ok(id as u32)
}

fn placement<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<(LayerPlacement, Size<CssPx, Css>)> {
    let value = value.into_object().ok_or_else(|| Exception::throw_type(ctx, "layer placement requires an object"))?;
    let output: Value = value.get("output")?;
    let monitor = if output.is_undefined() { None } else {
        let name = string(ctx, &output, "output")?;
        if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err(Exception::throw_type(ctx, "invalid layer output ID"));
        }
        Some(name)
    };
    let layer = match optional_string(ctx, &value, "layer", "top")?.as_str() {
        "background" => Layer::Background, "bottom" => Layer::Bottom, "top" => Layer::Top, "overlay" => Layer::Overlay,
        _ => return Err(Exception::throw_type(ctx, "invalid shell layer")),
    };
    let keyboard = match optional_string(ctx, &value, "keyboard", "none")?.as_str() {
        "none" => KeyboardInteractivity::None, "on-demand" => KeyboardInteractivity::OnDemand, "exclusive" => KeyboardInteractivity::Exclusive,
        _ => return Err(Exception::throw_type(ctx, "invalid layer keyboard interactivity")),
    };
    let mut anchors = Edges::new(false, false, false, false);
    let edges: Value = value.get("anchor")?;
    if !edges.is_undefined() {
        let edges = edges.into_array().ok_or_else(|| Exception::throw_type(ctx, "layer anchor requires an array"))?;
        for index in 0..array_length(ctx, &edges, 4, "layer anchor limit exceeded")? {
            let anchored = match string(ctx, &edges.get(index)?, "anchor edge")?.as_str() {
                "top" => &mut anchors.top, "right" => &mut anchors.right, "bottom" => &mut anchors.bottom, "left" => &mut anchors.left,
                _ => return Err(Exception::throw_type(ctx, "unknown layer anchor edge")),
            };
            if std::mem::replace(anchored, true) { return Err(Exception::throw_type(ctx, "duplicate layer anchor edge")); }
        }
    }
    let stretch_x = anchors.left && anchors.right;
    let stretch_y = anchors.top && anchors.bottom;
    let min_width = if stretch_x { 0.0 } else { 1.0 };
    let min_height = if stretch_y { 0.0 } else { 1.0 };
    let width = distance(ctx, &value, "width", min_width, min_width)?;
    let height = distance(ctx, &value, "height", min_height, min_height)?;
    let zone: Value = value.get("exclusiveZone")?;
    let zone = if zone.is_undefined() { 0.0 } else { number(ctx, &zone, "exclusiveZone")? };
    let exclusive_zone = if zone == -1.0 {
        None
    } else if (0.0..=f64::from(i32::MAX)).contains(&zone) {
        Some(CssPx(zone as f32))
    } else {
        return Err(Exception::throw_range(ctx, "exclusiveZone must be -1 or a nonnegative Wayland distance"));
    };
    let margin: Value = value.get("margin")?;
    let margin = if margin.is_undefined() { Edges::new(CssPx(0.0), CssPx(0.0), CssPx(0.0), CssPx(0.0)) } else {
        let margin = margin.into_object().ok_or_else(|| Exception::throw_type(ctx, "layer margin requires an object"))?;
        Edges::new(distance(ctx, &margin, "top", 0.0, -16384.0)?, distance(ctx, &margin, "right", 0.0, -16384.0)?,
            distance(ctx, &margin, "bottom", 0.0, -16384.0)?, distance(ctx, &margin, "left", 0.0, -16384.0)?)
    };
    Ok((LayerPlacement { layer, anchors, margin, exclusive_zone, keyboard, monitor }, Size::new(width, height)))
}

fn distance(ctx: &Ctx<'_>, value: &Object<'_>, key: &str, default: f64, min: f64) -> rquickjs::Result<CssPx> {
    let value: Value = value.get(key)?;
    let n = if value.is_undefined() { default } else { number(ctx, &value, key)? };
    if !(min..=16384.0).contains(&n) || n.fract() != 0.0 {
        return Err(Exception::throw_range(ctx, &format!("invalid layer {key}: expected an integer in {min}..=16384")));
    }
    Ok(CssPx(n as f32))
}

fn input_region<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<InputRegion> {
    if value.is_string() {
        return match string(ctx, &value, "input region")?.as_str() {
            "full" => Ok(InputRegion::Full), "empty" => Ok(InputRegion::Empty),
            _ => Err(Exception::throw_type(ctx, "input region must be full, empty, or an array of rectangles")),
        };
    }
    let array = value.into_array().ok_or_else(|| Exception::throw_type(ctx, "input region requires an array of rectangles"))?;
    let len = array_length(ctx, &array, MAX_INPUT_RECTS, "input region rectangle limit exceeded")?;
    let mut rects = Vec::with_capacity(len);
    for index in 0..len {
        let rect: Object = array.get(index)?;
        let x = number(ctx, &rect.get("x")?, "rectangle x")?;
        let y = number(ctx, &rect.get("y")?, "rectangle y")?;
        let width = number(ctx, &rect.get("width")?, "rectangle width")?;
        let height = number(ctx, &rect.get("height")?, "rectangle height")?;
        if width < 0.0 || height < 0.0 { return Err(Exception::throw_range(ctx, "input rectangle dimensions must be nonnegative")); }
        if width == 0.0 || height == 0.0 { continue; }
        let left = x.floor();
        let top = y.floor();
        let right = (x + width).ceil();
        let bottom = (y + height).ceil();
        let (left, top, right, bottom) = (region_int(ctx, left)?, region_int(ctx, top)?, region_int(ctx, right)?, region_int(ctx, bottom)?);
        let width = right.checked_sub(left).ok_or_else(|| Exception::throw_range(ctx, "input rectangle width exceeds protocol range"))?;
        let height = bottom.checked_sub(top).ok_or_else(|| Exception::throw_range(ctx, "input rectangle height exceeds protocol range"))?;
        if width > 0 && height > 0 { rects.push(InputRect { x: left, y: top, width, height }); }
    }
    Ok(if rects.is_empty() { InputRegion::Empty } else { InputRegion::Rects(rects) })
}

fn region_int(ctx: &Ctx<'_>, value: f64) -> rquickjs::Result<i32> {
    if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&value) {
        return Err(Exception::throw_range(ctx, "input rectangle exceeds protocol coordinate range"));
    }
    Ok(value as i32)
}

fn array_length(ctx: &Ctx<'_>, array: &Array<'_>, max: u32, error: &str) -> rquickjs::Result<usize> {
    let len: u32 = array.as_object().get("length")?;
    if len > max { return Err(Exception::throw_range(ctx, error)); }
    usize::try_from(len).map_err(|_| Exception::throw_range(ctx, error))
}

fn number(ctx: &Ctx<'_>, value: &Value<'_>, key: &str) -> rquickjs::Result<f64> {
    value.as_number().filter(|value| value.is_finite()).ok_or_else(|| Exception::throw_type(ctx, &format!("{key} must be a finite number")))
}

fn string(ctx: &Ctx<'_>, value: &Value<'_>, key: &str) -> rquickjs::Result<String> {
    value.as_string().ok_or_else(|| Exception::throw_type(ctx, &format!("{key} must be a string")))?.to_string()
}

fn optional_string(ctx: &Ctx<'_>, value: &Object<'_>, key: &str, default: &str) -> rquickjs::Result<String> {
    let value: Value = value.get(key)?;
    if value.is_undefined() { Ok(default.into()) } else { string(ctx, &value, key) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_requests_are_separate_from_layer_authority_and_blocked_during_cleanup() {
        let runtime = rquickjs::Runtime::new().expect("create QuickJS runtime");
        let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
        let waker = zgui::platform::PlatformCx::waker(&zgui_platform_headless::Headless::new());
        let shell = Shell::new(Authority::Denied, Authority::Allowed, SessionLockState::Locked, ContractPolicy::Throw, waker);
        shell.activate();
        context.with(|ctx| {
            shell.install(&ctx).expect("install shell bridge");
            let state = ctx.eval::<String, _>(r#"
                let state;
                let lock = __zguiShell.requestSessionLock(value => { state = value; });
                state
            "#).expect("subscribe under an existing lock");
            assert_eq!(state, "locked", "observer receives retained compositor state");
            assert_eq!(shell.take_lock_command(), Some(LockCommand::Request), "authorized generation stages a request");
            shell.with_cleanup(|| {
                for code in ["lock.unlock()", "__zguiShell.requestSessionLock(() => {})"] {
                    assert!(ctx.eval::<(), _>(code).is_err(), "cleanup admitted {code}");
                    ctx.catch();
                }
            });
            shell.mark_cleanup_jobs_pending();
            assert!(ctx.eval::<(), _>("lock.unlock()").is_err(), "disposer continuation staged unlock");
            ctx.catch();
            assert_eq!(shell.take_lock_command(), None, "cleanup must not leave a staged command");
            shell.clear_cleanup_jobs_pending();
            ctx.eval::<(), _>("lock.unlock()").expect("stage explicit unlock outside cleanup");
            assert_eq!(shell.take_lock_command(), Some(LockCommand::Unlock), "normal turn stages unlock after the cleanup barrier");
            ctx.eval::<(), _>("lock.unlock()").expect("stage a command before fencing");
            let id = ctx.eval::<u32, _>("__zguiShell.openLock('locker', 'output', () => {})")
                .expect("stage a surface before fencing");
            shell.fence_lock_authority();
            assert_eq!(shell.take_lock_command(), None, "fencing drops the staged unlock");
            assert!(!shell.update_lock_state(SessionLockState::Finished), "fenced generation must not receive further observer callbacks");
            assert!(!shell.update_lock_state(SessionLockState::Locked), "compositor state cannot restore generation authority");
            for code in [
                "lock.unlock()",
                "__zguiShell.requestSessionLock(() => {})",
                "__zguiShell.openLock('locker', 'output', () => {})",
            ] {
                assert!(ctx.eval::<(), _>(code).is_err(), "fenced generation admitted {code}");
                ctx.catch();
            }
            ctx.eval::<(), _>(format!("__zguiShell.closeLock({id})")).expect("fenced generation can release its lock surface");
            assert!(!shell.owns_lock(id), "teardown must remove the fenced generation's surface");
            ctx.eval::<(), _>("lock.unsubscribe()").expect("fenced generation can release an observer");
            shell.clear();
        });
    }

    #[test]
    fn exclusive_zone_preserves_the_wayland_sentinel_and_reservations() {
        let runtime = rquickjs::Runtime::new().expect("create QuickJS runtime");
        let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
        context.with(|ctx| {
            for (source, expected) in [
                ("({exclusiveZone:-1})", None),
                ("({exclusiveZone:0})", Some(CssPx(0.0))),
                ("({exclusiveZone:24})", Some(CssPx(24.0))),
                ("({exclusiveZone:1.5})", Some(CssPx(1.5))),
            ] {
                let input = ctx.eval::<Value, _>(source).expect("evaluate placement input");
                let (layer, _) = placement(&ctx, input).expect("valid exclusive zone");
                assert_eq!(layer.exclusive_zone, expected, "placement {source}");
            }
            for invalid in ["-2", "-0.5", "Infinity", "-Infinity", "NaN"] {
                let input = ctx.eval::<Value, _>(format!("({{exclusiveZone:{invalid}}})")).expect("evaluate invalid zone");
                assert!(placement(&ctx, input).is_err(), "accepted invalid exclusive zone {invalid}");
                ctx.catch();
            }
        });
    }
}
