//! Reconcile compositor-owned popup trees in parent-before-child open order.
use std::{collections::BTreeSet, rc::Rc};
use zgui::{platform::{PlatformCx, SurfaceId}, runtime::{Runtime, WindowContent, WindowToken}};

use super::{ImageChoice, Monitor, RuntimeSurface, Session, SessionHost, SessionSurface, ShellKind, ShellWindows, SurfaceOpacity, close_native_surface, open_region, present};
use crate::{session::Result, shell::PopupSpec};

pub(super) struct RunningPopup {
    pub(super) token: WindowToken,
    pub(super) surface: SurfaceId,
    pub(super) parent: SurfaceId,
    pub(super) spec: PopupSpec,
    pub(super) id: u32,
    grab: bool,
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
            let mut placement = spec.placement(parent);
            if let Some(index) = running {
                let popup = &mut self.popups[index];
                if popup.spec != spec {
                    placement.grab = popup.grab;
                    runtime.reconfigure_popup(popup.token, placement, spec.size()).map_err(|error| error.to_string())?;
                    popup.spec = spec;
                }
                continue;
            }
            let name = spec.name.clone();
            let grab = placement.grab;
            let mut attrs = zgui::platform::SurfaceAttributes::popup(placement);
            attrs.title = name.as_str().into();
            attrs.size = Some(spec.size());
            attrs.transparent = self.opacity == SurfaceOpacity::Transparent;
            let (token, surface, region) = open_region(runtime, cx, session, attrs, WindowContent::default())?;
            let document = region.document;
            self.popups.push(RunningPopup { id, token, surface, parent, spec, grab });
            let mut binding = RuntimeSurface::new(runtime, cx, images);
            binding.attach_shell(session, id, name, region, surface, ShellKind::Layer)?;
            binding.request_frame(document);
        }
        Ok(())
    }

    #[must_use]
    fn popup_parent(&self, session: &Session, name: &str) -> Option<SurfaceId> {
        self.running.iter().find(|(id, layer)| layer.spec.name == name && session.shell.owns(**id)).map(|(_, layer)| layer.surface)
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
        if let Removal::Dismissed(id) = removal { session.dismiss_popup(id)?; }
        Ok(())
    }
}
