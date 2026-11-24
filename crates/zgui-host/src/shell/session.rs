//! Generation-owned shell requests and monitor subscriptions. No platform work runs inside JS.
use std::{cell::{Cell, RefCell}, collections::BTreeMap, rc::{Rc, Weak}, sync::{Arc, atomic::{AtomicU64, Ordering}}};

use rquickjs::{Array, Class, Ctx, Exception, Function, Object, Persistent, Value, function::Func, object::Property};
use serde::{Deserialize, Serialize};
use ui_bridge::{NativeNode, UiBridge};
use zgui::{geom::{Css, CssPx, Edges, Point, Rect, Size}, platform::{ExclusiveZone, InputRect, InputRegion, KeyboardInteractivity, Layer, LayerPlacement, MonitorInfo, MonitorTarget, SessionLockState, WakeReason, Waker}, runtime::{BlurRegistration, BlurSlot}, view::DocumentId};
use zgui_dom::NodeKey;

use crate::{assets::{ContractFailure, MAX_SAFE_ID}, session::Session, work::service_error};

mod popup;
mod window;
pub(crate) use popup::PopupSpec;
use popup::{WantedPopup, duplicate_name};
pub(crate) use window::ShellWindowSpec;
use window::WantedWindow;

pub(crate) const MAX_LAYERS: usize = 32;
const MAX_SUBSCRIPTIONS: usize = 64;
const MAX_INPUT_RECTS: u32 = 256;
const MAX_BLUR_RECTS: u32 = 256;
const MAX_BLUR_NODES: u32 = 1024;
static NEXT_LOCK_ID: AtomicU64 = AtomicU64::new(1);

/// Owned normalized background blur intent stored on shell surface specs.
///
/// `None`, `Full`, and `Rects` remain valid before the target surface mounts; `Nodes` binds
/// strictly to the target mounted document instance (`DocumentId`) so named-layer recreation
/// reusing the same shell request ID never inherits stale node handles.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum DesiredBlur {
    #[default]
    None,
    Full,
    Rects(Vec<Rect<CssPx, Css>>),
    Nodes {
        document: DocumentId,
        nodes: Vec<NodeKey>,
        min_opacity: f64,
    },
}

impl DesiredBlur {
    /// Convert into a runtime [`BlurRegistration`] for the window owning `target_document`.
    #[must_use]
    pub(crate) fn to_registration(&self, target_document: DocumentId) -> BlurRegistration {
        match self {
            Self::None => BlurRegistration::None,
            Self::Full => BlurRegistration::Full,
            Self::Rects(rects) => BlurRegistration::Rects(rects.clone()),
            Self::Nodes { document, nodes, min_opacity } => {
                if *document == target_document {
                    BlurRegistration::Nodes {
                        nodes: nodes.clone(),
                        min_opacity: *min_opacity,
                    }
                } else {
                    BlurRegistration::None
                }
            }
        }
    }
}

#[derive(Clone)]
struct AttachedBlurTarget {
    document: DocumentId,
    bridge: Rc<UiBridge>,
    slot: Option<Rc<BlurSlot>>,
}

fn allocate_lock_id(next: &AtomicU64) -> Option<u64> {
    next.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
        (id <= MAX_SAFE_ID).then(|| id + 1)
    }).ok()
}

/// One named output, with physical desktop geometry, compositor logical extent, and advertised scale.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Monitor {
    pub id: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_height: Option<f64>,
    pub scale: f64,
    pub refresh_rate_milli_hz: Option<u32>,
}

impl Monitor {
    /// Compositor-reported logical width, falling back to `width / scale`.
    pub(crate) fn logical_width(&self) -> f64 {
        self.logical_width.unwrap_or_else(|| self.width / self.scale)
    }

    /// Compositor-reported logical height, falling back to `height / scale`.
    pub(crate) fn logical_height(&self) -> f64 {
        self.logical_height.unwrap_or_else(|| self.height / self.scale)
    }

    /// Populate explicit logical dimensions from `width / scale` and `height / scale` when absent.
    pub(crate) fn normalize_logical_size(&mut self) {
        if self.logical_width.is_none() {
            self.logical_width = Some(self.width / self.scale);
        }
        if self.logical_height.is_none() {
            self.logical_height = Some(self.height / self.scale);
        }
    }

