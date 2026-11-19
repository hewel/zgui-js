//! Realize one generation's shell requests outside JavaScript and native window frames.
use std::collections::{BTreeMap, BTreeSet};

use zgui::{app::Fonts, geom::{Css, CssPx, Size}, platform::{InputRegion, PlatformCx, SurfaceAttributes, SurfaceId, SurfaceRole}, runtime::{Runtime, WindowContent, WindowSpec, WindowToken}, view::View};

use super::{ImageChoice, PredecessorLeak, ReloadOutcome, Root, RuntimeSurface, SessionHost, SessionSurface};
use crate::{session::{ResourceCounts, Result, Session, SessionModule}, shell::{LayerSpec, Monitor}};
use std::{cell::RefCell, rc::Rc};

struct RunningLayer {
    token: WindowToken,
    surface: SurfaceId,
    spec: LayerSpec,
}

#[derive(Default)]
pub(crate) struct ShellWindows {
    generation: Option<u64>,
    running: BTreeMap<u32, RunningLayer>,
    dismissed: BTreeSet<u32>,
}

impl SessionHost {
    /// Configure a generation that launches without a primary mount region.
    pub(crate) fn set_pure_shell(&mut self, fonts: Fonts) { self.pure_fonts = Some(fonts); }

    /// Whether package replacement goes through the zero-primary startup path.
    pub(crate) fn is_pure_shell(&self) -> bool { self.pure_fonts.is_some() }

    /// Supply the initial monitor inventory before candidate mounting.
    pub(crate) fn set_monitor_snapshot(&mut self, monitors: &[Monitor]) { self.monitors.clear(); self.monitors.extend_from_slice(monitors); }

    /// Mount a candidate and its shell surfaces before replacing the active generation.
    pub(crate) fn replace_shell(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, module: SessionModule) -> ReloadOutcome {
        let outcome = self.mount_shell(runtime, cx, images, &module);
        self.record_outcome(&module, None, &outcome);
        outcome
    }

    fn mount_shell(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, module: &SessionModule) -> ReloadOutcome {
        self.pure_attempted = true;
        let Some(fonts) = &self.pure_fonts else { return ReloadOutcome::Rejected("pure-shell startup was not configured".into()); };
        let Some(generation) = self.generation.checked_add(1) else { return ReloadOutcome::Rejected("session generation exhausted".into()); };
        self.generation = generation;
        let candidate = match Session::prepare_shell(fonts.clone(), runtime.app_timers(), module.clone(), generation, cx.waker(), self.policy.clone()) {
            Ok(candidate) => candidate,
            Err(error) => return ReloadOutcome::Rejected(error),
        };
        let mut staged = ShellWindows::default();
        let result = candidate.update_monitors(&self.monitors)
            .and_then(|_| candidate.mount())
            .and_then(|()| staged.mount(runtime, cx, images, &self.monitors, &candidate));
        if let Err(error) = result {
            staged.retire(&candidate, |token, id| close_native_surface(runtime, cx, token, id));
            if self.pure_session.is_none() { self.pure_session = Some(candidate); }
            return ReloadOutcome::Rejected(error);
        }
        let predecessor_leak = self.pure_session.take().and_then(|previous| {
            let report = self.shell_windows.retire(&previous, |token, id| close_native_surface(runtime, cx, token, id));
            (report.counts != ResourceCounts::default()).then_some(PredecessorLeak {
                code: "E_TEARDOWN", generation: report.generation, resources: report.counts,
            })
        });
        self.shell_windows = staged;
        self.module = module.clone();
        self.last_good = Some(module.clone());
        self.pure_session = Some(candidate);
        ReloadOutcome::Mounted { generation, predecessor_leak }
    }

    /// Deliver output changes, retire superseded surfaces and realize pending shell requests.
    pub(crate) fn shell_turn(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, monitors: &[Monitor]) -> Result<()> {
        if self.monitors != monitors { self.set_monitor_snapshot(monitors); }
        if self.is_pure_shell() && !self.pure_attempted {
            match self.replace_shell(runtime, cx, images, self.module.clone()) {
                ReloadOutcome::Mounted { .. } => {}
                ReloadOutcome::Rejected(error) | ReloadOutcome::Fallback(error) => return Err(error),
            }
        }
        let session = self.sessions().next().cloned();
        let Some(session) = session else { return Ok(()); };
        if self.shell_windows.generation != Some(session.generation) {
            self.shell_windows.generation = Some(session.generation);
        }
        let result = self.step_shell(runtime, cx, images, &session);
        if result.is_err() {
            self.shell_windows.retire(&session, |token, id| close_native_surface(runtime, cx, token, id));
        }
        result
    }

