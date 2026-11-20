use crate::{Application, bounded_fs::{self, BoundedReadError}};
use serde::Deserialize;
use std::{error::Error, path::Path};
use zgui::{geom::{CssPx, Edges, Size}, platform::{AppHandler, IdlePolicy, KeyboardInteractivity, Layer, LayerPlacement, PlatformCx, SurfaceAttributes, SurfaceEvent, SurfaceId, SurfaceRole, WakeReason}};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

mod session;
pub(crate) use session::{Authority, ContractPolicy, LayerSpec, LockCommand, LockSpec, MAX_LAYERS, Monitor, PopupSpec, Shell, ShellConfig, valid_name};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Stack { Background, Bottom, Top, Overlay }
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Keyboard { None, OnDemand, Exclusive }
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Edge { Top, Right, Bottom, Left }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Margins { top: i16, right: i16, bottom: i16, left: i16 }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LayerConfig {
    pub namespace: String,
    monitor: String,
    layer: Stack,
    anchors: Vec<Edge>,
    width: u16,
    height: u16,
    margins: Margins,
    exclusive_zone: u16,
    keyboard: Keyboard,
    transparent: bool,
}

impl LayerConfig {
    pub(crate) fn is_transparent(&self) -> bool {
        self.transparent
    }

    pub(crate) fn load(path: &Path) -> Result<Self> {
        let bytes = bounded_fs::read_input(path, 65_536).map_err(|error| match error {
            BoundedReadError::TooLarge { .. } => Box::<dyn Error>::from("layer configuration exceeds 64 KiB"),
            other => Box::new(other),
        })?;
        let config: Self = serde_json::from_slice(&bytes)?;
        config.validate()?;
        Ok(config)
    }

    fn edges(&self) -> Result<Edges<bool>> {
        let mut edges = Edges::new(false, false, false, false);
        for edge in &self.anchors {
            let enabled = match edge { Edge::Top => &mut edges.top, Edge::Right => &mut edges.right, Edge::Bottom => &mut edges.bottom, Edge::Left => &mut edges.left };
            if *enabled { return Err("layer anchors contain a duplicate edge".into()); }
            *enabled = true;
        }
        Ok(edges)
    }

    fn validate(&self) -> Result<()> {
        if self.namespace.is_empty() || self.monitor.is_empty() { return Err("layer namespace and monitor must be nonempty".into()); }
        let edges = self.edges()?;
        if self.width == 0 && !(edges.left && edges.right) { return Err("zero layer width requires left and right anchors".into()); }
        if self.height == 0 && !(edges.top && edges.bottom) { return Err("zero layer height requires top and bottom anchors".into()); }
        if self.exclusive_zone != 0 && !matches!(self.anchors.len(), 1 | 3) {
            return Err("positive exclusiveZone requires exactly one edge or three edges".into());
        }
        Ok(())
    }

    fn monitor_present(&self, cx: &dyn PlatformCx) -> bool {
        cx.monitors().iter().any(|monitor| monitor.name.as_deref() == Some(self.monitor.as_str()))
    }

    fn configure(&self, attributes: &mut SurfaceAttributes) -> Result<()> {
        attributes.role = SurfaceRole::Layer(LayerPlacement {
            layer: match self.layer { Stack::Background => Layer::Background, Stack::Bottom => Layer::Bottom, Stack::Top => Layer::Top, Stack::Overlay => Layer::Overlay },
            anchors: self.edges()?,
            margin: Edges::new(CssPx(f32::from(self.margins.top)), CssPx(f32::from(self.margins.right)), CssPx(f32::from(self.margins.bottom)), CssPx(f32::from(self.margins.left))),
            // Never use None: the pinned backend emits protocol -1 for it, which
            // ignores other exclusive zones rather than reserving our extent.
            exclusive_zone: Some(CssPx(f32::from(self.exclusive_zone))),
            keyboard: match self.keyboard { Keyboard::None => KeyboardInteractivity::None, Keyboard::OnDemand => KeyboardInteractivity::OnDemand, Keyboard::Exclusive => KeyboardInteractivity::Exclusive },
            monitor: Some(self.monitor.clone()),
        });
        attributes.size = Some(Size::new(CssPx(f32::from(self.width)), CssPx(f32::from(self.height))));
        attributes.transparent = self.transparent;
        Ok(())
    }
}

