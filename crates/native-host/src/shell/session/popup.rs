//! Decode bounded popup requests independently of any live native surface.
use rquickjs::{Ctx, Exception, Function, Object, Persistent, Value, function::Func};
use std::{collections::{BTreeMap, VecDeque}, ops::RangeInclusive, rc::Rc};
use zgui::{geom::{Css, CssPx, Point, Rect, Size}, platform::{Anchor, Constrain, PopupPlacement, SurfaceId}};

use super::{Lifecycle, MAX_LAYERS, Shell, layer_id, number, owner, string, valid_name};

/// A named popup and its parent-independent configuration.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PopupSpec {
    pub(crate) name: String,
    pub(crate) parent: String,
    geometry: PopupGeometry,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PopupGeometry {
    size: Size<CssPx, Css>,
    anchor_rect: Rect<CssPx, Css>,
    anchor: Anchor,
    gravity: Anchor,
    offset: Point<CssPx, Css>,
    constrain: Constrain,
    grab: bool,
    reactive: bool,
}

impl PopupSpec {
    /// Requested extent in logical pixels.
    #[must_use]
    pub(crate) fn size(&self) -> Size<CssPx, Css> { self.geometry.size }

    /// Resolve the placement against a live parent in the same generation.
    #[must_use]
    pub(crate) fn placement(&self, parent: SurfaceId) -> PopupPlacement {
        let PopupGeometry { anchor_rect, anchor, gravity, offset, constrain, grab, reactive, .. } = self.geometry;
        PopupPlacement { parent, anchor_rect, anchor, gravity, offset, constrain, grab, reactive }
    }
}

pub(super) struct WantedPopup {
    pub(super) spec: PopupSpec,
    pub(super) attach: Persistent<Function<'static>>,
    dismiss: Persistent<Function<'static>>,
}

impl Shell {
    /// Snapshot desired popups while resolving their named live parents.
    #[must_use]
    pub(crate) fn popup_specs(&self) -> VecDeque<(u32, PopupSpec)> {
        self.popups.borrow().iter().map(|(&id, popup)| (id, popup.spec.clone())).collect()
    }

    /// Whether the owner still wants this popup, including a dormant request.
    #[must_use]
    pub(crate) fn owns_popup(&self, id: u32) -> bool { self.popups.borrow().contains_key(&id) }

    /// Invoke the dismissal handler only if its owning declaration still exists.
    pub(crate) fn dispatch_popup_dismiss(&self, ctx: &Ctx<'_>, id: u32) -> rquickjs::Result<()> {
        let callback = self.popups.borrow().get(&id).map(|popup| popup.dismiss.clone());
        if let Some(callback) = callback { callback.restore(ctx)?.call::<_, ()>(())?; }
        Ok(())
    }

    pub(super) fn install_popup<'js>(self: &Rc<Self>, api: &Object<'js>) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        api.set("openPopup", Func::from(move |ctx: Ctx<'js>, name: Value<'js>, data: Value<'js>, attach: Value<'js>, dismiss: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_authority(&ctx)?;
            shell.require_creation(&ctx)?;
            let name = popup_name(&ctx, &name)?;
            let (parent, geometry) = parse(&ctx, data)?;
            require_acyclic_parent(&ctx, &shell.popups.borrow(), &name, &parent)?;
            let spec = PopupSpec { name, parent, geometry };
            let attach = attach.into_function().ok_or_else(|| Exception::throw_type(&ctx, "popup content must be a function"))?;
            let dismiss = dismiss.into_function().ok_or_else(|| Exception::throw_type(&ctx, "popup dismissal handler must be a function"))?;
            if shell.layers.borrow().len() + shell.popups.borrow().len() >= MAX_LAYERS {
                return Err(Exception::throw_range(&ctx, "shell surface limit reached"));
            }
            if shell.name_is_taken(&spec.name) { return Err(duplicate_name(&ctx)); }
            let id = shell.id(&ctx)?;
            shell.popups.borrow_mut().insert(id, WantedPopup { spec, attach: Persistent::save(&ctx, attach), dismiss: Persistent::save(&ctx, dismiss) });
            shell.changed();
            Ok(id)
        }))?;
        let weak = Rc::downgrade(self);
        api.set("updatePopup", Func::from(move |ctx: Ctx<'js>, id: Value<'js>, data: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_authority(&ctx)?;
            shell.require_active(&ctx)?;
            let id = layer_id(&ctx, &id)?;
            let (parent, geometry) = parse(&ctx, data)?;
            let mut popups = shell.popups.borrow_mut();
            let popup = popups.get_mut(&id).ok_or_else(|| Exception::throw_reference(&ctx, "popup is no longer owned"))?;
            if popup.spec.parent != parent {
                require_acyclic_parent(&ctx, &popups, &popups[&id].spec.name, &parent)?;
                return Err(Exception::throw_type(&ctx, "changing a popup parent requires a new shell surface"));
            }
            if popup.spec.geometry != geometry { popup.spec.geometry = geometry; shell.changed(); }
            Ok::<(), rquickjs::Error>(())
        }))?;
        let weak = Rc::downgrade(self);
        api.set("closePopup", Func::from(move |ctx: Ctx<'js>, id: Value<'js>| {
            let shell = owner(&weak, &ctx)?;
            shell.require_authority(&ctx)?;
            if shell.life.get() != Lifecycle::Stopping { shell.require_active(&ctx)?; }
            if shell.popups.borrow_mut().remove(&layer_id(&ctx, &id)?).is_some() { shell.changed(); }
            Ok::<(), rquickjs::Error>(())
        }))
    }
}