    fn step_shell(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, session: &Rc<Session>) -> Result<()> {
        let mut worked = session.update_monitors(&self.monitors)?;
        let surface = RuntimeSurface::new(runtime, cx, images);
        if session.primary().is_none() {
            surface.validate_fonts(session)?;
            worked |= session.checkpoint()?;
        }
        if worked { session.request_frames(|document| surface.request_frame(document)); }
        self.shell_windows.reconcile(runtime, cx, images, &self.monitors, session)
    }

    /// Target the current extent/monitor of a named preview surface without querying Wayland.
    pub(crate) fn shell_geometry(&self, surface: SurfaceId) -> Option<(f64, f64, Option<&str>)> {
        let running = self.shell_windows.running.values().find(|running| running.surface == surface)?;
        Some((f64::from(running.spec.size.width.0), f64::from(running.spec.size.height.0), running.spec.placement.monitor.as_deref()))
    }

    /// Recreate one named surface for preview while retaining its session and other regions.
    pub(crate) fn recreate_shell_surface(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, monitors: &[Monitor], name: &str) -> Result<()> {
        let session = self.sessions().next().cloned().ok_or("no active session")?;
        let id = self.shell_windows.running.iter().find(|(_, layer)| layer.spec.name == name).map(|(&id, _)| id)
            .ok_or_else(|| format!("shell surface {name:?} is not active"))?;
        let mut spec = session.shell.specs().remove(&id).ok_or("shell surface was closed")?;
        spec.size = used_size(&spec, monitors, images)?;
        let result = self.shell_windows.close(runtime, cx, &session, id)
            .and_then(|()| self.shell_windows.open(runtime, cx, images, &session, id, spec));
        if result.is_err() {
            self.shell_windows.retire(&session, |token, id| close_native_surface(runtime, cx, token, id));
        }
        result
    }

    /// Dispose the shell owner and destroy its native surfaces before checking for leaks.
    pub(crate) fn stop_shell_surfaces(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx) {
        let owner = self.sessions().find(|session| Some(session.generation) == self.shell_windows.generation).cloned();
        if let Some(owner) = owner {
            self.shell_windows.retire(&owner, |token, id| close_native_surface(runtime, cx, token, id));
        }
    }
}

impl ShellWindows {
    pub(super) fn mount(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, monitors: &[Monitor], session: &Rc<Session>) -> Result<()> {
        self.generation = Some(session.generation);
        // Child mounts can enqueue more layer requests. Settle those before publishing acceptance.
        for _ in 0..=crate::shell::MAX_LAYERS {
            self.reconcile(runtime, cx, images, monitors, session)?;
            if !session.shell.is_dirty() { return Ok(()); }
        }
        Err("shell surface requests did not settle during mount".into())
    }

    pub(super) fn has_windows(&self) -> bool { !self.running.is_empty() }

    pub(super) fn retire(
        &mut self,
        session: &Session,
        mut close: impl FnMut(WindowToken, SurfaceId) -> bool,
    ) -> crate::session::SessionRetirement {
        session.dispose();
        if self.generation == Some(session.generation) {
            for layer in std::mem::take(&mut self.running).into_values() {
                if close(layer.token, layer.surface) { session.shell_surface_closed(); }
            }
            self.dismissed.clear();
            self.generation = None;
        }
        session.retire()
    }

    fn reconcile(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, monitors: &[Monitor], session: &Rc<Session>) -> Result<()> {
        let mut needs_sync = session.shell.take_dirty();
        while let Some((&id, _)) = self.running.iter().find(|(_, layer)| runtime.surface_id(layer.token) != Some(layer.surface)) {
            // A compositor closes a role permanently. A disconnected output may
            // return with a new proxy; a dismissed role on a live output must not loop.
            if present(&self.running[&id].spec, monitors) { self.dismissed.insert(id); }
            self.close(runtime, cx, session, id)?;
            needs_sync = true;
        }
        if !needs_sync { return Ok(()); }
        let specs = session.shell.specs();
        self.dismissed.retain(|id| specs.get(id).is_some_and(|spec| present(spec, monitors)));
        while let Some((&id, _)) = self.running.iter().find(|(id, _)| !specs.get(id).is_some_and(|spec| present(spec, monitors))) {
            self.close(runtime, cx, session, id)?;
        }
        for (id, mut spec) in specs {
            if self.dismissed.contains(&id) || !present(&spec, monitors) || !session.shell.owns(id) { continue; }
            spec.size = used_size(&spec, monitors, images)?;
            if let Some(running) = self.running.get_mut(&id) {
                if running.spec.placement != spec.placement || running.spec.size != spec.size {
                    runtime.reconfigure_layer(running.token, spec.placement.clone(), spec.size)
                        .map_err(|error| error.to_string())?;
                }
                if running.spec.input != spec.input {
                    let surface = cx.surface(running.surface).ok_or("layer surface disappeared during update")?;
                    surface.set_input_region(&spec.input).map_err(|error| error.to_string())?;
                }
                running.spec = spec;
            } else {
                self.open(runtime, cx, images, session, id, spec)?;
            }
        }
        Ok(())
    }

