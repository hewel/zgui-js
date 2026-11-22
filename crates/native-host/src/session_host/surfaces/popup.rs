//! Reconcile compositor-owned popup trees in parent-before-child open order.
use std::{collections::BTreeSet, rc::Rc};
use zgui::{platform::{InputRegion, InputSerial, PlatformCx, SurfaceId}, runtime::{Runtime, WindowToken}};

use super::{ImageChoice, Monitor, RuntimeSurface, Session, SessionHost, SessionSurface, ShellKind, ShellWindows, SurfaceOpacity, close_native_surface, open_region, present, shell_window_content};
use crate::{session::Result, shell::PopupSpec};

pub(super) struct RunningPopup {
    pub(super) token: WindowToken,
    pub(super) surface: SurfaceId,
    pub(super) parent: SurfaceId,
    pub(super) spec: PopupSpec,
    pub(super) id: u32,
    grab: Option<InputSerial>,
}

#[derive(Clone, Copy)]
enum Removal { Owner, Native, Dismissed(u32) }

impl SessionHost {
    /// Close a managed surface before the runtime invalidates its mount-region handles.
    pub(crate) fn shell_surface_destroyed(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, surface: SurfaceId) -> Result<bool> {
        let generation = self.shell_windows.generation;
        let Some(session) = self.sessions().find(|session| Some(session.generation) == generation).cloned() else { return Ok(false); };
        let result = self.shell_windows.compositor_closed(runtime, cx, &session, &self.monitors, surface);
        if result.is_err() {
            self.shell_windows.retire(&session, |token, id| close_native_surface(runtime, cx, token, id));
        }
        result
    }

    /// Whether a surface is an active popup owned by this session host.
    #[must_use]
    pub(crate) fn is_popup_surface(&self, surface: SurfaceId) -> bool {
        self.shell_windows.popups.iter().any(|popup| popup.surface == surface)
    }
}

