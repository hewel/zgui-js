//! Per-surface runtime window controls, reactive state snapshots, and synchronous close guards.
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
    sync::Arc,
};

use rquickjs::{
    Ctx, Exception, Function, Object, Persistent, Value,
    function::{Func, Opt},
};
use zgui::platform::{Decorations, SurfaceId, WakeReason, Waker};

const MAX_TITLE_BYTES: usize = 4096;
const MAX_HANDLERS_PER_SESSION: usize = 64;

/// Platform-neutral state snapshot tracked for each toplevel window surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WindowState {
    pub(crate) title: String,
    pub(crate) fullscreen: bool,
    pub(crate) maximized: bool,
    pub(crate) minimized: bool,
    pub(crate) decorations: Decorations,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            title: String::new(),
            fullscreen: false,
            maximized: false,
            minimized: false,
            decorations: Decorations::Full,
        }
    }
}

/// Queued runtime window command to apply on the platform thread outside JS turns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WindowCommand {
    SetTitle(String),
    SetFullscreen(bool),
    SetMaximized(bool),
    SetMinimized(bool),
    SetDecorations(Decorations),
    BeginMoveDrag,
    Close { force: bool },
}

type HandlerMap = HashMap<SurfaceId, Vec<(u64, Persistent<Function<'static>>)>>;

/// Holds per-surface window state, commands, reactive subscribers, and close-request guards.
///
/// All `Persistent` callbacks must be dropped via [`Self::clear`] before the owning JS runtime
/// is dropped.
pub(crate) struct WindowControls {
    states: RefCell<HashMap<SurfaceId, WindowState>>,
    commands: RefCell<Vec<(SurfaceId, WindowCommand)>>,
    close_handlers: RefCell<HandlerMap>,
    state_subscribers: RefCell<HandlerMap>,
    next_id: Cell<u64>,
    waker: Arc<dyn Waker>,
}

impl WindowControls {
    pub(crate) fn new(waker: Arc<dyn Waker>) -> Rc<Self> {
        Rc::new(Self {
            states: RefCell::new(HashMap::new()),
            commands: RefCell::new(Vec::new()),
            close_handlers: RefCell::new(HashMap::new()),
            state_subscribers: RefCell::new(HashMap::new()),
            next_id: Cell::new(1),
            waker,
        })
    }

    pub(crate) fn seed_surface(&self, surface: SurfaceId, state: WindowState) {
        self.states.borrow_mut().insert(surface, state);
    }

    pub(crate) fn remove_surface(&self, surface: SurfaceId) {
        self.states.borrow_mut().remove(&surface);
        self.close_handlers.borrow_mut().remove(&surface);
        self.state_subscribers.borrow_mut().remove(&surface);
        self.commands.borrow_mut().retain(|(id, _)| *id != surface);
    }

    pub(crate) fn clear(&self) {
        self.states.borrow_mut().clear();
        self.commands.borrow_mut().clear();
        self.close_handlers.borrow_mut().clear();
        self.state_subscribers.borrow_mut().clear();
    }

    pub(crate) fn callback_count(&self) -> usize {
        let close: usize = self.close_handlers.borrow().values().map(Vec::len).sum();
        let state: usize = self.state_subscribers.borrow().values().map(Vec::len).sum();
        close + state
    }

    pub(crate) fn has_commands(&self) -> bool {
        !self.commands.borrow().is_empty()
    }

    pub(crate) fn take_commands(&self) -> Vec<(SurfaceId, WindowCommand)> {
        std::mem::take(&mut *self.commands.borrow_mut())
    }

    /// Update platform-reported fullscreen/maximized/minimized flags and notify listeners if changed.
    pub(crate) fn sync_platform_flags(
        &self,
        ctx: &Ctx<'_>,
        surface: SurfaceId,
        fullscreen: bool,
        maximized: bool,
        minimized: Option<bool>,
    ) -> rquickjs::Result<bool> {
        let updated = {
            let mut states = self.states.borrow_mut();
            let Some(state) = states.get_mut(&surface) else {
                return Ok(false);
            };
            let next_minimized = minimized.unwrap_or(state.minimized);
            if state.fullscreen == fullscreen
                && state.maximized == maximized
                && state.minimized == next_minimized
            {
                return Ok(false);
            }
            state.fullscreen = fullscreen;
            state.maximized = maximized;
            state.minimized = next_minimized;
            state.clone()
        };
        self.notify_subscribers(ctx, surface, &updated)?;
        Ok(true)
    }

    /// Invoke all registered `onCloseRequest` guards synchronously for `surface`.
    /// Returns `Ok(true)` if any guard called `event.preventDefault()`.
    pub(crate) fn dispatch_close_request(&self, ctx: &Ctx<'_>, surface: SurfaceId) -> rquickjs::Result<bool> {
        let handlers: Vec<Persistent<Function<'static>>> = self
            .close_handlers
            .borrow()
            .get(&surface)
            .map(|list| list.iter().map(|(_, cb)| cb.clone()).collect())
            .unwrap_or_default();
        if handlers.is_empty() {
            return Ok(false);
        }
        let prevented = Rc::new(Cell::new(false));
        let event = Object::new(ctx.clone())?;
        let flag = prevented.clone();
        event.set("preventDefault", Func::from(move || {
            flag.set(true);
        }))?;
        let flag_read = prevented.clone();
        event.prop(
            "defaultPrevented",
            rquickjs::object::Accessor::new_get(move || flag_read.get()),
        )?;
        for handler in handlers {
            handler.restore(ctx)?.call::<_, ()>((event.clone(),))?;
        }
        Ok(prevented.get())
    }

    fn next_handler_id(&self, ctx: &Ctx<'_>) -> rquickjs::Result<u64> {
        let id = self.next_id.get();
        let next = id
            .checked_add(1)
            .ok_or_else(|| Exception::throw_range(ctx, "window handler identifiers exhausted"))?;
        self.next_id.set(next);
        Ok(id)
    }

    fn push_command(&self, surface: SurfaceId, command: WindowCommand) {
        self.commands.borrow_mut().push((surface, command));
        self.waker.wake(WakeReason::AppWork);
    }

    fn notify_subscribers(&self, ctx: &Ctx<'_>, surface: SurfaceId, state: &WindowState) -> rquickjs::Result<()> {
        let subscribers: Vec<Persistent<Function<'static>>> = self
            .state_subscribers
            .borrow()
            .get(&surface)
            .map(|list| list.iter().map(|(_, cb)| cb.clone()).collect())
            .unwrap_or_default();
        if subscribers.is_empty() {
            return Ok(());
        }
        for subscriber in subscribers {
            let obj = state_to_js(ctx, state)?;
            subscriber.restore(ctx)?.call::<_, ()>((obj,))?;
        }
        Ok(())
    }

    /// Attach per-region window control methods onto a region's `__zgui` bridge object.
    pub(crate) fn install_region_window_api<'js>(
        self: &Rc<Self>,
        api: &Object<'js>,
        surface: SurfaceId,
        is_window: bool,
    ) -> rquickjs::Result<()> {
        let require_window = move |ctx: &Ctx<'js>| -> rquickjs::Result<()> {
            if is_window {
                Ok(())
            } else {
                Err(Exception::throw_type(
                    ctx,
                    "window controls are only available inside a toplevel window",
                ))
            }
        };

        let weak = Rc::downgrade(self);
        api.set("setWindowTitle", Func::from(move |ctx: Ctx<'js>, title: Value<'js>| {
            require_window(&ctx)?;
            let controls = upgrade(&weak, &ctx)?;
            let Some(js_str) = title.into_string() else {
                return Err(Exception::throw_type(&ctx, "window title must be a string"));
            };
            let title = js_str.to_string()?;
            if title.len() > MAX_TITLE_BYTES {
                return Err(Exception::throw_range(&ctx, "window title exceeds 4096 UTF-8 bytes"));
            }
            let updated = {
                let mut states = controls.states.borrow_mut();
                let state = states.entry(surface).or_default();
                if state.title == title {
                    None
                } else {
                    state.title = title.clone();
                    Some(state.clone())
                }
            };
            controls.push_command(surface, WindowCommand::SetTitle(title));
            if let Some(state) = updated {
                controls.notify_subscribers(&ctx, surface, &state)?;
            }
            Ok::<(), rquickjs::Error>(())
        }))?;

        let weak = Rc::downgrade(self);
        api.set("setWindowFullscreen", Func::from(move |ctx: Ctx<'js>, value: Value<'js>| {
            require_window(&ctx)?;
            let controls = upgrade(&weak, &ctx)?;
            let fullscreen = value
                .as_bool()
                .ok_or_else(|| Exception::throw_type(&ctx, "fullscreen must be a boolean"))?;
            let updated = {
                let mut states = controls.states.borrow_mut();
                let state = states.entry(surface).or_default();
                if state.fullscreen == fullscreen {
                    None
                } else {
                    state.fullscreen = fullscreen;
                    Some(state.clone())
                }
            };
            controls.push_command(surface, WindowCommand::SetFullscreen(fullscreen));
            if let Some(state) = updated {
                controls.notify_subscribers(&ctx, surface, &state)?;
            }
            Ok::<(), rquickjs::Error>(())
        }))?;

        let weak = Rc::downgrade(self);
        api.set("setWindowMaximized", Func::from(move |ctx: Ctx<'js>, value: Value<'js>| {
            require_window(&ctx)?;
            let controls = upgrade(&weak, &ctx)?;
            let maximized = value
                .as_bool()
                .ok_or_else(|| Exception::throw_type(&ctx, "maximized must be a boolean"))?;
            let updated = {
                let mut states = controls.states.borrow_mut();
                let state = states.entry(surface).or_default();
                if state.maximized == maximized {
                    None
                } else {
                    state.maximized = maximized;
                    Some(state.clone())
                }
            };
            controls.push_command(surface, WindowCommand::SetMaximized(maximized));
            if let Some(state) = updated {
                controls.notify_subscribers(&ctx, surface, &state)?;
            }
            Ok::<(), rquickjs::Error>(())
        }))?;

        let weak = Rc::downgrade(self);
        api.set("setWindowMinimized", Func::from(move |ctx: Ctx<'js>, value: Value<'js>| {
            require_window(&ctx)?;
            let controls = upgrade(&weak, &ctx)?;
            let minimized = value
                .as_bool()
                .ok_or_else(|| Exception::throw_type(&ctx, "minimized must be a boolean"))?;
            let updated = {
                let mut states = controls.states.borrow_mut();
                let state = states.entry(surface).or_default();
                if state.minimized == minimized {
                    None
                } else {
                    state.minimized = minimized;
                    Some(state.clone())
                }
            };
            controls.push_command(surface, WindowCommand::SetMinimized(minimized));
            if let Some(state) = updated {
                controls.notify_subscribers(&ctx, surface, &state)?;
            }
            Ok::<(), rquickjs::Error>(())
        }))?;

        let weak = Rc::downgrade(self);
        api.set("setWindowDecorations", Func::from(move |ctx: Ctx<'js>, value: Value<'js>| {
            require_window(&ctx)?;
            let controls = upgrade(&weak, &ctx)?;
            let Some(js_str) = value.into_string() else {
                return Err(Exception::throw_type(&ctx, "window decorations must be a string"));
            };
            let decorations = parse_decorations(&ctx, &js_str.to_string()?)?;
            let updated = {
                let mut states = controls.states.borrow_mut();
                let state = states.entry(surface).or_default();
                if state.decorations == decorations {
                    None
                } else {
                    state.decorations = decorations;
                    Some(state.clone())
                }
            };
            controls.push_command(surface, WindowCommand::SetDecorations(decorations));
            if let Some(state) = updated {
                controls.notify_subscribers(&ctx, surface, &state)?;
            }
            Ok::<(), rquickjs::Error>(())
        }))?;

        let weak = Rc::downgrade(self);
        api.set("beginWindowMoveDrag", Func::from(move |ctx: Ctx<'js>| {
            require_window(&ctx)?;
            let controls = upgrade(&weak, &ctx)?;
            controls.push_command(surface, WindowCommand::BeginMoveDrag);
            Ok::<(), rquickjs::Error>(())
        }))?;

        let weak = Rc::downgrade(self);
        api.set("closeWindow", Func::from(move |ctx: Ctx<'js>, force: Opt<Value<'js>>| {
            require_window(&ctx)?;
            let controls = upgrade(&weak, &ctx)?;
            let force = match force.0 {
                None => false,
                Some(ref val) if val.is_undefined() || val.is_null() => false,
                Some(ref val) => val
                    .as_bool()
                    .ok_or_else(|| Exception::throw_type(&ctx, "closeWindow force option must be a boolean"))?,
            };
            controls.push_command(surface, WindowCommand::Close { force });
            Ok::<(), rquickjs::Error>(())
        }))?;

        let weak = Rc::downgrade(self);
        api.set("getWindowState", Func::from(move |ctx: Ctx<'js>| -> rquickjs::Result<Object<'js>> {
            require_window(&ctx)?;
            let controls = upgrade(&weak, &ctx)?;
            let snapshot = controls
                .states
                .borrow()
                .get(&surface)
                .cloned()
                .unwrap_or_default();
            state_to_js(&ctx, &snapshot)
        }))?;

        let weak = Rc::downgrade(self);
        api.set("subscribeWindowState", Func::from(move |ctx: Ctx<'js>, callback: Function<'js>| {
            require_window(&ctx)?;
            let controls = upgrade(&weak, &ctx)?;
            if controls.callback_count() >= MAX_HANDLERS_PER_SESSION {
                return Err(Exception::throw_range(&ctx, "window handler limit reached"));
            }
            let id = controls.next_handler_id(&ctx)?;
            controls
                .state_subscribers
                .borrow_mut()
                .entry(surface)
                .or_default()
                .push((id, Persistent::save(&ctx, callback)));
            let weak = weak.clone();
            Function::new(ctx, move || {
                if let Some(controls) = weak.upgrade()
                    && let Some(list) = controls.state_subscribers.borrow_mut().get_mut(&surface)
                {
                    list.retain(|(entry_id, _)| *entry_id != id);
                }
            })
        }))?;

        let weak = Rc::downgrade(self);
        api.set("onCloseRequest", Func::from(move |ctx: Ctx<'js>, callback: Function<'js>| {
            require_window(&ctx)?;
            let controls = upgrade(&weak, &ctx)?;
            if controls.callback_count() >= MAX_HANDLERS_PER_SESSION {
                return Err(Exception::throw_range(&ctx, "window handler limit reached"));
            }
            let id = controls.next_handler_id(&ctx)?;
            controls
                .close_handlers
                .borrow_mut()
                .entry(surface)
                .or_default()
                .push((id, Persistent::save(&ctx, callback)));
            let weak = weak.clone();
            Function::new(ctx, move || {
                if let Some(controls) = weak.upgrade()
                    && let Some(list) = controls.close_handlers.borrow_mut().get_mut(&surface)
                {
                    list.retain(|(entry_id, _)| *entry_id != id);
                }
            })
        }))?;

        Ok(())
    }
}