fn require_acyclic_parent(ctx: &Ctx<'_>, popups: &BTreeMap<u32, WantedPopup>, name: &str, parent: &str) -> rquickjs::Result<()> {
    if name == parent { return Err(Exception::throw_type(ctx, "a popup cannot parent itself")); }
    let mut ancestor = parent;
    while let Some(popup) = popups.values().find(|popup| popup.spec.name == ancestor) {
        ancestor = &popup.spec.parent;
        // Check the name even if its declaration is still missing.
        if ancestor == name { return Err(Exception::throw_type(ctx, "popup parent chain contains a cycle")); }
    }
    Ok(())
}

pub(super) fn duplicate_name(ctx: &Ctx<'_>) -> rquickjs::Error {
    Exception::throw_type(ctx, "duplicate shell surface name")
}

fn popup_name(ctx: &Ctx<'_>, value: &Value<'_>) -> rquickjs::Result<String> {
    let name = string(ctx, value, "popup surface name")?;
    if name == "main" || !valid_name(&name) { return Err(Exception::throw_type(ctx, "invalid or reserved popup surface name")); }
    Ok(name)
}

fn parse(ctx: &Ctx<'_>, data: Value<'_>) -> rquickjs::Result<(String, PopupGeometry)> {
    let data = data.into_object().ok_or_else(|| Exception::throw_type(ctx, "popup placement requires an object"))?;
    let parent = popup_name(ctx, &data.get("parent")?)?;
    let rect: Object = data.get("anchorRect")?;
    let offset: Object = data.get("offset")?;
    let position = |value: &Object<'_>| -> rquickjs::Result<_> {
        Ok(Point::new(css(ctx, value, "x", -MAX_EXACT_CSS..=MAX_EXACT_CSS)?, css(ctx, value, "y", -MAX_EXACT_CSS..=MAX_EXACT_CSS)?))
    };
    let extent = |value: &Object<'_>, range: RangeInclusive<f64>| -> rquickjs::Result<_> {
        Ok(Size::new(css(ctx, value, "width", range.clone())?, css(ctx, value, "height", range)?))
    };
    Ok((parent, PopupGeometry {
        size: extent(&data, 1.0..=16_384.0)?,
        anchor_rect: Rect::new(position(&rect)?, extent(&rect, f64::from(f32::MIN_POSITIVE)..=MAX_EXACT_CSS)?),
        anchor: anchor(ctx, &data, "anchor")?, gravity: anchor(ctx, &data, "gravity")?,
        offset: position(&offset)?,
        constrain: Constrain { slide: flag(ctx, &data, "slide")?, flip: flag(ctx, &data, "flip")?, resize: flag(ctx, &data, "resize")? },
        grab: flag(ctx, &data, "grab")?, reactive: flag(ctx, &data, "reactive")?,
    }))
}

const MAX_EXACT_CSS: f64 = 16_777_216.0;

fn css(ctx: &Ctx<'_>, data: &Object<'_>, key: &str, range: RangeInclusive<f64>) -> rquickjs::Result<CssPx> {
    let value = number(ctx, &data.get(key)?, key)?;
    if !range.contains(&value) { return Err(Exception::throw_range(ctx, &format!("popup {key} out of range"))); }
    Ok(CssPx(value as f32))
}

fn flag(ctx: &Ctx<'_>, data: &Object<'_>, key: &str) -> rquickjs::Result<bool> {
    data.get::<_, Value>(key)?.as_bool().ok_or_else(|| Exception::throw_type(ctx, &format!("popup {key} must be a boolean")))
}

fn anchor(ctx: &Ctx<'_>, data: &Object<'_>, key: &str) -> rquickjs::Result<Anchor> {
    Ok(match string(ctx, &data.get(key)?, key)?.as_str() {
        "center" => Anchor::Center,
        "top" => Anchor::Top,
        "bottom" => Anchor::Bottom,
        "left" => Anchor::Left,
        "right" => Anchor::Right,
        "top-left" => Anchor::TopLeft,
        "top-right" => Anchor::TopRight,
        "bottom-left" => Anchor::BottomLeft,
        "bottom-right" => Anchor::BottomRight,
        _ => return Err(Exception::throw_type(ctx, &format!("invalid popup {key}"))),
    })
}