impl ShellWindows {
    fn compositor_closed(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, session: &Session, monitors: &[Monitor], surface: SurfaceId) -> Result<bool> {
        if let Some(id) = self.popups.iter().find(|popup| popup.surface == surface).map(|popup| popup.id) {
            let removal = if cx.surface(surface).is_some() { Removal::Dismissed(id) } else { Removal::Native };
            self.close_popup_tree(runtime, cx, session, surface, removal)?;
            return Ok(true);
        }
        if let Some((&id, _)) = self.windows.iter().find(|(_, window)| window.surface == surface) {
            self.dismissed_windows.insert(id);
            session.close_window(id, || self.close_window(runtime, cx, session, id))?;
            return Ok(true);
        }
        if let Some((&id, layer)) = self.running.iter().find(|(_, layer)| layer.surface == surface) {
            if present(&layer.spec, monitors) { self.dismissed.insert(id); }
            self.close(runtime, cx, session, id)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn reconcile_popup_removals(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, session: &Session) -> Result<bool> {
        let mut changed = false;
        // Protocol dispatch may dismiss a popup before delivering its close event.
        while let Some((surface, removal)) = self.popups.iter().rev().find_map(|popup| {
            let native = cx.surface(popup.surface);
            if native.as_ref().is_some_and(|surface| surface.popup_dismissed()) {
                Some((popup.surface, Removal::Dismissed(popup.id)))
            } else if native.is_none() || runtime.surface_id(popup.token) != Some(popup.surface) {
                Some((popup.surface, Removal::Native))
            } else if !session.shell.owns_popup(popup.id) {
                Some((popup.surface, Removal::Owner))
            } else {
                None
            }
        }) {
            self.close_popup_tree(runtime, cx, session, surface, removal)?;
            changed = true;
        }
        Ok(changed)
    }

    pub(super) fn reconcile_popups(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, session: &Rc<Session>) -> Result<()> {
        self.popups_waiting_for_geometry = false;
        self.dismissed_popups.retain(|id| session.shell.owns_popup(*id));
        let mut pending = session.shell.popup_specs();
        let mut blocked = 0;
        while let Some((id, spec)) = pending.pop_front() {
            if self.dismissed_popups.contains(&id) || !session.shell.owns_popup(id) { continue; }
            let running = self.popups.iter().position(|popup| popup.id == id);
            let parent = self.popup_parent(session, &spec.parent).filter(|&parent| {
                let ready = running.is_some() || cx.surface(parent).is_some_and(|surface| {
                    let size = surface.size();
                    size.width.0 > 0.0 && size.height.0 > 0.0
                });
                // A stretched layer has no logical extent until its first configure.
                self.popups_waiting_for_geometry |= !ready;
                ready
            });
            let Some(parent) = parent else {
                pending.push_back((id, spec));
                blocked += 1;
                if blocked >= pending.len() { break; }
                continue;
            };
            blocked = 0;
            if let Some(index) = running {
                let popup = &mut self.popups[index];
                if !popup.spec.geometry_eq(&spec) {
                    let placement = spec.placement(parent, popup.grab);
                    let auto = spec.auto_height(parent, popup.grab);
                    let next_size = match (&auto, popup.spec.is_auto_height()) {
                        (Some(policy), true) => {
                            let current_h = cx.surface(popup.surface).map_or(policy.max_height.0, |surface| {
                                let scale = (surface.scale_factor() as f32).max(f32::EPSILON);
                                (surface.size().height.0 / scale)
                                    .round()
                                    .clamp(policy.min_height.0, policy.max_height.0)
                            });
                            zgui::geom::Size::new(policy.width, zgui::geom::CssPx(current_h))
                        }
                        _ => spec.size(),
                    };
                    runtime.reconfigure_popup(popup.token, placement, next_size).map_err(|error| error.to_string())?;
                    runtime.set_popup_auto_height(popup.token, auto).map_err(|error| error.to_string())?;
                }
                if popup.spec.input != spec.input {
                    cx.surface(popup.surface).ok_or("running popup has no platform surface")?
                        .set_shell_input_region(&spec.input).map_err(|error| error.to_string())?;
                }
                popup.spec = spec;
                continue;
            }
            let name = spec.name.clone();
            let grab = spec
                .wants_grab()
                .then(|| cx.surface(parent).and_then(|surface| surface.input_serial()))
                .flatten();
            let placement = spec.placement(parent, grab);
            let auto = spec.auto_height(parent, grab);
            let mut attrs = zgui::platform::SurfaceAttributes::popup(placement);
            attrs.title = name.as_str().into();
            attrs.size = Some(spec.size());
            attrs.transparent = self.opacity == SurfaceOpacity::Transparent;
            let content = shell_window_content(attrs.transparent, images);
            let (token, surface, region) = open_region(runtime, cx, session, attrs, content)?;
            runtime.set_popup_auto_height(token, auto).map_err(|error| error.to_string())?;
            let document = region.document;
            self.popups.push(RunningPopup { id, token, surface, parent, spec, grab });
            let mut binding = RuntimeSurface::new(runtime, cx, images);
            binding.attach_shell(session, id, name, region, surface, ShellKind::Layer)?;
            let popup = self.popups.last_mut().expect("new popup is tracked before its first frame");
            if let Some(input) = session.shell.updated_input(id, &popup.spec.input) { popup.spec.input = input; }
            if popup.spec.input != InputRegion::Full {
                cx.surface(surface).ok_or("opened popup has no platform surface")?
                    .set_shell_input_region(&popup.spec.input).map_err(|error| error.to_string())?;
            }
            binding.request_frame(document);
        }
        Ok(())
    }

    #[must_use]
    fn popup_parent(&self, session: &Session, name: &str) -> Option<SurfaceId> {
        self.running.iter().find(|(id, layer)| layer.spec.name == name && session.shell.owns(**id)).map(|(_, layer)| layer.surface)
            .or_else(|| self.windows.iter().find(|(id, window)| window.spec.name == name && session.shell.owns_window(**id)).map(|(_, window)| window.surface))
            .or_else(|| self.popups.iter().find(|popup| popup.spec.name == name && session.shell.owns_popup(popup.id)).map(|popup| popup.surface))
    }

    pub(super) fn close_child_popups(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, session: &Session, parent: SurfaceId) -> Result<()> {
        self.close_popup_tree(runtime, cx, session, parent, Removal::Owner)
    }

    fn close_popup_tree(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, session: &Session, root: SurfaceId, removal: Removal) -> Result<()> {
        if self.popups.is_empty() { return Ok(()); }
        let mut targets = BTreeSet::from([root]);
        let indices: Vec<_> = self.popups.iter().enumerate().filter_map(|(index, popup)| {
            if popup.surface == root || targets.contains(&popup.parent) {
                targets.insert(popup.surface);
                Some(index)
            } else {
                None
            }
        }).collect();
        let close = || {
            for index in indices.into_iter().rev() {
                let popup = self.popups.remove(index);
                if !matches!(removal, Removal::Owner) { self.dismissed_popups.insert(popup.id); }
                let cleanup = session.detach_layer(popup.id);
                if !close_native_surface(runtime, cx, popup.token, Some(popup.surface)) {
                    return Err("E_TEARDOWN: native popup surface remained open".into());
                }
                session.shell_surface_closed();
                cleanup?;
            }
            Ok(())
        };
        match removal {
            Removal::Dismissed(id) => session.dismiss_popup(id, close),
            _ => close(),
        }
    }
}
