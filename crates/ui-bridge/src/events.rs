//! Native listener roots and owned snapshots. No native borrow crosses into JavaScript.
//!
//! `onFocus`/`onBlur` deliberately subscribe to native `FocusIn`/`FocusOut`, so their
//! snapshot types are `focusin`/`focusout` and they participate in native bubbling.
//! Timestamps are monotonic milliseconds, positions are window-relative CSS pixels,
//! pointer IDs are lossless decimal strings, and selection offsets are UTF-8 bytes.

use std::{cell::Cell, collections::{HashMap, HashSet}, rc::Rc};

use rquickjs::{function::Func, Array, Ctx, Exception, Function, IntoJs, Object, Persistent, Value};
use zgui::{
    view::{DomHandle, EventCx, ListenerId, NodeId},
    vocab::{EventKind, FocusCause, Key, KeyLocation, ListenerOptions, Modifiers, NamedKey, Payload,
        Phase, PhysicalKey, PointerButton, PointerKind, PointerSample, ScrollDelta, ScrollPhase,
        Timestamp, ValueChange},
};

use crate::registry::Handle;

pub(crate) type Dispatcher = dyn Fn(u64, &mut EventCx<'_>);

struct Listener {
    native: NodeId,
    registration: ListenerId,
    callback: u64,
}

/// Persistent roots are held here, never in closures owned by the native tree.
/// Drop this registry before dropping the owning JavaScript runtime.
pub(crate) struct Events {
    dom: DomHandle,
    listeners: HashMap<(u64, EventKind, bool), Listener>,
    callbacks: HashMap<u64, Persistent<Function<'static>>>,
    dismiss_callbacks: HashSet<u64>,
    next_callback: u64,
}

impl Events {
    pub(crate) fn new(dom: DomHandle) -> Self {
        Self { dom, listeners: HashMap::new(), callbacks: HashMap::new(),
            dismiss_callbacks: HashSet::new(), next_callback: 1 }
    }

    fn reserve(&mut self, ctx: &Ctx<'_>) -> rquickjs::Result<u64> {
        let id = self.next_callback;
        self.next_callback = id.checked_add(1).ok_or_else(|| Exception::throw_range(ctx, "native callback identifiers exhausted"))?;
        Ok(id)
    }

    pub(crate) fn set<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        owner: Handle,
        native: NodeId,
        name: &str,
        value: Value<'js>,
        dispatcher: Rc<Dispatcher>,
    ) -> rquickjs::Result<()> {
        let (kind, capture) = event_property(ctx, name)?;
        let callback = if value.is_null() || value.is_undefined() {
            None
        } else {
            Some(value.into_function().ok_or_else(|| Exception::throw_type(
                ctx, "native event properties accept only Function, null, or undefined; listener options are unsupported",
            ))?)
        };
        // Validate everything before replacing a working listener.
        let id = if callback.is_some() { Some(self.reserve(ctx)?) } else { None };
        let key = (owner.token(), kind, capture);
        if let Some(old) = self.listeners.remove(&key) {
            self.dom.remove_listener(old.native, old.registration);
            self.callbacks.remove(&old.callback);
        }
        if let (Some(callback), Some(id)) = (callback, id) {
            self.callbacks.insert(id, Persistent::save(ctx, callback));
            let registration = self.dom.add_listener(
                native, kind, ListenerOptions { capture, ..ListenerOptions::DEFAULT },
                Rc::new(move |event| dispatcher(id, event)),
            );
            self.listeners.insert(key, Listener { native, registration, callback: id });
        }
        Ok(())
    }

    pub(crate) fn callback(&self, id: u64) -> Option<Persistent<Function<'static>>> {
        self.callbacks.get(&id).cloned()
    }

    pub(crate) fn register_dismiss<'js>(&mut self, ctx: &Ctx<'js>, callback: Function<'js>) -> rquickjs::Result<u64> {
        let id = self.reserve(ctx)?;
        self.callbacks.insert(id, Persistent::save(ctx, callback));
        self.dismiss_callbacks.insert(id);
        Ok(id)
    }

    pub(crate) fn is_dismiss(&self, id: u64) -> bool { self.dismiss_callbacks.contains(&id) }

    pub(crate) fn remove_callback(&mut self, id: u64) {
        self.callbacks.remove(&id);
        self.dismiss_callbacks.remove(&id);
    }

    /// Removes every listener belonging to one reclaimed bridge node token.
    pub(crate) fn remove(&mut self, token: u64) {
        self.listeners.retain(|(owner, _, _), listener| {
            if *owner != token { return true; }
            self.dom.remove_listener(listener.native, listener.registration);
            self.callbacks.remove(&listener.callback);
            self.dismiss_callbacks.remove(&listener.callback);
            false
        });
    }

    pub(crate) fn clear(&mut self) {
        for (_, listener) in self.listeners.drain() {
            self.dom.remove_listener(listener.native, listener.registration);
        }
        self.callbacks.clear();
        self.dismiss_callbacks.clear();
    }

    pub(crate) fn len(&self) -> usize { self.callbacks.len() }
}

