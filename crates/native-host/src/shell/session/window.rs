//! Decode bounded shell toplevel window requests independently of any live native surface.
use rquickjs::{Ctx, Exception, Function, Object, Persistent, Value, function::Func};
use std::{collections::BTreeMap, ops::RangeInclusive, rc::Rc};
use zgui::{geom::{Css, CssPx, Size}, platform::Decorations};

use super::{Lifecycle, MAX_LAYERS, Shell, layer_id, number, owner, popup::duplicate_name, string, valid_name};

const MAX_TITLE_BYTES: usize = 4096;
const MAX_APP_ID_BYTES: usize = 256;
const EXTENT_RANGE: RangeInclusive<f64> = 1.0..=16_384.0;

/// A named compositor-managed toplevel window owned by a shell session.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ShellWindowSpec {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) app_id: Option<String>,
    pub(crate) size: Size<CssPx, Css>,
    pub(crate) min_size: Option<Size<CssPx, Css>>,
    pub(crate) max_size: Option<Size<CssPx, Css>>,
    pub(crate) resizable: bool,
    pub(crate) decorations: Decorations,
    pub(crate) transparent: bool,
}

pub(super) struct WantedWindow {
    pub(super) spec: ShellWindowSpec,
    pub(super) attach: Persistent<Function<'static>>,
    close: Persistent<Function<'static>>,
}

impl Shell {
    /// Snapshot desired toplevel windows in deterministic identifier order.
    #[must_use]
    pub(crate) fn window_specs(&self) -> BTreeMap<u32, ShellWindowSpec> {
        self.windows.borrow().iter().map(|(&id, window)| (id, window.spec.clone())).collect()
    }

    /// Whether the owner still wants this toplevel window.
    #[must_use]
    pub(crate) fn owns_window(&self, id: u32) -> bool { self.windows.borrow().contains_key(&id) }

    /// Invoke the window close handler only if its owning declaration still exists.
    pub(crate) fn dispatch_window_close(&self, ctx: &Ctx<'_>, id: u32) -> rquickjs::Result<()> {
        let callback = self.windows.borrow().get(&id).map(|window| window.close.clone());
        if let Some(callback) = callback { callback.restore(ctx)?.call::<_, ()>(())?; }
        Ok(())
    }

