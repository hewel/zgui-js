//! Registry lifecycle and protocol selection for compositor background blur.

use std::sync::Mutex;

use smithay_client_toolkit::globals::GlobalData;
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use smithay_client_toolkit::registry::RegistryHandler;
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, Capability, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};

use zgui_platform::{WakeReason, Waker as _};

use crate::driver::WaylandState;

#[allow(
    missing_docs, unreachable_pub, dead_code, unused_imports,
    non_upper_case_globals, non_camel_case_types, clippy::all
)]
pub(crate) mod niri_blur_shape_v1 {
    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("src/driver/niri-blur-shape-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("src/driver/niri-blur-shape-v1.xml");
}

pub(crate) use niri_blur_shape_v1::{
    niri_blur_shape_manager_v1::NiriBlurShapeManagerV1,
    niri_blur_shape_surface_v1::NiriBlurShapeSurfaceV1,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Selection {
    None,
    Ext(u64),
    Niri(u64),
}

pub(crate) enum ActiveBlurProtocol {
    None,
    ExtBackgroundEffect(u64, ExtBackgroundEffectManagerV1),
    NiriBlurShape(u64, NiriBlurShapeManagerV1),
}

#[derive(Debug, Default)]
struct BackgroundEffectInner {
    ext: Option<(u32, ExtBackgroundEffectManagerV1)>,
    niri: Option<(u32, NiriBlurShapeManagerV1)>,
    ext_blur: bool,
    ext_gen: u64,
    niri_gen: u64,
}

impl BackgroundEffectInner {
    fn selection(&self) -> Selection {
        if self.niri.is_some() {
            Selection::Niri(self.niri_gen)
        } else if self.ext_blur {
            Selection::Ext(self.ext_gen)
        } else {
            Selection::None
        }
    }
}

/// Thread-safe handle to the live blur managers, preferring niri shapes over binary blur.
#[derive(Debug, Default)]
pub struct BackgroundEffectLink {
    inner: Mutex<BackgroundEffectInner>,
}

impl BackgroundEffectLink {
    pub(crate) fn new(
        ext: Option<(u32, ExtBackgroundEffectManagerV1)>,
        niri: Option<(u32, NiriBlurShapeManagerV1)>,
    ) -> Self {
        Self {
            inner: Mutex::new(BackgroundEffectInner { ext, niri, ..Default::default() }),
        }
    }

    pub(crate) fn blur_supported(&self) -> bool {
        self.inner.lock().expect("blur managers not poisoned").selection() != Selection::None
    }

    pub(crate) fn supports_blur_shapes(&self) -> bool {
        self.inner.lock().expect("blur managers not poisoned").niri.is_some()
    }

    pub(crate) fn active(&self) -> ActiveBlurProtocol {
        let inner = self.inner.lock().expect("blur managers not poisoned");
        if let Some((_, manager)) = &inner.niri {
            ActiveBlurProtocol::NiriBlurShape(inner.niri_gen, manager.clone())
        } else if let Some((_, manager)) = &inner.ext
            && inner.ext_blur
        {
            ActiveBlurProtocol::ExtBackgroundEffect(inner.ext_gen, manager.clone())
        } else {
            ActiveBlurProtocol::None
        }
    }
}

/// Tracks dynamic addition and removal of both background-blur globals.
pub(super) struct BackgroundEffectHandler;

impl RegistryHandler<WaylandState> for BackgroundEffectHandler {
    fn new_global(
        state: &mut WaylandState,
        _conn: &Connection,
        qh: &QueueHandle<WaylandState>,
        name: u32,
        interface: &str,
        version: u32,
    ) {
        if version == 0 {
            return;
        }
        let registry = state.registry.registry().clone();
        if interface == NiriBlurShapeManagerV1::interface().name {
            state.update_blur_managers(|inner| {
                inner.niri_gen += 1;
                let manager = registry.bind(name, 1, qh, GlobalData);
                if let Some((_, old)) = inner.niri.replace((name, manager))
                    && old.is_alive()
                {
                    old.destroy();
                }
            });
        } else if interface == ExtBackgroundEffectManagerV1::interface().name {
            state.update_blur_managers(|inner| {
                inner.ext_gen += 1;
                inner.ext_blur = false;
                let manager = registry.bind(name, 1, qh, GlobalData);
                if let Some((_, old)) = inner.ext.replace((name, manager))
                    && old.is_alive()
                {
                    old.destroy();
                }
            });
        }
    }

    fn remove_global(
        state: &mut WaylandState,
        _conn: &Connection,
        _qh: &QueueHandle<WaylandState>,
        name: u32,
        interface: &str,
    ) {
        if interface != NiriBlurShapeManagerV1::interface().name
            && interface != ExtBackgroundEffectManagerV1::interface().name
        {
            return;
        }
        state.update_blur_managers(|inner| {
            if inner.niri.as_ref().is_some_and(|(n, _)| *n == name)
                && let Some((_, manager)) = inner.niri.take()
                && manager.is_alive()
            {
                manager.destroy();
            }
            if inner.ext.as_ref().is_some_and(|(n, _)| *n == name) {
                inner.ext_blur = false;
                if let Some((_, manager)) = inner.ext.take()
                    && manager.is_alive()
                {
                    manager.destroy();
                }
            }
        });
    }
}

impl WaylandState {
    fn update_blur_managers(&mut self, update: impl FnOnce(&mut BackgroundEffectInner)) {
        let (old, new) = {
            let mut inner = self.background_effect.inner.lock().expect("blur managers not poisoned");
            let old = inner.selection();
            update(&mut inner);
            (old, inner.selection())
        };
        if old == new {
            return;
        }
        let blur = new != Selection::None;
        let shapes = matches!(new, Selection::Niri(_));
        let capabilities = &mut self.live.capabilities;
        let changed = blur != capabilities.supports_background_blur
            || shapes != capabilities.supports_blur_shapes;
        capabilities.supports_background_blur = blur;
        capabilities.supports_blur_shapes = shapes;
        for surface in self.live.all() {
            surface.blur_protocol_changed();
        }
        if changed {
            self.live.waker.wake(WakeReason::AppWork);
        }
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, GlobalData> for WaylandState {
    fn event(
        state: &mut Self,
        proxy: &ExtBackgroundEffectManagerV1,
        event: ext_background_effect_manager_v1::Event,
        _data: &GlobalData,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let ext_background_effect_manager_v1::Event::Capabilities { flags } = event {
            let blur = match flags {
                WEnum::Value(caps) => caps.contains(Capability::Blur),
                WEnum::Unknown(raw) => (raw & Capability::Blur.bits()) != 0,
            };
            state.update_blur_managers(|inner| {
                if inner.ext.as_ref().is_some_and(|(_, live)| live == proxy) {
                    inner.ext_blur = blur;
                }
            });
        }
    }
}

macro_rules! no_events {
    ($proxy:ty, $data:ty) => {
        impl Dispatch<$proxy, $data> for WaylandState {
            fn event(
                _state: &mut Self,
                _proxy: &$proxy,
                _event: <$proxy as Proxy>::Event,
                _data: &$data,
                _conn: &Connection,
                _qh: &QueueHandle<Self>,
            ) {}
        }
    };
}

no_events!(ExtBackgroundEffectSurfaceV1, ());
no_events!(NiriBlurShapeManagerV1, GlobalData);
no_events!(NiriBlurShapeSurfaceV1, ());

#[cfg(test)]
#[test]
fn niri_protocol_xml_pin() {
    use sha2::{Digest, Sha256};
    assert_eq!(
        format!("{:x}", Sha256::digest(include_bytes!("niri-blur-shape-v1.xml"))),
        "b1118671ff88677d9acefb63c5894dbe241e003b435d28f7832eba84909dd352"
    );
}