impl Drop for Events {
    fn drop(&mut self) { self.clear(); }
}

fn event_property(ctx: &Ctx<'_>, name: &str) -> rquickjs::Result<(EventKind, bool)> {
    let (name, capture) = name.strip_suffix("Capture").map_or((name, false), |name| (name, true));
    let kind = match name {
        "onClick" => EventKind::Click,
        "onPointerDown" => EventKind::PointerDown,
        "onPointerMove" => EventKind::PointerMove,
        "onPointerEnter" => EventKind::PointerEnter,
        "onPointerLeave" => EventKind::PointerLeave,
        "onPointerUp" => EventKind::PointerUp,
        "onPointerCancel" => EventKind::PointerCancel,
        "onWheel" => EventKind::Wheel,
        "onContextMenu" => EventKind::ContextMenu,
        "onKeyDown" => EventKind::KeyDown,
        "onKeyUp" => EventKind::KeyUp,
        "onInput" => EventKind::Input,
        "onChange" => EventKind::Change,
        "onFocus" => EventKind::FocusIn,
        "onBlur" => EventKind::FocusOut,
        "onAnimationStart" => EventKind::AnimationStart,
        "onAnimationIteration" => EventKind::AnimationIteration,
        "onAnimationEnd" => EventKind::AnimationEnd,
        "onAnimationCancel" => EventKind::AnimationCancel,
        "onTransitionRun" => EventKind::TransitionRun,
        "onTransitionStart" => EventKind::TransitionStart,
        "onTransitionEnd" => EventKind::TransitionEnd,
        "onTransitionCancel" => EventKind::TransitionCancel,
        _ => return Err(Exception::throw_type(ctx, "unsupported native event property or listener options")),
    };
    Ok((kind, capture))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DismissReason { OutsidePress, Escape }

impl DismissReason {
    pub(crate) fn name(self) -> &'static str {
        match self { Self::OutsidePress => "outside-press", Self::Escape => "escape" }
    }
}

pub(crate) fn dismiss_reason(kind: EventKind, payload: &Payload) -> Option<DismissReason> {
    match (kind, payload) {
        (EventKind::PointerDown, Payload::Pointer(_)) => Some(DismissReason::OutsidePress),
        (EventKind::KeyDown, Payload::Key(key)) if key.key == Key::Named(NamedKey::Escape) => Some(DismissReason::Escape),
        _ => None,
    }
}

pub(crate) struct EventSnapshot {
    pub(crate) target: NodeId,
    pub(crate) current_target: NodeId,
    kind: EventKind,
    timestamp: Timestamp,
    phase: Phase,
    modifiers: Modifiers,
    payload: Payload,
    coalesced: Vec<PointerSample>,
}

impl EventSnapshot {
    pub(crate) fn capture(event: &EventCx<'_>) -> Self {
        Self {
            target: event.target, current_target: event.current, kind: event.kind,
            timestamp: event.timestamp, phase: event.phase, modifiers: event.modifiers,
            payload: event.payload().clone(), coalesced: event.coalesced().to_vec(),
        }
    }

