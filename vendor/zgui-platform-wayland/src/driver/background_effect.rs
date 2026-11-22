//! Optional Wayland `ext_background_effect_v1` manager lifecycle and capability dispatch.

use std::sync::Mutex;

use smithay_client_toolkit::globals::GlobalData;
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use smithay_client_toolkit::registry::RegistryHandler;
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, Capability, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};

use crate::driver::WaylandState;

#[derive(Debug, Default)]
struct BackgroundEffectInner {
    active_name: Option<u32>,
    manager: Option<ExtBackgroundEffectManagerV1>,
    blur_supported: bool,
}

/// Thread-safe handle to the live `ext_background_effect_manager_v1` global and its blur flag.
#[derive(Debug, Default)]
pub struct BackgroundEffectLink {
    inner: Mutex<BackgroundEffectInner>,
}

impl BackgroundEffectLink {
    /// Creates a link initialized from an optionally bound startup manager.
    ///
    /// Blur support starts `false` even when a manager is bound; only a `capabilities` event from
    /// the live manager can enable it.
    pub(crate) fn new(initial: Option<(u32, ExtBackgroundEffectManagerV1)>) -> Self {
        let (active_name, manager) = match initial {
            Some((name, manager)) => (Some(name), Some(manager)),
            None => (None, None),
        };
        Self {
            inner: Mutex::new(BackgroundEffectInner {
                active_name,
                manager,
                blur_supported: false,
            }),
        }
    }

    /// Whether the currently active manager advertises [`Capability::Blur`].
    #[must_use]
    pub(crate) fn blur_supported(&self) -> bool {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.blur_supported && inner.manager.as_ref().is_some_and(Proxy::is_alive)
    }

    /// Clones the live manager if and only if background blur is currently supported.
    #[must_use]
    pub(crate) fn active_manager_for_blur(&self) -> Option<ExtBackgroundEffectManagerV1> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !inner.blur_supported {
            return None;
        }
        inner.manager.as_ref().filter(|m| m.is_alive()).cloned()
    }

    /// Applies a `capabilities` event if `proxy` is the live manager, ignoring retired managers.
    ///
    /// Returns `Some((was_supported, now_supported))` for the live manager or `None` if ignored.
    fn on_capabilities(
        &self,
        proxy: &ExtBackgroundEffectManagerV1,
        blur: bool,
    ) -> Option<(bool, bool)> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.manager.as_ref() != Some(proxy) {
            return None;
        }
        let was_supported = inner.blur_supported;
        inner.blur_supported = blur;
        Some((was_supported, blur))
    }

    /// Installs a freshly bound manager for `name`, retiring any previous manager and resetting
    /// blur support until the new manager reports its capabilities.
    fn replace_manager(
        &self,
        name: u32,
        manager: ExtBackgroundEffectManagerV1,
    ) -> (Option<ExtBackgroundEffectManagerV1>, bool) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.active_name = Some(name);
        let old = inner.manager.replace(manager);
        let was_supported = std::mem::replace(&mut inner.blur_supported, false);
        (old, was_supported)
    }

    /// Clears the active manager if its registry global `name` was removed.
    fn remove_manager(&self, name: u32) -> (Option<ExtBackgroundEffectManagerV1>, bool) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.active_name != Some(name) {
            return (None, false);
        }
        inner.active_name = None;
        let old = inner.manager.take();
        let was_supported = std::mem::replace(&mut inner.blur_supported, false);
        (old, was_supported)
    }
}

/// Registry handler that tracks dynamic addition and removal of `ext_background_effect_manager_v1`.
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
        if interface != ExtBackgroundEffectManagerV1::interface().name || version == 0 {
            return;
        }
        let manager: ExtBackgroundEffectManagerV1 =
            state
                .registry
                .registry()
                .bind(name, version.min(1), qh, GlobalData);
        state.replace_background_effect_manager(name, manager);
    }

    fn remove_global(
        state: &mut WaylandState,
        _conn: &Connection,
        _qh: &QueueHandle<WaylandState>,
        name: u32,
        interface: &str,
    ) {
        if interface == ExtBackgroundEffectManagerV1::interface().name {
            state.remove_background_effect_manager(name);
        }
    }
}

impl WaylandState {
    fn handle_background_effect_capabilities(
        &mut self,
        proxy: &ExtBackgroundEffectManagerV1,
        blur: bool,
    ) {
        let Some((was_supported, now_supported)) =
            self.background_effect.on_capabilities(proxy, blur)
        else {
            return;
        };
        self.live.capabilities.supports_background_blur = now_supported;
        if was_supported != now_supported {
            if now_supported {
                self.on_background_blur_gained();
            } else {
                self.on_background_blur_lost();
            }
            zgui_platform::Waker::wake(
                self.live.waker.as_ref(),
                zgui_platform::WakeReason::AppWork,
            );
        }
    }

    fn replace_background_effect_manager(
        &mut self,
        name: u32,
        manager: ExtBackgroundEffectManagerV1,
    ) {
        let (old, was_supported) = self.background_effect.replace_manager(name, manager);
        if let Some(old) = old
            && old.is_alive()
        {
            old.destroy();
        }
        self.live.capabilities.supports_background_blur = false;
        if was_supported {
            self.on_background_blur_lost();
            zgui_platform::Waker::wake(
                self.live.waker.as_ref(),
                zgui_platform::WakeReason::AppWork,
            );
        }
    }

    fn remove_background_effect_manager(&mut self, name: u32) {
        let (old, was_supported) = self.background_effect.remove_manager(name);
        let had_manager = old.is_some();
        if let Some(old) = old
            && old.is_alive()
        {
            old.destroy();
        }
        if had_manager || was_supported {
            self.live.capabilities.supports_background_blur = false;
            self.on_background_blur_lost();
        }
        if was_supported {
            zgui_platform::Waker::wake(
                self.live.waker.as_ref(),
                zgui_platform::WakeReason::AppWork,
            );
        }
    }

    fn on_background_blur_lost(&self) {
        for surface in self.live.all() {
            surface.background_blur_lost();
        }
    }

    fn on_background_blur_gained(&self) {
        for surface in self.live.all() {
            surface.background_blur_gained();
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
            state.handle_background_effect_capabilities(proxy, blur);
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtBackgroundEffectSurfaceV1,
        _event: <ExtBackgroundEffectSurfaceV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // `ext_background_effect_surface_v1` has no compositor events.
    }
}