    fn close(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, session: &Session, id: u32) -> Result<()> {
        let Some(running) = self.running.remove(&id) else { return Ok(()); };
        let result = session.detach_layer(id);
        if !close_native_surface(runtime, cx, running.token, running.surface) {
            return Err("E_TEARDOWN: native shell surface remained open".into());
        }
        session.shell_surface_closed();
        result
    }

    fn open(&mut self, runtime: &mut Runtime, cx: &dyn PlatformCx, images: ImageChoice, session: &Rc<Session>, id: u32, spec: LayerSpec) -> Result<()> {
        if matches!(images, ImageChoice::Live) && !cx.capabilities().layer_surfaces {
            return Err("selected native backend does not support layer surfaces".into());
        }
        let fonts = session.fonts.clone();
        let pending = Rc::new(RefCell::new(Vec::new()));
        let roots = pending.clone();
        let mut attrs = SurfaceAttributes::new(spec.name.as_str());
        attrs.role = SurfaceRole::Layer(spec.placement.clone());
        attrs.size = Some(spec.size);
        // The preview's opaque/profile styling remains in charge of background pixels.
        attrs.transparent = matches!(images, ImageChoice::Live);
        let token = runtime.open_surface(cx, WindowSpec::new(attrs, WindowContent::default(), Box::new(move |cx| {
            zgui::reactive::provide_context(fonts.clone());
            Box::new(Root(roots.clone()).build(cx))
        }))).map_err(|error| error.to_string())?;
        let Some(surface) = runtime.surface_id(token) else {
            runtime.close_surface(cx, token);
            return Err("opened shell window has no platform surface".into());
        };
        session.shell_surface_opened();
        let Some(mut region) = pending.borrow_mut().pop() else {
            if close_native_surface(runtime, cx, token, surface) { session.shell_surface_closed(); }
            return Err("opened shell window has no mount region".into());
        };
        region.root = region.root_anchor.get();
        let document = region.document;
        self.running.insert(id, RunningLayer { token, surface, spec });
        let mut runtime_surface = RuntimeSurface::new(runtime, cx, images);
        runtime_surface.bind(document, session);
        let checkpoint = runtime_surface.image_source_checkpoint(document);
        if let Err(error) = session.attach_layer(id, self.running[&id].spec.name.clone(), region, surface) {
            runtime_surface.discard_image_source_writes(document, checkpoint);
            return Err(error);
        }
        let writes = runtime_surface.seal_image_source_writes(document, checkpoint);
        runtime_surface.commit_image_policy(document, session, writes);
        let running = self.running.get_mut(&id).expect("new layer is tracked before its first frame");
        if let Some(input) = session.shell.updated_input(id, &running.spec.input) { running.spec.input = input; }
        let input = &running.spec.input;
        if *input != InputRegion::Full {
            cx.surface(surface).ok_or("opened shell window has no platform surface")?
                .set_input_region(input).map_err(|error| error.to_string())?;
        }
        runtime_surface.request_frame(document);
        Ok(())
    }
}

pub(super) fn close_native_surface(runtime: &mut Runtime, cx: &dyn PlatformCx, token: WindowToken, id: SurfaceId) -> bool {
    runtime.close_surface(cx, token);
    runtime.surface_id(token).is_none() && cx.surface(id).is_none()
}

fn present(spec: &LayerSpec, monitors: &[Monitor]) -> bool {
    spec.placement.monitor.as_ref().is_none_or(|id| monitors.iter().any(|monitor| monitor.id == *id))
}

fn used_size(spec: &LayerSpec, monitors: &[Monitor], images: ImageChoice) -> Result<Size<CssPx, Css>> {
    if matches!(images, ImageChoice::Live) { return Ok(spec.size); }
    let monitor = spec.placement.monitor.as_ref().map_or_else(|| monitors.first(), |id| monitors.iter().find(|monitor| monitor.id == *id));
    let margin = &spec.placement.margin;
    let width = if spec.size.width.0 == 0.0 {
        monitor.map_or(1.0, |output| (output.width / output.scale - f64::from(margin.left.0 + margin.right.0)).round().clamp(1.0, 16384.0))
    } else { f64::from(spec.size.width.0) };
    let height = if spec.size.height.0 == 0.0 {
        monitor.map_or(1.0, |output| (output.height / output.scale - f64::from(margin.top.0 + margin.bottom.0)).round().clamp(1.0, 16384.0))
    } else { f64::from(spec.size.height.0) };
    let scale = monitor.map_or(1.0, |output| output.scale);
    let physical_width = (width * scale).ceil();
    let physical_height = (height * scale).ceil();
    if physical_width > 16384.0 || physical_height > 16384.0 || (physical_width * physical_height).max(width * height) > 16_777_216.0 {
        return Err("preview shell surface exceeds its physical extent budget".into());
    }
    Ok(Size::new(CssPx(width as f32), CssPx(height as f32)))
}