    /// Wrappers are resolved/retargeted by the owner; no raw native IDs enter JS.
    pub(crate) fn to_js<'js>(
        &self, ctx: &Ctx<'js>, target: &Object<'js>, current: &Object<'js>,
    ) -> rquickjs::Result<(Object<'js>, DispatchControl)> {
        let control = DispatchControl(Rc::new(Cell::new(ControlState { active: true, ..ControlState::default() })));
        let object = Object::new(ctx.clone())?;
        object.set("type", self.kind.web_name())?;
        object.set("timestamp", milliseconds(self.timestamp))?;
        object.set("phase", match self.phase { Phase::Capture => "capture", Phase::Target => "target", Phase::Bubble => "bubble", _ => "unknown" })?;
        object.set("target", target.clone())?;
        object.set("currentTarget", current.clone())?;
        let modifiers = Object::new(ctx.clone())?;
        modifiers.set("shift", self.modifiers.shift())?;
        modifiers.set("control", self.modifiers.control())?;
        modifiers.set("alt", self.modifiers.alt())?;
        modifiers.set("meta", self.modifiers.meta())?;
        object.set("modifiers", modifiers)?;
        match &self.payload {
            Payload::Pointer(pointer) => {
                object.set("position", xy_object(ctx, pointer.position.x.0, pointer.position.y.0)?)?;
                object.set("pointerId", pointer.id.raw().to_string())?;
                object.set("pointerKind", pointer_kind(pointer.kind))?;
                object.set("primary", pointer.primary)?;
                let button = match pointer.button {
                    Some(PointerButton::Primary) => Some("primary"), Some(PointerButton::Secondary) => Some("secondary"),
                    Some(PointerButton::Middle) => Some("middle"), Some(PointerButton::Back) => Some("back"),
                    Some(PointerButton::Forward) => Some("forward"), Some(PointerButton::Other(_)) => Some("other"),
                    Some(_) => Some("unknown"), None => None,
                };
                object.set("button", nullable(ctx, button)?)?;
                if let Some(PointerButton::Other(index)) = pointer.button { object.set("buttonIndex", index)?; }
                object.set("pressure", nullable(ctx, pointer.pressure)?)?;
                let samples = Array::new(ctx.clone())?;
                for (index, sample) in self.coalesced.iter().enumerate() {
                    let value = Object::new(ctx.clone())?;
                    value.set("position", xy_object(ctx, sample.position.x.0, sample.position.y.0)?)?;
                    value.set("pressure", nullable(ctx, sample.pressure)?)?;
                    value.set("timestamp", milliseconds(sample.timestamp))?;
                    samples.set(index, value)?;
                }
                object.set("coalesced", samples)?;
                let state = control.0.clone();
                let kind = self.kind;
                object.set("activateNow", Func::from(move |ctx: Ctx<'js>| -> rquickjs::Result<()> {
                    let mut value = state.get();
                    if !value.active { return Err(Exception::throw_reference(&ctx, "expired-dispatch: native event cancellation is only valid during its synchronous handler")); }
                    if kind != EventKind::PointerDown { return Err(Exception::throw_type(&ctx, "activateNow is only valid during a native pointerdown event")); }
                    value.activate_now = true;
                    state.set(value);
                    Ok(())
                }))?;
                for (name, capture) in [("capturePointer", true), ("releasePointer", false)] {
                    let state = control.0.clone();
                    let kind = self.kind;
                    object.set(name, Func::from(move |ctx: Ctx<'js>| -> rquickjs::Result<()> {
                        let mut value = state.get();
                        if !value.active { return Err(Exception::throw_reference(&ctx, "expired-dispatch: pointer capture is only valid during its synchronous handler")); }
                        if (capture && kind != EventKind::PointerDown)
                            || !matches!(kind, EventKind::PointerDown | EventKind::PointerMove | EventKind::PointerUp | EventKind::PointerCancel)
                        {
                            return Err(Exception::throw_type(&ctx, "capturePointer requires pointerdown; releasePointer requires a pointer event"));
                        }
                        value.pointer_capture = Some(capture);
                        state.set(value);
                        Ok(())
                    }))?;
                }
            }
            Payload::Wheel(wheel) => {
                let (x, y, unit) = match wheel.delta {
                    ScrollDelta::Pixels(size) => (size.width.0, size.height.0, "pixels"),
                    ScrollDelta::Lines { x, y } => (x, y, "lines"),
                    _ => return Err(Exception::throw_type(ctx, "unsupported native wheel delta")),
                };
                let delta = xy_object(ctx, x, y)?;
                delta.set("unit", unit)?;
                object.set("delta", delta)?;
                object.set("scrollPhase", match wheel.phase {
                    ScrollPhase::Discrete => "discrete", ScrollPhase::Started => "started",
                    ScrollPhase::Moved => "moved", ScrollPhase::Ended => "ended",
                    ScrollPhase::Momentum => "momentum", _ => "unknown",
                })?;
                object.set("position", xy_object(ctx, wheel.position.x.0, wheel.position.y.0)?)?;
                object.set("pointerId", wheel.id.raw().to_string())?;
                object.set("pointerKind", pointer_kind(wheel.kind))?;
            }
            Payload::Key(key) => {
                object.set("key", nullable(ctx, key.key.as_str())?)?;
                object.set("character", nullable(ctx, match &key.key { Key::Character(text) => Some(text.as_str()), _ => None })?)?;
                object.set("keyWithoutModifiers", nullable(ctx, key.key_without_modifiers.as_str())?)?;
                object.set("deadKey", nullable(ctx, match &key.key { Key::Dead(accent) => accent.map(|accent| accent.to_string()), _ => None })?)?;
                object.set("code", nullable(ctx, key.physical.code().map(|code| code.as_str()))?)?;
                object.set("physicalCode", nullable(ctx, match key.physical { PhysicalKey::Unidentified(code) => Some(code), _ => None })?)?;
                object.set("location", match key.location { KeyLocation::Standard => "standard", KeyLocation::Left => "left", KeyLocation::Right => "right", KeyLocation::Numpad => "numpad", _ => "unknown" })?;
                object.set("repeat", key.repeat)?;
            }
            Payload::Value(value) => {
                object.set("value", value.value.as_str())?;
                let selection = Object::new(ctx.clone())?;
                selection.set("start", value.selection.start)?;
                selection.set("end", value.selection.end)?;
                object.set("selection", selection)?;
                object.set("valueChange", match value.kind { ValueChange::Input => "input", ValueChange::Committed => "committed", _ => "unknown" })?;
            }
            Payload::Focus(focus) => {
                object.set("cause", match focus.cause { FocusCause::Pointer => "pointer", FocusCause::Keyboard => "keyboard", FocusCause::Programmatic => "programmatic", FocusCause::Window => "window", _ => "unknown" })?;
                // Related is an accessibility ID, not a view NodeId. Do not fabricate a wrapper.
            }
            Payload::Animation(animation) => {
                object.set("animationName", animation.name.as_str())?;
                object.set("elapsedTime", animation.elapsed.as_secs_f64())?;
                object.set("pseudoElement", animation.pseudo.map(|pseudo| format!("::{}", pseudo.label())).unwrap_or_default())?;
            }
            Payload::Transition(transition) => {
                object.set("propertyName", transition.property.as_str())?;
                object.set("elapsedTime", transition.elapsed.as_secs_f64())?;
                object.set("pseudoElement", transition.pseudo.map(|pseudo| format!("::{}", pseudo.label())).unwrap_or_default())?;
            }
            _ => return Err(Exception::throw_type(ctx, "unsupported native event payload")),
        }
        for (name, flag) in [("preventDefault", Flag::Default), ("stopPropagation", Flag::Propagation), ("stopImmediatePropagation", Flag::Immediate)] {
            let state = control.0.clone();
            object.set(name, Func::from(move |ctx: Ctx<'js>| -> rquickjs::Result<()> {
                let mut value = state.get();
                if !value.active { return Err(Exception::throw_reference(&ctx, "expired-dispatch: native event cancellation is only valid during its synchronous handler")); }
                match flag { Flag::Default => value.prevent_default = true, Flag::Propagation => value.stop = true, Flag::Immediate => value.immediate = true }
                state.set(value);
                Ok(())
            }))?;
        }
        Ok((object, control))
    }

    pub(crate) fn dismiss_reason(&self) -> Option<DismissReason> { dismiss_reason(self.kind, &self.payload) }
}