pub(crate) fn decorations_str(decorations: Decorations) -> &'static str {
    match decorations {
        Decorations::Full => "full",
        Decorations::None => "none",
        Decorations::NoTitleBar => "no-title-bar",
        _ => "full",
    }
}

fn parse_decorations(ctx: &Ctx<'_>, raw: &str) -> rquickjs::Result<Decorations> {
    match raw {
        "full" => Ok(Decorations::Full),
        "none" => Ok(Decorations::None),
        "no-title-bar" => Ok(Decorations::NoTitleBar),
        _ => Err(Exception::throw_type(ctx, "invalid window decorations")),
    }
}

fn state_to_js<'js>(ctx: &Ctx<'js>, state: &WindowState) -> rquickjs::Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.set("title", state.title.as_str())?;
    obj.set("fullscreen", state.fullscreen)?;
    obj.set("maximized", state.maximized)?;
    obj.set("minimized", state.minimized)?;
    obj.set("decorations", decorations_str(state.decorations))?;
    Ok(obj)
}

fn upgrade(weak: &Weak<WindowControls>, ctx: &Ctx<'_>) -> rquickjs::Result<Rc<WindowControls>> {
    weak.upgrade()
        .ok_or_else(|| Exception::throw_reference(ctx, "native session no longer exists"))
}