    /// Only named outputs can be targeted by layer-shell.
    pub(crate) fn snapshot(monitors: &[MonitorInfo]) -> Vec<Self> {
        let mut result: Vec<_> = monitors.iter().filter_map(|monitor| {
            let name = monitor.name.as_ref()?.to_string();
            let logical = monitor.logical_size();
            Some(Self {
                id: name.clone(), name,
                x: f64::from(monitor.position.x.0), y: f64::from(monitor.position.y.0),
                width: f64::from(monitor.size.width.0), height: f64::from(monitor.size.height.0),
                logical_width: Some(f64::from(logical.width.0)),
                logical_height: Some(f64::from(logical.height.0)),
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
    pub blur: DesiredBlur,
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

/// Fixed generation identity, authority, and initial compositor state.
#[derive(Debug)]
pub(crate) struct ShellConfig {
    pub(crate) generation: u64,
    pub(crate) authority: Authority,
    pub(crate) window_authority: Authority,
    pub(crate) lock_authority: Authority,
    pub(crate) lock_state: SessionLockState,
    pub(crate) contract: ContractPolicy,
}

/// Holds only generation-local requests, never a native surface or registry.
pub(crate) struct Shell {
    generation: u64,
    authority: Authority,
    window_authority: Authority,
    contract: ContractPolicy,
    failure: RefCell<Option<ContractFailure>>,
    life: Cell<Lifecycle>,
    monitors: RefCell<Vec<Monitor>>,
    subscriptions: RefCell<BTreeMap<u32, Persistent<Function<'static>>>>,
    background_blur_supported: Cell<bool>,
    blur_support_subscribers: RefCell<BTreeMap<u32, Persistent<Function<'static>>>>,
    layers: RefCell<BTreeMap<u32, WantedLayer>>,
    popups: RefCell<BTreeMap<u32, WantedPopup>>,
    windows: RefCell<BTreeMap<u32, WantedWindow>>,
    blur_targets: RefCell<BTreeMap<u32, AttachedBlurTarget>>,
    next_id: Cell<u32>,
    lock_authority: Authority,
    lock_fenced: Cell<bool>,
    lock_state: Cell<SessionLockState>,
    lock_command: Cell<Option<LockCommand>>,
    lock_subscriptions: RefCell<BTreeMap<u64, Persistent<Function<'static>>>>,
    lock_surfaces: RefCell<BTreeMap<u32, WantedLock>>,
    in_cleanup: Cell<bool>,
    cleanup_jobs: Cell<bool>,
    lock_dirty: Cell<bool>,
    dirty: Cell<bool>,
    waker: Arc<dyn Waker>,
}

impl Shell {
    /// Fixes generation authority before preflight; creation opens only for mount.
    pub(crate) fn new(config: ShellConfig, waker: Arc<dyn Waker>) -> Rc<Self> {
        let ShellConfig { generation, authority, window_authority, lock_authority, lock_state, contract } = config;
        Rc::new(Self { generation, authority, window_authority, contract, failure: RefCell::new(None), life: Cell::new(Lifecycle::Loading),
            monitors: RefCell::new(Vec::new()), subscriptions: RefCell::new(BTreeMap::new()),
            background_blur_supported: Cell::new(false), blur_support_subscribers: RefCell::new(BTreeMap::new()),
            layers: RefCell::new(BTreeMap::new()),
            popups: RefCell::new(BTreeMap::new()), windows: RefCell::new(BTreeMap::new()),
            blur_targets: RefCell::new(BTreeMap::new()),
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
        self.popups.borrow_mut().clear();
        self.windows.borrow_mut().clear();
        self.blur_targets.borrow_mut().clear();
        self.subscriptions.borrow_mut().clear();
        self.blur_support_subscribers.borrow_mut().clear();
        self.lock_surfaces.borrow_mut().clear();
        self.lock_subscriptions.borrow_mut().clear();
        self.lock_command.take();
    }
    pub(crate) fn begin_stop(&self) { self.life.set(Lifecycle::Stopping); }
    pub(crate) fn failure(&self) -> Option<ContractFailure> { self.failure.borrow().clone() }
    pub(crate) fn callback_count(&self) -> usize {
        self.layers.borrow().len() + self.subscriptions.borrow().len()
            + self.blur_support_subscribers.borrow().len()
            + 2 * self.popups.borrow().len() + 2 * self.windows.borrow().len()
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
            .or_else(|| self.popups.borrow().get(&id).map(|wanted| wanted.attach.clone()))
            .or_else(|| self.windows.borrow().get(&id).map(|wanted| wanted.attach.clone()))
            .or_else(|| self.lock_surfaces.borrow().get(&id).map(|wanted| wanted.attach.clone()))
    }

    pub(crate) fn owns(&self, id: u32) -> bool { self.layers.borrow().contains_key(&id) }

    #[must_use]
    fn name_is_taken(&self, name: &str) -> bool {
        self.layers.borrow().values().any(|wanted| wanted.spec.name == name)
            || self.popups.borrow().values().any(|wanted| wanted.spec.name == name)
            || self.windows.borrow().values().any(|wanted| wanted.spec.name == name)
            || self.lock_surfaces.borrow().values().any(|wanted| wanted.spec.name == name)
    }

    pub(crate) fn updated_input(&self, id: u32, current: &InputRegion) -> Option<InputRegion> {
        let layers = self.layers.borrow();
        if let Some(layer) = layers.get(&id) {
            return (layer.spec.input != *current).then(|| layer.spec.input.clone());
        }
        drop(layers);
        let popups = self.popups.borrow();
        let input = &popups.get(&id)?.spec.input;
        (input != current).then(|| input.clone())
    }

    /// Bind a mounted non-lock shell surface's bridge and live window blur slot, seeding any
    /// pre-mount non-node blur intent immediately before the surface's first composition.
    pub(crate) fn attach_blur_target(
        &self,
        id: u32,
        document: DocumentId,
        bridge: Rc<UiBridge>,
        slot: Option<Rc<BlurSlot>>,
    ) {
        let initial = self.current_blur(id);
        if let Some(slot) = &slot {
            slot.set(initial.to_registration(document));
        }
        self.blur_targets.borrow_mut().insert(id, AttachedBlurTarget { document, bridge, slot });
    }

    /// Detach the mounted blur target for `id` when its native document closes or recreates.
    pub(crate) fn detach_blur_target(&self, id: u32) {
        self.blur_targets.borrow_mut().remove(&id);
    }

    fn blur_target(&self, id: u32) -> Option<AttachedBlurTarget> {
        self.blur_targets.borrow().get(&id).cloned()
    }

    fn current_blur(&self, id: u32) -> DesiredBlur {
        if let Some(layer) = self.layers.borrow().get(&id) {
            return layer.spec.blur.clone();
        }
        if let Some(popup) = self.popups.borrow().get(&id) {
            return popup.spec.blur.clone();
        }
        if let Some(window) = self.windows.borrow().get(&id) {
            return window.spec.blur.clone();
        }
        DesiredBlur::None
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

    pub(crate) fn seed_background_blur_supported(&self, supported: bool) {
        self.background_blur_supported.set(supported);
    }

    pub(crate) fn dispatch_background_blur_supported(&self, ctx: &Ctx<'_>, supported: bool) -> rquickjs::Result<()> {
        self.background_blur_supported.set(supported);
        let ids: Vec<_> = self.blur_support_subscribers.borrow().keys().copied().collect();
        for id in ids {
            let callback = self.blur_support_subscribers.borrow().get(&id).cloned();
            if let Some(callback) = callback {
                callback.restore(ctx)?.call::<_, ()>((supported,))?;
            }
        }
        Ok(())
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
        self.capability_error(ctx, "shell surfaces require capabilities.shellSurfaces")
    }

    fn require_window_authority(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        if self.window_authority == Authority::Allowed { return Ok(()); }
        self.capability_error(ctx, "window surfaces require a primary window or capabilities.shellSurfaces")
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
    pub(crate) fn install<'js>(self: &Rc<Self>, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        let api = Object::new(ctx.clone())?;
        let generation = self.generation as f64;
        api.prop("getSessionGeneration", Property::from(Func::from(move || generation)))?;
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
        api.set("getBackgroundBlurSupported", Func::from(move |ctx: Ctx<'js>| -> rquickjs::Result<bool> {
            Ok(owner(&weak, &ctx)?.background_blur_supported.get())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("subscribeBackgroundBlurSupported", Func::from(move |ctx: Ctx<'js>, callback: Function<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_creation(&ctx)?;
            if shell.blur_support_subscribers.borrow().len() >= MAX_SUBSCRIPTIONS {
                return Err(Exception::throw_range(&ctx, "background-blur subscription limit reached"));
            }
            let id = shell.id(&ctx)?;
            shell.blur_support_subscribers.borrow_mut().insert(id, Persistent::save(&ctx, callback));
            let weak = weak.clone();
            Function::new(ctx, move || {
                if let Some(shell) = weak.upgrade() {
                    shell.blur_support_subscribers.borrow_mut().remove(&id);
                }
            })
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
            if shell.layers.borrow().len() + shell.popups.borrow().len() + shell.windows.borrow().len() >= MAX_LAYERS {
                return Err(Exception::throw_range(&ctx, "shell surface limit reached"));
            }
            if shell.name_is_taken(&name) { return Err(duplicate_name(&ctx)); }
            let id = shell.id(&ctx)?;
            shell.layers.borrow_mut().insert(id, WantedLayer { spec: LayerSpec { name, placement, size, input: InputRegion::Full, blur: DesiredBlur::None }, attach: Persistent::save(&ctx, attach) });
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
            if let Some(layer) = layers.get_mut(&id) {
                if layer.spec.input != input { layer.spec.input = input; shell.changed(); }
                return Ok(());
            }
            drop(layers);
            let mut popups = shell.popups.borrow_mut();
            let Some(popup) = popups.get_mut(&id) else { return Ok(()); };
            if popup.spec.input != input { popup.spec.input = input; shell.changed(); }
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("setBlurRegion", Func::from(move |ctx: Ctx<'js>, id: Value<'js>, data: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            if shell.authority == Authority::Denied && shell.window_authority == Authority::Denied {
                shell.require_authority(&ctx)?;
            }
            if shell.life.get() == Lifecycle::Stopping { return Ok(()); }
            shell.require_active(&ctx)?;
            let id = layer_id(&ctx, &id)?;
            if shell.owns_lock(id) {
                return Err(Exception::throw_type(&ctx, "session lock surfaces do not support blurRegion"));
            }
            enum BlurOwner { Layer, Popup, Window }
            let target = if shell.owns(id) {
                shell.require_authority(&ctx)?;
                BlurOwner::Layer
            } else if shell.owns_popup(id) {
                shell.require_authority(&ctx)?;
                BlurOwner::Popup
            } else if shell.owns_window(id) {
                shell.require_window_authority(&ctx)?;
                BlurOwner::Window
            } else {
                return Err(Exception::throw_reference(&ctx, "shell surface is no longer owned"));
            };
            // Snapshot mounted target before running any JS getters or handle lookups with zero
            // RefCell borrows held.
            let initial_target = shell.blur_target(id);
            let blur = blur_region(&ctx, data, initial_target.as_ref())?;
            if shell.life.get() == Lifecycle::Stopping {
                return Ok(());
            }
            shell.require_active(&ctx)?;
            let current_target = shell.blur_target(id);
            if let DesiredBlur::Nodes { document, .. } = &blur {
                let same_instance = current_target
                    .as_ref()
                    .is_some_and(|live| live.document == *document && !live.bridge.is_disposed());
                if !same_instance {
                    return Err(Exception::throw_reference(
                        &ctx,
                        "shell surface mount instance is no longer active",
                    ));
                }
            }
            let updated = match target {
                BlurOwner::Layer => {
                    let mut layers = shell.layers.borrow_mut();
                    let layer = layers.get_mut(&id).ok_or_else(|| Exception::throw_reference(&ctx, "shell surface is no longer owned"))?;
                    let changed = layer.spec.blur != blur;
                    if changed { layer.spec.blur = blur.clone(); }
                    changed
                }
                BlurOwner::Popup => {
                    let mut popups = shell.popups.borrow_mut();
                    let popup = popups.get_mut(&id).ok_or_else(|| Exception::throw_reference(&ctx, "shell surface is no longer owned"))?;
                    let changed = popup.spec.blur != blur;
                    if changed { popup.spec.blur = blur.clone(); }
                    changed
                }
                BlurOwner::Window => {
                    let mut windows = shell.windows.borrow_mut();
                    let window = windows.get_mut(&id).ok_or_else(|| Exception::throw_reference(&ctx, "shell surface is no longer owned"))?;
                    let changed = window.spec.blur != blur;
                    if changed { window.spec.blur = blur.clone(); }
                    changed
                }
            };
            if let Some(live) = current_target
                && let Some(slot) = live.slot
            {
                slot.set(blur.to_registration(live.document));
            } else if updated {
                shell.changed();
            }
            Ok::<(), rquickjs::Error>(())
        }))?;
        self.install_popup(&api)?;
        self.install_window(&api)?;
        self.install_lock(&api)?;
        ctx.globals().set("__zguiShell", api.clone())?;
        Ok(api)
    }

    /// Exposes generation-wide stylesheet operations without looking up a mount region.
    pub(crate) fn install_stylesheet_api<'js>(self: &Rc<Self>, api: &Object<'js>, session: &Rc<Session>) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        let session = Rc::downgrade(session);
        let install = session.clone();
        api.set("installStylesheet", Func::from(move |ctx: Ctx<'js>, name: String, css: String| {
            let shell = owner(&weak, &ctx)?;
            shell.require_creation(&ctx)?;
            let key = shell.stylesheet_key(&ctx, &name)?;
            let session = install.upgrade().ok_or_else(|| Exception::throw_reference(&ctx, "shell session is disposed"))?;
            if let Err(diagnostic) = session.install_shell_stylesheet(key, css) {
                let message = format!("stylesheet {name} rejected: {diagnostic}");
                return Err(ctx.throw(service_error(&ctx, "E_STYLE_PROFILE", &message)?.into_value()));
            }
            Ok(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("removeStylesheet", Func::from(move |ctx: Ctx<'js>, name: String| {
            let shell = owner(&weak, &ctx)?;
            if shell.life.get() != Lifecycle::Stopping { shell.require_active(&ctx)?; }
            let key = shell.stylesheet_key(&ctx, &name)?;
            // During Session::drop, retirement itself still clears the tracked sheets.
            if let Some(session) = session.upgrade() { session.remove_shell_stylesheet(&key); }
            Ok::<(), rquickjs::Error>(())
        }))
    }

    fn stylesheet_key(&self, ctx: &Ctx<'_>, name: &str) -> rquickjs::Result<String> {
        ui_bridge::validate_stylesheet_name(ctx, name)?;
        // A staged successor can share the primary window with its predecessor.
        Ok(format!("shell:{}:{name}", self.generation))
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
            let id = allocate_lock_id(&NEXT_LOCK_ID)
                .ok_or_else(|| Exception::throw_range(&ctx, "session lock request identifiers exhausted"))?;
            let handle = Object::new(ctx.clone())?;
            handle.prop("id", Property::from(id as f64).enumerable())?;
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
            if shell.name_is_taken(&name) { return Err(duplicate_name(&ctx)); }
            let mut surfaces = shell.lock_surfaces.borrow_mut();
            if surfaces.len() >= MAX_LAYERS { return Err(Exception::throw_range(&ctx, "lock surface limit reached")); }
            if surfaces.values().any(|wanted| wanted.spec.monitor == monitor) {
                return Err(Exception::throw_type(&ctx, "output already owns a lock surface"));
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
///
/// Permits ASCII uppercase letters so shell apps can compose surface names directly from real
/// Wayland output names such as `"DP-1"` or `"HDMI-A-2"`.
pub(crate) fn valid_name(name: &str) -> bool {
    name.len() <= 128 && name.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
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
        ExclusiveZone::Ignore
    } else if zone == 0.0 {
        ExclusiveZone::Avoid
    } else if (0.0..=f64::from(i32::MAX)).contains(&zone) {
        ExclusiveZone::Reserve(CssPx(zone as f32))
    } else {
        return Err(Exception::throw_range(ctx, "exclusiveZone must be -1 or a nonnegative Wayland distance"));
    };
    let margin: Value = value.get("margin")?;
    let margin = if margin.is_undefined() { Edges::new(CssPx(0.0), CssPx(0.0), CssPx(0.0), CssPx(0.0)) } else {
        let margin = margin.into_object().ok_or_else(|| Exception::throw_type(ctx, "layer margin requires an object"))?;
        Edges::new(distance(ctx, &margin, "top", 0.0, -16384.0)?, distance(ctx, &margin, "right", 0.0, -16384.0)?,
            distance(ctx, &margin, "bottom", 0.0, -16384.0)?, distance(ctx, &margin, "left", 0.0, -16384.0)?)
    };
    let mut placement = LayerPlacement::new(layer)
        .with_anchors(anchors)
        .with_margin(margin)
        .with_exclusive_zone(exclusive_zone)
        .with_keyboard(keyboard);
    if let Some(name) = monitor {
        placement = placement.with_monitor(MonitorTarget::Name(name));
    }
    Ok((placement, Size::new(width, height)))
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

fn blur_region<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    target: Option<&AttachedBlurTarget>,
) -> rquickjs::Result<DesiredBlur> {
    if value.is_string() {
        return match string(ctx, &value, "blur region")?.as_str() {
            "none" => Ok(DesiredBlur::None),
            "full" => Ok(DesiredBlur::Full),
            _ => Err(Exception::throw_type(
                ctx,
                "blur region must be \"none\", \"full\", an array of rectangles/nodes, or { nodes, minOpacity? }",
            )),
        };
    }
    if let Some(array) = value.as_array() {
        let len = array_length(
            ctx,
            array,
            MAX_BLUR_NODES.max(MAX_BLUR_RECTS),
            "blur region entry limit exceeded",
        )?;
        if len == 0 {
            return Ok(DesiredBlur::None);
        }
        // Snapshot all elements once before validating so getters cannot alter array length or
        // observe half-validated state.
        let mut items = Vec::with_capacity(len);
        for index in 0..len {
            items.push(array.get::<Value<'js>>(index)?);
        }
        let first_is_node = Class::<NativeNode>::from_value(&items[0]).is_ok();
        if first_is_node {
            if len > MAX_BLUR_NODES as usize {
                return Err(Exception::throw_range(ctx, "blur region node limit exceeded"));
            }
            return parse_blur_nodes(ctx, items, 0.5, target);
        }
        if len > MAX_BLUR_RECTS as usize {
            return Err(Exception::throw_range(ctx, "blur region rectangle limit exceeded"));
        }
        return parse_blur_rects(ctx, items);
    }
    if Class::<NativeNode>::from_value(&value).is_ok() {
        return Err(Exception::throw_type(
            ctx,
            "blur region requires \"none\", \"full\", an array, or { nodes, minOpacity? }",
        ));
    }
    let Some(obj) = value.into_object() else {
        return Err(Exception::throw_type(
            ctx,
            "blur region requires \"none\", \"full\", an array, or { nodes, minOpacity? }",
        ));
    };
    let nodes_value: Value<'js> = obj.get("nodes")?;
    let min_opacity_value: Value<'js> = obj.get("minOpacity")?;
    let min_opacity = parse_min_opacity(ctx, &min_opacity_value)?;
    let nodes_array = nodes_value
        .into_array()
        .ok_or_else(|| Exception::throw_type(ctx, "blur region nodes must be an array"))?;
    let len = array_length(ctx, &nodes_array, MAX_BLUR_NODES, "blur region node limit exceeded")?;
    let mut items = Vec::with_capacity(len);
    for index in 0..len {
        items.push(nodes_array.get::<Value<'js>>(index)?);
    }
    if items.is_empty() {
        return Ok(DesiredBlur::None);
    }
    parse_blur_nodes(ctx, items, min_opacity, target)
}

fn parse_min_opacity(ctx: &Ctx<'_>, value: &Value<'_>) -> rquickjs::Result<f64> {
    if value.is_undefined() {
        return Ok(0.5);
    }
    let Some(n) = value.as_number() else {
        return Err(Exception::throw_type(
            ctx,
            "blur region minOpacity must be a finite number",
        ));
    };
    if !n.is_finite() {
        return Err(Exception::throw_type(
            ctx,
            "blur region minOpacity must be a finite number",
        ));
    }
    if !(0.0..=1.0).contains(&n) {
        return Err(Exception::throw_range(
            ctx,
            "blur region minOpacity must be within [0, 1]",
        ));
    }
    Ok(n)
}

fn parse_blur_nodes<'js>(
    ctx: &Ctx<'js>,
    items: Vec<Value<'js>>,
    min_opacity: f64,
    target: Option<&AttachedBlurTarget>,
) -> rquickjs::Result<DesiredBlur> {
    let mut branded = Vec::with_capacity(items.len());
    for item in items {
        let node = Class::<NativeNode>::from_value(&item).map_err(|_| {
            Exception::throw_type(ctx, "blur region node entries must be genuine native nodes")
        })?;
        branded.push(node);
    }
    let Some(target) = target else {
        return Err(Exception::throw_reference(
            ctx,
            "blur region node targets require a mounted owning surface",
        ));
    };
    let mut nodes = Vec::with_capacity(branded.len());
    for node in branded {
        let native_id = target
            .bridge
            .resolve_owned_native_node(&node)
            .map_err(|msg| Exception::throw_reference(ctx, &msg))?;
        if native_id.document() != target.document {
            return Err(Exception::throw_reference(
                ctx,
                "blur region node must belong to the owning surface",
            ));
        }
        let key = zgui_view_dom::id::to_document(native_id).ok_or_else(|| {
            Exception::throw_reference(ctx, "blur region node handle is stale")
        })?;
        nodes.push(key);
    }
    Ok(DesiredBlur::Nodes {
        document: target.document,
        nodes,
        min_opacity,
    })
}

fn parse_blur_rects<'js>(
    ctx: &Ctx<'js>,
    items: Vec<Value<'js>>,
) -> rquickjs::Result<DesiredBlur> {
    let mut rects = Vec::with_capacity(items.len());
    for item in items {
        if Class::<NativeNode>::from_value(&item).is_ok() {
            return Err(Exception::throw_type(
                ctx,
                "blur region array cannot mix rectangles and native nodes",
            ));
        }
        let rect = item
            .into_object()
            .ok_or_else(|| Exception::throw_type(ctx, "blur region entry must be a rectangle object"))?;
        let x = number(ctx, &rect.get("x")?, "blur rectangle x")?;
        let y = number(ctx, &rect.get("y")?, "blur rectangle y")?;
        let width = number(ctx, &rect.get("width")?, "blur rectangle width")?;
        let height = number(ctx, &rect.get("height")?, "blur rectangle height")?;
        if width < 0.0 || height < 0.0 {
            return Err(Exception::throw_range(ctx, "blur rectangle dimensions must be nonnegative"));
        }
        let right_sum = x + width;
        let bottom_sum = y + height;
        if !right_sum.is_finite() || !bottom_sum.is_finite() {
            return Err(Exception::throw_range(ctx, "blur rectangle edges must be finite"));
        }
        let range = f64::from(i32::MIN)..=f64::from(i32::MAX);
        let left = x.floor();
        let top = y.floor();
        let right = right_sum.ceil();
        let bottom = bottom_sum.ceil();
        if !range.contains(&left)
            || !range.contains(&top)
            || !range.contains(&right)
            || !range.contains(&bottom)
        {
            return Err(Exception::throw_range(ctx, "blur rectangle exceeds protocol coordinate range"));
        }
        let (li, ti, ri, bi) = (left as i32, top as i32, right as i32, bottom as i32);
        if ri.checked_sub(li).is_none() || bi.checked_sub(ti).is_none() {
            return Err(Exception::throw_range(ctx, "blur rectangle extent exceeds protocol range"));
        }
        if width == 0.0 || height == 0.0 {
            continue;
        }
        let (xf, yf, wf, hf) = (x as f32, y as f32, width as f32, height as f32);
        if !xf.is_finite()
            || !yf.is_finite()
            || !wf.is_finite()
            || !hf.is_finite()
            || !(xf + wf).is_finite()
            || !(yf + hf).is_finite()
        {
            return Err(Exception::throw_range(ctx, "blur rectangle exceeds finite CSS pixel range"));
        }
        rects.push(Rect::new(
            Point::new(CssPx(xf), CssPx(yf)),
            Size::new(CssPx(wf), CssPx(hf)),
        ));
    }
    Ok(if rects.is_empty() {
        DesiredBlur::None
    } else {
        DesiredBlur::Rects(rects)
    })
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
    fn popup_requires_shell_authority_even_if_javascript_catches_the_error() {
        let runtime = rquickjs::Runtime::new().expect("create QuickJS runtime");
        let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
        let waker = zgui::platform::PlatformCx::waker(&zgui_platform_headless::Headless::new());
        let shell = Shell::new(ShellConfig {
            generation: 1,
            authority: Authority::Denied,
            window_authority: Authority::Denied,
            lock_authority: Authority::Allowed,
            lock_state: SessionLockState::Locked,
            contract: ContractPolicy::Sticky,
        }, waker);
        shell.activate();
        context.with(|ctx| {
            shell.install(&ctx).expect("install shell bridge");
            for call in ["openPopup('menu', {}, () => {}, () => {})", "updatePopup(1, {})", "closePopup(1)"] {
                let code = ctx.eval::<String, _>(format!(
                    "try {{ __zguiShell.{call} }} catch (error) {{ error.code ?? 'missing code' }}"
                )).expect("read typed shell capability error");
                assert_eq!(code, "E_CAPABILITY", "{call} must require shell surface authority");
            }
        });
        assert_eq!(shell.failure().map(|failure| failure.code), Some("E_CAPABILITY"), "caught capability denial must still fail preview");
        assert_eq!(shell.callback_count(), 0, "denial must not retain popup callbacks");
    }

    #[test]
    fn popup_requests_reject_parent_cycles_without_changing_the_graph() {
        let runtime = rquickjs::Runtime::new().expect("create QuickJS runtime");
        let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
        let waker = zgui::platform::PlatformCx::waker(&zgui_platform_headless::Headless::new());
        let shell = Shell::new(ShellConfig {
            generation: 1,
            authority: Authority::Allowed,
            window_authority: Authority::Allowed,
            lock_authority: Authority::Denied,
            lock_state: SessionLockState::Locked,
            contract: ContractPolicy::Throw,
        }, waker);
        shell.activate();
        context.with(|ctx| {
            shell.install(&ctx).expect("install shell bridge");
            ctx.eval::<(), _>(r#"
                const shell = __zguiShell;
                const placement = parent => ({
                    parent, width: 10, height: 10,
                    anchorRect: { x: 0, y: 0, width: 1, height: 1 },
                    anchor: 'bottom-left', gravity: 'bottom-right', offset: { x: 0, y: 0 },
                    slide: true, flip: true, resize: false, reactive: false, grab: false,
                });
                const open = (name, parent) => shell.openPopup(name, placement(parent), () => {}, () => {});
                open('first', 'second');
            "#).expect("accept a popup whose parent is not yet declared");
            let reject_cycle = |call: &str| {
                let result = ctx.eval::<String, _>(format!(
                    "try {{ {call}; 'accepted' }} catch (error) {{ error instanceof TypeError ? error.message : String(error) }}"
                )).expect("read popup request outcome");
                assert_eq!(result, "popup parent chain contains a cycle", "{call}");
            };
            reject_cycle("open('second', 'first')");
            ctx.eval::<(), _>("const second = open('second', 'third');")
                .expect("accept an acyclic parent after rejecting a mutual cycle");
            reject_cycle("shell.updatePopup(second, placement('first'))");
            reject_cycle("open('third', 'first')");
            ctx.eval::<(), _>("const third = open('third', 'layer');")
                .expect("accept an acyclic parent after rejecting a multi-hop cycle");
            reject_cycle("shell.updatePopup(third, placement('first'))");
            ctx.eval::<(), _>("shell.updatePopup(third, { ...placement('layer'), width: 20 });")
                .expect("allow geometry updates with the original parent");
        });
        let popups = shell.popup_specs();
        let parents: Vec<_> = popups.iter().map(|(_, popup)| (popup.name.as_str(), popup.parent.as_str())).collect();
        assert_eq!(parents, [("first", "second"), ("second", "third"), ("third", "layer")]);
        assert_eq!(popups[2].1.size().width, CssPx(20.0), "apply valid geometry after rejected updates");
        shell.clear();
    }

    #[test]
    fn lock_request_id_exhaustion_does_not_wrap_or_repeat() {
        let next = AtomicU64::new(9_007_199_254_740_991);
        assert_eq!(allocate_lock_id(&next), Some(9_007_199_254_740_991), "the last safe ID remains usable");
        assert_eq!(allocate_lock_id(&next), None, "allocation beyond the safe range fails closed");
        assert_eq!(allocate_lock_id(&next), None, "exhaustion is permanent");
    }

    #[test]
    fn lock_request_ids_remain_readonly_and_increase_between_shells() {
        let runtime = rquickjs::Runtime::new().expect("create QuickJS runtime");
        let waker = zgui::platform::PlatformCx::waker(&zgui_platform_headless::Headless::new());
        let mut previous = 0.0;
        for generation in 1..=2 {
            let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
            let shell = Shell::new(ShellConfig {
                generation,
                authority: Authority::Denied,
                window_authority: Authority::Denied,
                lock_authority: Authority::Allowed,
                lock_state: SessionLockState::Locked,
                contract: ContractPolicy::Throw,
            }, waker.clone());
            shell.activate();
            let (first, second) = context.with(|ctx| {
                shell.install(&ctx).expect("install shell bridge");
                let ids = ctx.eval::<Array, _>(r#"
                    const first = __zguiShell.requestSessionLock(() => {});
                    const failure = Error('initial lock observer failed');
                    try {
                        __zguiShell.requestSessionLock(() => { throw failure; });
                        throw Error('observer failure did not propagate');
                    } catch (error) {
                        if (error !== failure) throw error;
                    }
                    const second = __zguiShell.requestSessionLock(() => {});
                    const id = first.id;
                    first.unsubscribe();
                    second.unsubscribe();
                    if (!Number.isSafeInteger(id) || !Number.isSafeInteger(second.id)
                        || Reflect.set(first, 'id', 0) || first.id !== id) {
                        throw Error('lock identity must be stable and read-only');
                    }
                    [id, second.id]
                "#).expect("read identities of two lock requests");
                (ids.get::<f64>(0).expect("first lock ID"), ids.get::<f64>(1).expect("second lock ID"))
            });
            assert!(previous < first, "lock IDs must increase across sessions: {previous}, {first}");
            assert!(second - first >= 2.0, "observer failure must consume an ID: {first}, {second}");
            previous = second;
        }
    }

    #[test]
    fn lock_requests_are_separate_from_layer_authority_and_blocked_during_cleanup() {
        let runtime = rquickjs::Runtime::new().expect("create QuickJS runtime");
        let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
        let waker = zgui::platform::PlatformCx::waker(&zgui_platform_headless::Headless::new());
        let shell = Shell::new(ShellConfig {
            generation: 1,
            authority: Authority::Denied,
            window_authority: Authority::Denied,
            lock_authority: Authority::Allowed,
            lock_state: SessionLockState::Locked,
            contract: ContractPolicy::Throw,
        }, waker);
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
                ("({exclusiveZone:-1})", ExclusiveZone::Ignore),
                ("({exclusiveZone:0})", ExclusiveZone::Avoid),
                ("({exclusiveZone:24})", ExclusiveZone::Reserve(CssPx(24.0))),
                ("({exclusiveZone:1.5})", ExclusiveZone::Reserve(CssPx(1.5))),
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

    #[test]
    fn valid_name_accepts_wayland_connector_casing_and_rejects_path_traversal() {
        for valid in ["bar-DP-1", "panel-HDMI-A-2", "DP-1", "eDP-1", "lock_1.0"] {
            assert!(valid_name(valid), "expected {valid:?} to be valid");
        }
        for invalid in ["", "-leading", ".hidden", "../escape", "bar/DP-1", "has space"] {
            assert!(!valid_name(invalid), "expected {invalid:?} to be rejected");
        }
    }

    #[test]
    fn monitor_snapshot_preserves_fractional_logical_size() {
        use zgui::geom::{DevicePx, Point};
        let native = vec![
            MonitorInfo::new(
                Point::new(DevicePx(0.0), DevicePx(0.0)),
                Size::new(DevicePx(3840.0), DevicePx(2160.0)),
                2.0,
            )
            .with_logical_size(Size::new(CssPx(2194.0), CssPx(1234.0)))
            .with_name("DP-1"),
            MonitorInfo::new(
                Point::new(DevicePx(3840.0), DevicePx(0.0)),
                Size::new(DevicePx(1920.0), DevicePx(1080.0)),
                2.0,
            )
            .with_name("HDMI-A-2"),
        ];
        let snapshot = Monitor::snapshot(&native);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].id, "DP-1");
        assert_eq!(snapshot[0].logical_width, Some(2194.0));
        assert_eq!(snapshot[0].logical_height, Some(1234.0));
        assert_eq!(snapshot[1].id, "HDMI-A-2");
        assert_eq!(snapshot[1].logical_width, Some(960.0));
        assert_eq!(snapshot[1].logical_height, Some(540.0));
    }

    #[test]
    fn window_requests_validate_and_require_shell_authority() {
        let runtime = rquickjs::Runtime::new().expect("create QuickJS runtime");
        let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
        let waker = zgui::platform::PlatformCx::waker(&zgui_platform_headless::Headless::new());
        let denied = Shell::new(ShellConfig {
            generation: 1,
            authority: Authority::Denied,
            window_authority: Authority::Denied,
            lock_authority: Authority::Denied,
            lock_state: SessionLockState::Unlocked,
            contract: ContractPolicy::Sticky,
        }, waker.clone());
        denied.activate();
        context.with(|ctx| {
            denied.install(&ctx).expect("install shell bridge");
            for call in ["openWindow('settings', {}, () => {}, () => {})", "updateWindow(1, {})", "closeWindow(1)"] {
                let code = ctx.eval::<String, _>(format!(
                    "try {{ __zguiShell.{call} }} catch (error) {{ error.code ?? 'missing code' }}"
                )).expect("read capability error");
                assert_eq!(code, "E_CAPABILITY", "{call} must require shell surface authority");
            }
        });
        assert_eq!(denied.callback_count(), 0);

        let allowed = Shell::new(ShellConfig {
            generation: 2,
            authority: Authority::Allowed,
            window_authority: Authority::Allowed,
            lock_authority: Authority::Denied,
            lock_state: SessionLockState::Unlocked,
            contract: ContractPolicy::Throw,
        }, waker);
        allowed.activate();
        context.with(|ctx| {
            allowed.install(&ctx).expect("install shell bridge");
            ctx.eval::<(), _>(r#"
                const shell = __zguiShell;
                const opts = (overrides = {}) => ({
                    title: 'Settings', appId: 'z-shell-settings',
                    width: 820, height: 600, minWidth: 640, minHeight: 480,
                    maxWidth: 1440, maxHeight: 1080, resizable: true,
                    decorations: 'no-title-bar', transparent: false,
                    ...overrides,
                });
                const id = shell.openWindow('settings', opts(), () => () => {}, () => {});
                shell.updateWindow(id, opts({ title: 'Updated', width: 900, decorations: 'none' }));
                for (const bad of [
                    () => shell.openWindow('main', opts(), () => {}, () => {}),
                    () => shell.openWindow('settings', opts(), () => {}, () => {}),
                    () => shell.openWindow('bad-range', opts({ minWidth: 900, maxWidth: 800 }), () => {}, () => {}),
                    () => shell.openWindow('bad-dec', opts({ decorations: 'invalid' }), () => {}, () => {}),
                    () => shell.updateWindow(id, opts({ appId: 'changed' })),
                    () => shell.updateWindow(id, opts({ transparent: true })),
                ]) {
                    let threw = false;
                    try { bad(); } catch { threw = true; }
                    if (!threw) throw new Error('expected validation failure');
                }
            "#).expect("validate window bridge calls");
        });
        let windows = allowed.window_specs();
        assert_eq!(windows.len(), 1);
        let spec = windows.values().next().expect("stored window spec");
        assert_eq!(spec.name, "settings");
        assert_eq!(spec.title, "Updated");
        assert_eq!(spec.app_id.as_deref(), Some("z-shell-settings"));
        assert_eq!(spec.size, Size::new(CssPx(900.0), CssPx(600.0)));
        assert_eq!(spec.min_size, Some(Size::new(CssPx(640.0), CssPx(480.0))));
        assert_eq!(spec.max_size, Some(Size::new(CssPx(1440.0), CssPx(1080.0))));
        assert_eq!(spec.decorations, zgui::platform::Decorations::None);
        allowed.clear();
    }

    #[test]
    fn popup_auto_height_parses_bounds_and_rejects_invalid_ranges() {
        let runtime = rquickjs::Runtime::new().expect("create QuickJS runtime");
        let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
        let waker = zgui::platform::PlatformCx::waker(&zgui_platform_headless::Headless::new());
        let shell = Shell::new(ShellConfig {
            generation: 1,
            authority: Authority::Allowed,
            window_authority: Authority::Allowed,
            lock_authority: Authority::Denied,
            lock_state: SessionLockState::Unlocked,
            contract: ContractPolicy::Throw,
        }, waker);
        shell.activate();
        context.with(|ctx| {
            shell.install(&ctx).expect("install shell bridge");
            ctx.eval::<(), _>(r#"
                const shell = __zguiShell;
                const base = {
                    parent: 'bar', width: 320,
                    anchorRect: { x: 0, y: 0, width: 20, height: 20 },
                    anchor: 'bottom-left', gravity: 'bottom-right', offset: { x: 0, y: 0 },
                    slide: true, flip: true, resize: false, reactive: false, grab: false,
                };
                shell.openPopup('auto-default', { ...base }, () => {}, () => {});
                shell.openPopup('auto-bounded', { ...base, height: 'auto', minHeight: 48, maxHeight: 420 }, () => {}, () => {});
                shell.openPopup('fixed', { ...base, height: 180 }, () => {}, () => {});
                for (const bad of [
                    { ...base, height: 'bogus' },
                    { ...base, height: 'auto', minHeight: 500, maxHeight: 400 },
                    { ...base, height: 'auto', minHeight: 0 },
                ]) {
                    let threw = false;
                    try { shell.openPopup('bad', bad, () => {}, () => {}); } catch { threw = true; }
                    if (!threw) throw new Error('expected popup height validation error');
                }
            "#).expect("validate popup auto-height bridge parsing");
        });
        let popups = shell.popup_specs();
        let by_name = |name: &str| popups.iter().find(|(_, p)| p.name == name).expect("popup spec").1.clone();
        let default_auto = by_name("auto-default");
        assert!(default_auto.is_auto_height());
        assert_eq!(default_auto.size(), Size::new(CssPx(320.0), CssPx(600.0)));
        let bounded_auto = by_name("auto-bounded");
        assert!(bounded_auto.is_auto_height());
        assert_eq!(bounded_auto.size(), Size::new(CssPx(320.0), CssPx(420.0)));
        let policy = bounded_auto.auto_height(zgui::platform::SurfaceId::new(1), None).expect("auto policy");
        assert_eq!(policy.min_height, CssPx(48.0));
        assert_eq!(policy.max_height, CssPx(420.0));
        let fixed = by_name("fixed");
        assert!(!fixed.is_auto_height());
        assert_eq!(fixed.size(), Size::new(CssPx(320.0), CssPx(180.0)));
        assert!(fixed.auto_height(zgui::platform::SurfaceId::new(1), None).is_none());
        shell.clear();
    }

    #[test]
    fn blur_region_enforces_authority_role_ownership_and_atomic_bounds() {
        let runtime = rquickjs::Runtime::new().expect("create QuickJS runtime");
        let waker = zgui::platform::PlatformCx::waker(&zgui_platform_headless::Headless::new());

        // 1. sessionLock alone grants zero blur authority (sticky E_CAPABILITY).
        {
            let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
            let lock_only = Shell::new(ShellConfig {
                generation: 1,
                authority: Authority::Denied,
                window_authority: Authority::Denied,
                lock_authority: Authority::Allowed,
                lock_state: SessionLockState::Locked,
                contract: ContractPolicy::Sticky,
            }, waker.clone());
            lock_only.activate();
            context.with(|ctx| {
                lock_only.install(&ctx).expect("install shell bridge");
                let code = ctx.eval::<String, _>(r#"
                    const lockId = __zguiShell.openLock('lock-dp1', 'DP-1', () => {});
                    try { __zguiShell.setBlurRegion(lockId, 'full'); 'accepted' }
                    catch (error) { error.code ?? error.name }
                "#).expect("eval lock-only blur denial");
                assert_eq!(code, "E_CAPABILITY", "sessionLock alone must never grant blur authority");
            });
            assert_eq!(lock_only.failure().map(|f| f.code), Some("E_CAPABILITY"));
            lock_only.clear();
        }

        // 2. Window-only authority allows owned windows, rejects lock & foreign/closed IDs.
        {
            let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
            let window_only = Shell::new(ShellConfig {
                generation: 2,
                authority: Authority::Denied,
                window_authority: Authority::Allowed,
                lock_authority: Authority::Allowed,
                lock_state: SessionLockState::Locked,
                contract: ContractPolicy::Throw,
            }, waker.clone());
            window_only.activate();
            context.with(|ctx| {
                window_only.install(&ctx).expect("install shell bridge");
                ctx.eval::<(), _>(r#"
                    const winId = __zguiShell.openWindow('aux', {
                        title: 'Aux', width: 320, height: 240, transparent: true,
                    }, () => {}, () => {});
                    const lockId = __zguiShell.openLock('lock-dp1', 'DP-1', () => {});
                    __zguiShell.setBlurRegion(winId, 'full');
                    __zguiShell.updateWindow(winId, {
                        title: 'Aux Renamed', width: 360, height: 240, transparent: true,
                    });
                    const expectErr = (fn, name) => {
                        try { fn(); throw new Error('expected ' + name); }
                        catch (error) {
                            if (error.name !== name) throw new Error('expected ' + name + ', got ' + error.name + ': ' + error.message);
                        }
                    };
                    expectErr(() => __zguiShell.setBlurRegion(lockId, 'full'), 'TypeError');
                    expectErr(() => __zguiShell.setBlurRegion(999, 'full'), 'ReferenceError');
                    __zguiShell.closeWindow(winId);
                    expectErr(() => __zguiShell.setBlurRegion(winId, 'none'), 'ReferenceError');
                "#).expect("window-only authority checks");
            });
            window_only.clear();
        }

        // 3. Full authority: lock rejection, persistence across updates, bounded atomic parser, retired owner.
        {
            let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
            let shell = Shell::new(ShellConfig {
                generation: 3,
                authority: Authority::Allowed,
                window_authority: Authority::Allowed,
                lock_authority: Authority::Allowed,
                lock_state: SessionLockState::Locked,
                contract: ContractPolicy::Throw,
            }, waker);
            shell.activate();
            context.with(|ctx| {
                shell.install(&ctx).expect("install shell bridge");
                ctx.eval::<(), _>(r#"
                    const s = __zguiShell;
                    const layerId = s.openLayer('bar', { layer: 'top', anchor: ['top', 'left', 'right'], width: 0, height: 40 }, () => {});
                    const popupId = s.openPopup('pop', {
                        parent: 'bar', width: 200, height: 120,
                        anchorRect: { x: 0, y: 0, width: 20, height: 20 },
                        anchor: 'bottom-left', gravity: 'bottom-right', offset: { x: 0, y: 0 },
                        slide: true, flip: true, resize: false, reactive: false, grab: false,
                    }, () => {}, () => {});
                    const winId = s.openWindow('win', { title: 'Win', width: 400, height: 300, transparent: true }, () => {}, () => {});
                    const lockId = s.openLock('lock-main', 'DP-1', () => {});

                    // Valid updates + zero-area skipping + persistence across unrelated updates.
                    s.setBlurRegion(layerId, [
                        { x: 10.5, y: 4, width: 100, height: 32 },
                        { x: 0, y: 0, width: 0, height: 20 },
                    ]);
                    s.updateLayer(layerId, { layer: 'overlay', anchor: ['top', 'left', 'right'], width: 0, height: 48 });
                    s.setBlurRegion(popupId, 'full');
                    s.updatePopup(popupId, {
                        parent: 'bar', width: 240, height: 120,
                        anchorRect: { x: 0, y: 0, width: 20, height: 20 },
                        anchor: 'bottom-left', gravity: 'bottom-right', offset: { x: 0, y: 0 },
                        slide: true, flip: true, resize: false, reactive: false, grab: false,
                    });
                    s.setBlurRegion(winId, [{ x: 0, y: 0, width: 0, height: 0 }]);

                    // Lock surface rejected even with all authorities allowed.
                    let lockRejected = false;
                    try { s.setBlurRegion(lockId, 'full'); } catch (e) { lockRejected = e instanceof TypeError; }
                    if (!lockRejected) throw new Error('lock surface must reject blurRegion');

                    // Atomic rejection: invalid inputs must NOT mutate layerId's stored rect!
                    const tooMany = Array.from({ length: 257 }, (_, i) => ({ x: i, y: 0, width: 1, height: 1 }));
                    const badInputs = [
                        'empty',
                        42,
                        tooMany,
                        [{ x: 1, y: 1, width: 10, height: 10 }, { x: NaN, y: 0, width: 10, height: 10 }],
                        [{ x: Infinity, y: 0, width: 10, height: 10 }],
                        [{ x: 0, y: 0, width: -1, height: 10 }],
                        [{ x: 1e308, y: 0, width: 1e308, height: 10 }],
                        [{ x: 2147483640, y: 0, width: 100, height: 10 }],
                        [{ x: -2000000000, y: 0, width: 3000000000, height: 10 }],
                        ['not-a-rect'],
                    ];
                    for (const bad of badInputs) {
                        let threw = false;
                        try { s.setBlurRegion(layerId, bad); } catch { threw = true; }
                        if (!threw) throw new Error('expected invalid blur region rejection');
                    }

                    // Single length snapshot: mutating array during getter read cannot expand beyond snapshot.
                    const sneaky = [{ x: 1, y: 2, width: 3, height: 4 }];
                    Object.defineProperty(sneaky, '0', {
                        get() {
                            if (sneaky.length < 20) sneaky.push({ x: 9, y: 9, width: 9, height: 9 });
                            return { x: 1, y: 2, width: 3, height: 4 };
                        },
                    });
                    s.setBlurRegion(winId, sneaky);
                "#).expect("run host boundary blur tests");
            });

            let layer_spec = shell.specs().into_values().next().expect("layer spec");
            assert_eq!(
                layer_spec.blur,
                DesiredBlur::Rects(vec![Rect::new(Point::new(CssPx(10.5), CssPx(4.0)), Size::new(CssPx(100.0), CssPx(32.0)))]),
                "atomic rejection and updateLayer must preserve prior valid blur"
            );
            let popup_spec = shell.popup_specs().pop_front().expect("popup spec").1;
            assert_eq!(popup_spec.blur, DesiredBlur::Full, "updatePopup must preserve blur");
            let win_spec = shell.window_specs().into_values().next().expect("window spec");
            assert_eq!(
                win_spec.blur,
                DesiredBlur::Rects(vec![Rect::new(Point::new(CssPx(1.0), CssPx(2.0)), Size::new(CssPx(3.0), CssPx(4.0)))]),
                "single length snapshot must read only the initial length"
            );

            // Retired generation rejects setBlurRegion cleanly.
            shell.clear();
            drop(shell);
            context.with(|ctx| {
                let err = ctx.eval::<String, _>(
                    "try { __zguiShell.setBlurRegion(1, 'full'); 'ok' } catch (e) { e.name }"
                ).expect("eval retired shell blur");
                assert_eq!(err, "ReferenceError", "retired shell generation must reject setBlurRegion");
            });
        }
    }

    #[test]
    fn popup_geometry_insets_validates_edges_and_expanded_extent_atomically() {
        let runtime = rquickjs::Runtime::new().expect("create QuickJS runtime");
        let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
        let waker = zgui::platform::PlatformCx::waker(&zgui_platform_headless::Headless::new());
        let shell = Shell::new(
            ShellConfig {
                generation: 1,
                authority: Authority::Allowed,
                window_authority: Authority::Allowed,
                lock_authority: Authority::Denied,
                lock_state: SessionLockState::Unlocked,
                contract: ContractPolicy::Throw,
            },
            waker,
        );
        shell.activate();
        context.with(|ctx| {
            shell.install(&ctx).expect("install shell bridge");
            ctx.eval::<(), _>(r#"
                const s = __zguiShell;
                s.openLayer('bar', { layer: 'top', anchor: ['top', 'left', 'right'], width: 0, height: 40 }, () => {});
                const base = {
                    parent: 'bar', width: 360, height: 200,
                    anchorRect: { x: 10, y: 10, width: 40, height: 20 },
                    anchor: 'bottom-left', gravity: 'bottom-right', offset: { x: 0, y: 0 },
                    slide: true, flip: true, resize: false, reactive: false, grab: false,
                };
                const popId = s.openPopup('pop', {
                    ...base,
                    geometryInsets: { top: 16, right: 28, bottom: 32, left: 12 },
                }, () => {}, () => {});

                const badInsets = [
                    12,
                    [12, 12, 12, 12],
                    '28px',
                    { top: -1, right: 0, bottom: 0, left: 0 },
                    { top: 0.5, right: 0, bottom: 0, left: 0 },
                    { top: NaN, right: 0, bottom: 0, left: 0 },
                    { top: Infinity, right: 0, bottom: 0, left: 0 },
                    { top: 0, right: 16385, bottom: 0, left: 0 },
                ];
                for (const bad of badInsets) {
                    let threw = false;
                    try { s.updatePopup(popId, { ...base, geometryInsets: bad }); } catch { threw = true; }
                    if (!threw) throw new Error('expected invalid geometryInsets rejection');
                }

                // Expanded logical extent overflow (both fixed height and auto maxHeight).
                for (const overflow of [
                    { ...base, width: 16300, geometryInsets: { top: 0, right: 50, bottom: 0, left: 50 } },
                    { ...base, height: 16300, geometryInsets: { top: 50, right: 0, bottom: 50, left: 0 } },
                    { ...base, height: 'auto', minHeight: 40, maxHeight: 16300, geometryInsets: { top: 50, right: 0, bottom: 50, left: 0 } },
                ]) {
                    let threw = false;
                    try { s.updatePopup(popId, overflow); } catch (e) { threw = e instanceof RangeError; }
                    if (!threw) throw new Error('expected RangeError on expanded extent overflow');
                }
            "#).expect("run popup geometryInsets validation tests");
        });

        let popup_spec = shell.popup_specs().pop_front().expect("popup spec").1;
        assert_eq!(
            popup_spec.geometry_insets(),
            Edges::new(CssPx(16.0), CssPx(28.0), CssPx(32.0), CssPx(12.0)),
            "invalid updates must throw atomically without mutating live popup geometryInsets"
        );
        shell.clear();
    }

    #[test]
    fn blur_region_nodes_enforce_document_ownership_atomicity_limits_and_recreation() {
        let runtime = rquickjs::Runtime::new().expect("create QuickJS runtime");
        let context = rquickjs::Context::full(&runtime).expect("create QuickJS context");
        let headless = zgui_platform_headless::Headless::new();
        let waker = zgui::platform::PlatformCx::waker(&headless);
        let shell = Shell::new(
            ShellConfig {
                generation: 1,
                authority: Authority::Allowed,
                window_authority: Authority::Allowed,
                lock_authority: Authority::Denied,
                lock_state: SessionLockState::Unlocked,
                contract: ContractPolicy::Throw,
            },
            waker.clone(),
        );
        shell.activate();

        // Build two distinct mounted documents + bridges + live BlurSlots.
        let make_region = |doc_id: u16, generation: u64| {
            let doc_id = zgui_dom::DocumentId::new(doc_id).expect("nonzero document id");
            let doc = zgui_dom::Document::with_id(doc_id);
            let backend = Rc::new(zgui_view_dom::DocumentDom::new(Rc::new(RefCell::new(doc))));
            let dom = zgui::view::DomHandle::from_rc(backend.clone());
            let root = dom.create_element(zgui::view::ElementName::new("box"));
            dom.insert(backend.root_node(), root, None);
            let host = zgui::view::HostHandle::from_rc(Rc::new(zgui::view::stub::StubHost::default()));
            let registry = ui_bridge::registry::NodeRegistry::new(dom, root, 1, generation, 100_000);
            let bridge = UiBridge::new(registry, host, ui_bridge::HostPort::queued());
            let gate = Arc::new(zgui::runtime::FrameGate::new());
            let rw = Arc::new(zgui::runtime::RuntimeWaker::new(waker.clone(), gate));
            let slot = Rc::new(BlurSlot::new(rw));
            (backend.document_id(), bridge, slot)
        };

        let (doc_a, bridge_a, slot_a) = make_region(10, 1);
        let (doc_b, bridge_b, slot_b) = make_region(11, 1);

        context.with(|ctx| {
            shell.install(&ctx).expect("install shell");
            let node_a = bridge_a.root(&ctx).expect("node A");
            let node_b = bridge_b.root(&ctx).expect("node B");
            ctx.globals().set("nodeA", node_a).expect("set nodeA");
            ctx.globals().set("nodeB", node_b).expect("set nodeB");

            ctx.eval::<(), _>(
                r#"
                const a = __zguiShell.openLayer('bar-a', { width: 200, height: 40 }, () => {});
                const b = __zguiShell.openLayer('bar-b', { width: 200, height: 40 }, () => {});
                globalThis.layerA = a;
                globalThis.layerB = b;
                "#,
            )
            .expect("open layers");
            let layer_a: u32 = ctx.globals().get("layerA").expect("layerA");
            let layer_b: u32 = ctx.globals().get("layerB").expect("layerB");

            // Before mount: fixed/full/none work, while node mode rejects with ReferenceError.
            ctx.eval::<(), _>(
                r#"
                __zguiShell.setBlurRegion(layerA, 'full');
                let unmountedErr = null;
                try { __zguiShell.setBlurRegion(layerA, [nodeA]); } catch (e) { unmountedErr = e.name; }
                if (unmountedErr !== 'ReferenceError') {
                    throw new Error('expected ReferenceError before mount, got ' + unmountedErr);
                }
                "#,
            )
            .expect("pre-mount checks");

            // Attach mounted instances for layerA and layerB.
            shell.attach_blur_target(layer_a, doc_a, bridge_a.clone(), Some(Rc::clone(&slot_a)));
            shell.attach_blur_target(layer_b, doc_b, bridge_b.clone(), Some(Rc::clone(&slot_b)));

            // Register valid node blur on layerA via both [node] and { nodes, minOpacity }.
            ctx.eval::<(), _>(
                r#"
                __zguiShell.setBlurRegion(layerA, [nodeA]);
                __zguiShell.setBlurRegion(layerA, { nodes: [nodeA], minOpacity: 0.35 });
                "#,
            )
            .expect("register node blur");

            let (reg_before, ver_before) = slot_a.snapshot();
            assert!(
                matches!(&reg_before, BlurRegistration::Nodes { nodes, min_opacity } if nodes.len() == 1 && (*min_opacity - 0.35).abs() < 1e-12),
                "live BlurSlot must reflect valid node registration, got {reg_before:?}"
            );

            // Atomic rejections: mixed arrays, forged node, foreign node last, bad minOpacity
            // (including on { nodes: [] }), and > MAX_BLUR_NODES must throw without mutating
            // live BlurSlot or stored DesiredBlur.
            ctx.eval::<(), _>(
                r#"
                const expectErr = (fn, expected, label) => {
                    let actual = 'NONE';
                    try { fn(); } catch (e) { actual = e.name; }
                    if (actual !== expected) throw new Error(`${label}: expected ${expected}, got ${actual}`);
                };
                const rect = { x: 0, y: 0, width: 10, height: 10 };
                expectErr(() => __zguiShell.setBlurRegion(layerA, [nodeA, rect]), 'TypeError', 'mixed [node, rect]');
                expectErr(() => __zguiShell.setBlurRegion(layerA, [rect, nodeA]), 'TypeError', 'mixed [rect, node]');
                expectErr(() => __zguiShell.setBlurRegion(layerA, { nodes: [{}] }), 'TypeError', 'forged node');
                expectErr(() => __zguiShell.setBlurRegion(layerA, [nodeA, nodeB]), 'ReferenceError', 'foreign node last');
                expectErr(() => __zguiShell.setBlurRegion(layerA, { nodes: [nodeA, nodeB], minOpacity: 0.5 }), 'ReferenceError', 'foreign node in object');
                for (const bad of [null, '0.5', NaN, Infinity, -Infinity]) {
                    expectErr(() => __zguiShell.setBlurRegion(layerA, { nodes: [], minOpacity: bad }), 'TypeError', `bad minOpacity ${bad}`);
                }
                for (const oor of [-0.01, 1.01]) {
                    expectErr(() => __zguiShell.setBlurRegion(layerA, { nodes: [], minOpacity: oor }), 'RangeError', `oor minOpacity ${oor}`);
                }
                // > 256 nodes (up to 1024) is accepted (independent of 256 output rect cap),
                // while 1025 nodes throws RangeError.
                const many = Array.from({ length: 300 }, () => nodeB);
                __zguiShell.setBlurRegion(layerB, { nodes: many, minOpacity: 0.2 });
                const tooMany = Array.from({ length: 1025 }, () => nodeA);
                expectErr(() => __zguiShell.setBlurRegion(layerA, { nodes: tooMany }), 'RangeError', 'over MAX_BLUR_NODES');
                "#,
            )
            .expect("atomic rejection and node limit checks");

            let (reg_after, ver_after) = slot_a.snapshot();
            assert_eq!(reg_after, reg_before, "rejected calls must preserve live BlurSlot");
            assert_eq!(ver_after, ver_before, "rejected calls must not bump intent version");

            // Named-layer recreation reusing the same shell request ID with a new DocumentId:
            // DesiredBlur::Nodes bound to doc_a must resolve to BlurRegistration::None on doc_c!
            let spec_a = shell.specs().remove(&layer_a).expect("layer A spec");
            let doc_c = DocumentId::new(99).expect("nonzero document id");
            assert_eq!(
                spec_a.blur.to_registration(doc_c),
                BlurRegistration::None,
                "stale DocumentId node blur must never carry onto a recreated mount instance"
            );
            assert_eq!(
                spec_a.blur.to_registration(doc_a),
                reg_before,
                "matching DocumentId preserves node registration"
            );
        });

        shell.clear();
    }
}