    pub(super) fn install_window<'js>(self: &Rc<Self>, api: &Object<'js>) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        api.set("openWindow", Func::from(move |ctx: Ctx<'js>, name: Value<'js>, data: Value<'js>, attach: Value<'js>, close: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_authority(&ctx)?;
            shell.require_creation(&ctx)?;
            let name = window_name(&ctx, &name)?;
            let spec = parse(&ctx, name, data)?;
            let attach = attach.into_function().ok_or_else(|| Exception::throw_type(&ctx, "window content must be a function"))?;
            let close = close.into_function().ok_or_else(|| Exception::throw_type(&ctx, "window close handler must be a function"))?;
            if shell.layers.borrow().len() + shell.popups.borrow().len() + shell.windows.borrow().len() >= MAX_LAYERS {
                return Err(Exception::throw_range(&ctx, "shell surface limit reached"));
            }
            if shell.name_is_taken(&spec.name) { return Err(duplicate_name(&ctx)); }
            let id = shell.id(&ctx)?;
            shell.windows.borrow_mut().insert(id, WantedWindow {
                spec,
                attach: Persistent::save(&ctx, attach),
                close: Persistent::save(&ctx, close),
            });
            shell.changed();
            Ok(id)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("updateWindow", Func::from(move |ctx: Ctx<'js>, id: Value<'js>, data: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_authority(&ctx)?;
            shell.require_active(&ctx)?;
            let id = layer_id(&ctx, &id)?;
            let mut windows = shell.windows.borrow_mut();
            let window = windows.get_mut(&id).ok_or_else(|| Exception::throw_reference(&ctx, "window is no longer owned"))?;
            let spec = parse(&ctx, window.spec.name.clone(), data)?;
            if window.spec.app_id != spec.app_id {
                return Err(Exception::throw_type(&ctx, "changing window appId requires a new shell surface"));
            }
            if window.spec.transparent != spec.transparent {
                return Err(Exception::throw_type(&ctx, "changing window transparency requires a new shell surface"));
            }
            if window.spec != spec { window.spec = spec; shell.changed(); }
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("closeWindow", Func::from(move |ctx: Ctx<'js>, id: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_authority(&ctx)?;
            if shell.life.get() != Lifecycle::Stopping { shell.require_active(&ctx)?; }
            if shell.windows.borrow_mut().remove(&layer_id(&ctx, &id)?).is_some() { shell.changed(); }
            Ok::<(), rquickjs::Error>(())
        }))
    }
}

fn window_name(ctx: &Ctx<'_>, value: &Value<'_>) -> rquickjs::Result<String> {
    let name = string(ctx, value, "window surface name")?;
    if name == "main" || !valid_name(&name) {
        return Err(Exception::throw_type(ctx, "invalid or reserved window surface name"));
    }
    Ok(name)
}

fn parse(ctx: &Ctx<'_>, name: String, data: Value<'_>) -> rquickjs::Result<ShellWindowSpec> {
    let data = data.into_object().ok_or_else(|| Exception::throw_type(ctx, "window options require an object"))?;
    let title = string(ctx, &data.get("title")?, "title")?;
    if title.len() > MAX_TITLE_BYTES {
        return Err(Exception::throw_range(ctx, "window title exceeds 4096 UTF-8 bytes"));
    }
    let app_id = optional_string(ctx, &data, "appId", MAX_APP_ID_BYTES)?;
    let width = required_extent(ctx, &data, "width")?;
    let height = required_extent(ctx, &data, "height")?;
    let min_width = optional_extent(ctx, &data, "minWidth")?;
    let min_height = optional_extent(ctx, &data, "minHeight")?;
    let max_width = optional_extent(ctx, &data, "maxWidth")?;
    let max_height = optional_extent(ctx, &data, "maxHeight")?;
    if let (Some(min), Some(max)) = (min_width, max_width)
        && min.0 > max.0
    {
        return Err(Exception::throw_range(ctx, "window minWidth cannot exceed maxWidth"));
    }
    if let (Some(min), Some(max)) = (min_height, max_height)
        && min.0 > max.0
    {
        return Err(Exception::throw_range(ctx, "window minHeight cannot exceed maxHeight"));
    }
    let min_size = (min_width.is_some() || min_height.is_some())
        .then(|| Size::new(min_width.unwrap_or(CssPx(0.0)), min_height.unwrap_or(CssPx(0.0))));
    let max_size = (max_width.is_some() || max_height.is_some())
        .then(|| Size::new(max_width.unwrap_or(CssPx(0.0)), max_height.unwrap_or(CssPx(0.0))));
    let resizable = optional_flag(ctx, &data, "resizable", true)?;
    let decorations = decorations(ctx, &data)?;
    let transparent = optional_flag(ctx, &data, "transparent", false)?;
    Ok(ShellWindowSpec {
        name,
        title,
        app_id,
        size: Size::new(width, height),
        min_size,
        max_size,
        resizable,
        decorations,
        transparent,
    })
}

fn required_extent(ctx: &Ctx<'_>, data: &Object<'_>, key: &str) -> rquickjs::Result<CssPx> {
    let value = number(ctx, &data.get(key)?, key)?;
    if !EXTENT_RANGE.contains(&value) {
        return Err(Exception::throw_range(ctx, &format!("window {key} out of range")));
    }
    Ok(CssPx(value as f32))
}

fn optional_extent(ctx: &Ctx<'_>, data: &Object<'_>, key: &str) -> rquickjs::Result<Option<CssPx>> {
    let value: Value = data.get(key)?;
    if value.is_undefined() || value.is_null() { return Ok(None); }
    let value = number(ctx, &value, key)?;
    if !EXTENT_RANGE.contains(&value) {
        return Err(Exception::throw_range(ctx, &format!("window {key} out of range")));
    }
    Ok(Some(CssPx(value as f32)))
}

fn optional_string(ctx: &Ctx<'_>, data: &Object<'_>, key: &str, max_bytes: usize) -> rquickjs::Result<Option<String>> {
    let value: Value = data.get(key)?;
    if value.is_undefined() || value.is_null() { return Ok(None); }
    let text = string(ctx, &value, key)?;
    if text.is_empty() || text.len() > max_bytes {
        return Err(Exception::throw_range(ctx, &format!("window {key} must be 1..={max_bytes} UTF-8 bytes")));
    }
    Ok(Some(text))
}

fn optional_flag(ctx: &Ctx<'_>, data: &Object<'_>, key: &str, default: bool) -> rquickjs::Result<bool> {
    let value: Value = data.get(key)?;
    if value.is_undefined() || value.is_null() { return Ok(default); }
    value.as_bool().ok_or_else(|| Exception::throw_type(ctx, &format!("window {key} must be a boolean")))
}

fn decorations(ctx: &Ctx<'_>, data: &Object<'_>) -> rquickjs::Result<Decorations> {
    let value: Value = data.get("decorations")?;
    if value.is_undefined() || value.is_null() { return Ok(Decorations::Full); }
    Ok(match string(ctx, &value, "decorations")?.as_str() {
        "full" => Decorations::Full,
        "none" => Decorations::None,
        "no-title-bar" => Decorations::NoTitleBar,
        _ => return Err(Exception::throw_type(ctx, "invalid window decorations")),
    })
}