/// Owns one explicitly selected output. A removed output suspends its JS session;
/// return of that same output creates a fresh native document and JS generation.
pub(crate) struct LayerHost {
    inner: Application,
    config: LayerConfig,
    waiting_for_output: bool,
}

impl LayerHost {
    pub(crate) fn new(inner: Application, config: LayerConfig) -> Self {
        Self { inner, config, waiting_for_output: false }
    }

    fn open(&mut self, cx: &dyn PlatformCx) -> Result<()> {
        if !cx.capabilities().layer_surfaces { return Err("selected native backend does not support layer surfaces".into()); }
        if !self.config.monitor_present(cx) { return Err(format!("requested layer monitor {:?} is unavailable", self.config.monitor).into()); }
        let attributes = self.inner.native.primary_surface_attributes_mut().ok_or("primary layer is already open or permanently closed")?;
        self.config.configure(attributes)?;
        self.waiting_for_output = false;
        self.inner.surfaces_available(cx);
        eprintln!("layer-created: namespace={} monitor={} requested-css={}x{} exclusive-zone={}", self.config.namespace, self.config.monitor, self.config.width, self.config.height, self.config.exclusive_zone);
        Ok(())
    }

    fn suspend_output(&mut self, cx: &dyn PlatformCx) {
        let primary = self.inner.native.windows().first().map(|window| window.surface().id());
        self.inner.surfaces_lost(cx);
        if let Some(primary) = primary { cx.destroy_surface(primary); }
        self.waiting_for_output = true;
        eprintln!("layer-monitor-removed: {}; waiting for that output to return", self.config.monitor);
    }
}

impl AppHandler for LayerHost {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        if let Err(error) = self.open(cx) { self.inner.fail(cx, error); }
    }

    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) {
        self.waiting_for_output = false;
        self.inner.surfaces_lost(cx);
    }

    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) {
        let owned = self.inner.native.windows().iter().any(|window| window.surface().id() == surface);
        if owned && !cx.is_exiting() && matches!(event, SurfaceEvent::CloseRequested | SurfaceEvent::Destroyed)
            && !self.config.monitor_present(cx)
        {
            self.suspend_output(cx);
            return;
        }
        if owned && let SurfaceEvent::ScaleFactorChanged { scale_factor, size } = &event {
            eprintln!("layer-scale-changed: scale={scale_factor} device-size={size:?}");
        }
        self.inner.surface_event(cx, surface, event);
    }

    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) { self.inner.wake(cx, reason); }

    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy { self.inner.idle(cx) }

    fn deadline_reached(&mut self, cx: &dyn PlatformCx) { self.inner.deadline_reached(cx); }

    fn monitors_changed(&mut self, cx: &dyn PlatformCx) {
        if cx.is_exiting() { return; }
        if !self.config.monitor_present(cx) {
            if !self.waiting_for_output { self.suspend_output(cx); }
        } else if self.waiting_for_output {
            if let Err(error) = self.open(cx) { self.inner.fail(cx, error); }
        } else {
            self.inner.monitors_changed(cx);
        }
    }

    fn shutting_down(&mut self, cx: &dyn PlatformCx) {
        self.inner.shutting_down(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LayerConfig {
        LayerConfig { namespace: "test.panel".into(), monitor: "TEST-1".into(), layer: Stack::Top,
            anchors: vec![Edge::Top, Edge::Left, Edge::Right], width: 0, height: 64,
            margins: Margins { top: 0, right: 0, bottom: 0, left: 0 }, exclusive_zone: 64,
            keyboard: Keyboard::None, transparent: true }
    }

    #[test]
    fn invalid_stretch_or_reservation_is_rejected_before_wayland_protocol_use() {
        let mut invalid = config();
        invalid.anchors = vec![Edge::Top];
        assert!(invalid.validate().unwrap_err().to_string().contains("zero layer width"));
        invalid.width = 200;
        invalid.anchors = vec![Edge::Top, Edge::Left];
        assert!(invalid.validate().unwrap_err().to_string().contains("exclusiveZone"));
        invalid.anchors = vec![Edge::Top, Edge::Top];
        assert!(invalid.validate().unwrap_err().to_string().contains("duplicate"));
        invalid = config();
        invalid.height = 0;
        assert!(invalid.validate().unwrap_err().to_string().contains("zero layer height"));
    }
}
