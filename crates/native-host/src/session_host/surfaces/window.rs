//! Reconcile compositor-managed toplevel windows owned by a shell session.
use std::rc::Rc;
use zgui::{platform::{PlatformCx, SurfaceAttributes, SurfaceId, SurfaceRole}, runtime::{Runtime, WindowToken}};

use super::{ImageChoice, RuntimeSurface, Session, SessionHost, SessionSurface, ShellKind, ShellWindows, close_native_surface, open_region, shell_window_content};
use crate::{session::Result, shell::ShellWindowSpec};

pub(super) struct RunningWindow {
    pub(super) token: WindowToken,
    pub(super) surface: SurfaceId,
    pub(super) spec: ShellWindowSpec,
}

impl SessionHost {
    /// Handle a compositor close request for a shell-owned toplevel window.
    pub(crate) fn shell_window_close_requested(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, surface: SurfaceId) -> Result<bool> {
        let generation = self.shell_windows.generation;
        let Some(session) = self.sessions().find(|session| Some(session.generation) == generation).cloned() else { return Ok(false); };
        let result = self.shell_windows.window_close_requested(runtime, cx, &session, surface);
        if result.is_err() {
            self.shell_windows.retire(&session, |token, id| close_native_surface(runtime, cx, token, id));
        }
        result
    }
}

impl ShellWindows {
    fn window_close_requested(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, session: &Session, surface: SurfaceId) -> Result<bool> {
        let Some((&id, _)) = self.windows.iter().find(|(_, window)| window.surface == surface) else { return Ok(false); };
        self.dismissed_windows.insert(id);
        session.close_window(id, || self.close_window(runtime, cx, session, id))?;
        Ok(true)
    }

    pub(super) fn close_window(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, session: &Session, id: u32) -> Result<()> {
        let Some(running) = self.windows.get(&id) else { return Ok(()); };
        self.close_child_popups(runtime, cx, session, running.surface)?;
        let running = self.windows.remove(&id).expect("window remains tracked until its popups close");
        let result = session.detach_layer(id);
        if !close_native_surface(runtime, cx, running.token, Some(running.surface)) {
            return Err("E_TEARDOWN: native shell window remained open".into());
        }
        session.shell_surface_closed();
        result
    }

    pub(super) fn reconcile_window_removals(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, session: &Session) -> Result<bool> {
        let mut changed = false;
        while let Some((&id, _)) = self.windows.iter().find(|(_, window)| {
            runtime.surface_id(window.token) != Some(window.surface) || cx.surface(window.surface).is_none()
        }) {
            self.dismissed_windows.insert(id);
            session.close_window(id, || self.close_window(runtime, cx, session, id))?;
            changed = true;
        }
        Ok(changed)
    }

    pub(super) fn reconcile_windows(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, session: &Rc<Session>) -> Result<()> {
        let specs = session.shell.window_specs();
        self.dismissed_windows.retain(|id| specs.contains_key(id));
        while let Some((&id, _)) = self.windows.iter().find(|(id, _)| !specs.contains_key(id)) {
            self.close_window(runtime, cx, session, id)?;
        }
        for (id, spec) in specs {
            if self.dismissed_windows.contains(&id) || !session.shell.owns_window(id) { continue; }
            if let Some(running) = self.windows.get_mut(&id) {
                if running.spec != spec {
                    let size_changed = (running.spec.size != spec.size).then_some(spec.size);
                    runtime.reconfigure_window(
                        running.token,
                        spec.title.as_str(),
                        size_changed,
                        spec.min_size,
                        spec.max_size,
                        spec.resizable,
                        spec.decorations,
                    ).map_err(|error| error.to_string())?;
                    running.spec = spec;
                }
                continue;
            }
            let name = spec.name.clone();
            let mut attrs = SurfaceAttributes::new(spec.title.as_str());
            attrs.role = SurfaceRole::Toplevel;
            attrs.application_id = spec.app_id.as_deref().map(Into::into);
            attrs.size = Some(spec.size);
            attrs.min_size = spec.min_size;
            attrs.max_size = spec.max_size;
            attrs.resizable = spec.resizable;
            attrs.decorations = spec.decorations;
            attrs.transparent = spec.transparent;
            let content = shell_window_content(attrs.transparent, images);
            let (token, surface, region) = open_region(runtime, cx, session, attrs, content)?;
            let document = region.document;
            self.windows.insert(id, RunningWindow { token, surface, spec });
            let mut binding = RuntimeSurface::new(runtime, cx, images);
            binding.attach_shell(session, id, name, region, surface, ShellKind::Layer)?;
            binding.request_frame(document);
        }
        Ok(())
    }
}