fn milliseconds(timestamp: Timestamp) -> f64 { timestamp.since_origin().as_secs_f64() * 1000.0 }

fn pointer_kind(kind: PointerKind) -> &'static str {
    match kind {
        PointerKind::Mouse => "mouse", PointerKind::Touch => "touch",
        PointerKind::Pen => "pen", _ => "unknown",
    }
}

fn nullable<'js, T: IntoJs<'js>>(ctx: &Ctx<'js>, value: Option<T>) -> rquickjs::Result<Value<'js>> {
    match value { Some(value) => value.into_js(ctx), None => Ok(Value::new_null(ctx.clone())) }
}

fn xy_object<'js>(ctx: &Ctx<'js>, x: f32, y: f32) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("x", x)?;
    object.set("y", y)?;
    Ok(object)
}

#[derive(Clone, Copy, Default)]
struct ControlState { active: bool, prevent_default: bool, stop: bool, immediate: bool, activate_now: bool, pointer_capture: Option<bool> }
#[derive(Clone, Copy)]
enum Flag { Default, Propagation, Immediate }

/// Owns only copied flags. Dropping it revokes JS cancellation even on errors.
pub(crate) struct DispatchControl(Rc<Cell<ControlState>>);

impl DispatchControl {
    pub(crate) fn expire_and_apply(&self, event: &mut EventCx<'_>) {
        let state = self.0.replace(ControlState::default());
        if !state.active { return; }
        // Revoke authority before touching native dispatch, including on repeated calls.
        if state.prevent_default { event.prevent_default(); }
        if state.immediate { event.stop_immediate_propagation(); }
        else if state.stop { event.stop_propagation(); }
        match state.pointer_capture {
            Some(true) => event.capture_pointer(),
            Some(false) => event.release_pointer(),
            None => {}
        }
        if state.activate_now { event.activate_now(); }
    }
}

impl Drop for DispatchControl {
    fn drop(&mut self) { self.0.set(ControlState::default()); }
}
