//! Asking the desktop to bring a window forward, draw attention to it, or export a single-use
//! `xdg_activation_v1` token for an external client.
//!
//! Internal focus/attention and external token export share the `xdg_activation_v1` global, yet
//! diverge completely in ownership: internal activation queues by surface and immediately calls
//! `activate` when `done` arrives, whereas external export commits `(wl_surface, wl_seat, serial)`
//! synchronously at the caller's gesture boundary so closing a popup or layer surface later in the
//! same turn cannot race the outgoing request, destroys the `xdg_activation_token_v1` object
//! exactly once on completion or cancellation, and never calls `activate` inside this process.
//! Inbound redemption instead submits an existing token to the target window, without minting.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use smithay_client_toolkit::activation::{ActivationHandler, ActivationState, RequestData};
use smithay_client_toolkit::delegate_activation;
use smithay_client_toolkit::globals::ProvidesBoundGlobal;
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_protocols::xdg::activation::v1::client::{
    xdg_activation_token_v1::{self, XdgActivationTokenV1},
    xdg_activation_v1::XdgActivationV1,
};
use zgui_platform::{
    ActivationToken, ActivationTokenSerial, ActivationTokenSerials, InputGrant,
    PendingActivationToken, SurfaceId, WakeReason, Waker,
};

use crate::driver::WaylandState;
use crate::surface::SeatLink;

/// Shared handle allowing live surfaces to mint and cancel external `xdg_activation_v1` tokens.
#[derive(Clone, Default)]
pub struct ActivationLink {
    inner: Option<Arc<ActivationInner>>,
}

struct ActivationInner {
    manager: XdgActivationV1,
    serials: ActivationTokenSerials,
    in_flight: Mutex<HashMap<ActivationTokenSerial, XdgActivationTokenV1>>,
}

impl core::fmt::Debug for ActivationLink {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ActivationLink")
            .field("supported", &self.inner.is_some())
            .finish_non_exhaustive()
    }
}

/// User data attached to an exported `xdg_activation_token_v1` protocol object.
pub struct ExternalTokenRequest {
    serial: ActivationTokenSerial,
}

impl ActivationLink {
    /// Wraps the bound `xdg_activation_v1` global when the compositor advertises one.
    pub(crate) fn new(activation: Option<&ActivationState>) -> Self {
        let manager = activation.and_then(|state| {
            <ActivationState as ProvidesBoundGlobal<XdgActivationV1, 1>>::bound_global(state).ok()
        });
        Self {
            inner: manager.map(|manager| {
                Arc::new(ActivationInner {
                    manager,
                    serials: ActivationTokenSerials::new(),
                    in_flight: Mutex::new(HashMap::new()),
                })
            }),
        }
    }

    pub(crate) fn redemption_manager(&self) -> Option<&XdgActivationV1> {
        self.inner.as_ref().map(|inner| &inner.manager).filter(|manager| manager.is_alive())
    }

    /// Synchronously commits an external `xdg_activation_token_v1` request on `conn`.
    #[expect(clippy::too_many_arguments, reason = "captures full wire context synchronously")]
    pub(crate) fn request_token(
        &self,
        conn: &Connection,
        qh: &QueueHandle<WaylandState>,
        wl_surface: &WlSurface,
        seat_link: &SeatLink,
        surface_id: SurfaceId,
        grant: InputGrant,
        app_id: Option<&str>,
    ) -> Option<PendingActivationToken> {
        let inner = self.inner.as_ref()?;
        if grant.surface() != surface_id || !wl_surface.is_alive() || !inner.manager.is_alive() {
            return None;
        }
        let seat = seat_link.seat_for_generation(grant.seat_generation())?;
        if !seat.is_alive() {
            return None;
        }
        let serial = inner.serials.take();
        {
            let mut in_flight = inner
                .in_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let token_obj = inner
                .manager
                .get_activation_token(qh, ExternalTokenRequest { serial });
            token_obj.set_surface(wl_surface);
            token_obj.set_serial(grant.serial().raw(), &seat);
            if let Some(app_id) = app_id.filter(|value| !value.is_empty()) {
                token_obj.set_app_id(app_id.to_owned());
            }
            token_obj.commit();
            in_flight.insert(serial, token_obj);
        }
        let _ = conn.flush();
        let cancel_inner = Arc::clone(inner);
        let cancel_conn = conn.clone();
        Some(PendingActivationToken::new(
            serial,
            Some(Arc::new(move || {
                let removed = cancel_inner
                    .in_flight
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&serial);
                if let Some(proxy) = removed
                    && proxy.is_alive()
                {
                    proxy.destroy();
                    let _ = cancel_conn.flush();
                }
            })),
        ))
    }

    pub(crate) fn take_in_flight(
        &self,
        serial: ActivationTokenSerial,
    ) -> Option<XdgActivationTokenV1> {
        let inner = self.inner.as_ref()?;
        inner
            .in_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&serial)
    }

    /// Destroys every still-pending external token object during connection shutdown.
    pub(crate) fn destroy_all(&self) {
        let Some(inner) = &self.inner else {
            return;
        };
        let drained: Vec<XdgActivationTokenV1> = inner
            .in_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain()
            .map(|(_, proxy)| proxy)
            .collect();
        for proxy in drained {
            if proxy.is_alive() {
                proxy.destroy();
            }
        }
    }
}

/// What a surface asked the desktop for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wanted {
    /// The window should be brought forward.
    Focus,
    /// The desktop should draw attention to the window.
    Attention,
}

impl WaylandState {
    /// Carries out every activation a surface asked for since the last turn.
    pub(crate) fn carry_out_activations(&mut self) {
        for (surface, focus) in self.link.take_activations() {
            self.activate(
                surface,
                if focus {
                    Wanted::Focus
                } else {
                    Wanted::Attention
                },
            );
        }
    }

    /// Asks for a token so that `surface` can be activated.
    ///
    /// The seat and the serial of the last press are quoted, because many compositors issue a
    /// token that does nothing for a request without one — and none of them says so. Asking anyway
    /// is still right: a token that turns out to be refused costs one round trip, and the
    /// alternative is a window that never comes forward even when the person asked it to.
    pub(crate) fn activate(&mut self, surface: SurfaceId, wanted: Wanted) {
        let Some(activation) = &self.activation else {
            return;
        };
        let Some(held) = self.live.surface(surface) else {
            return;
        };
        self.wanted.insert(surface, wanted);
        activation.request_token(
            &self.qh,
            RequestData {
                app_id: self.live.app_id.borrow().clone(),
                seat_and_serial: self.seat.serials.seat.clone().zip(self.seat.serials.grab()),
                surface: Some(held.wl_surface().clone()),
            },
        );
    }
}

impl ActivationHandler for WaylandState {
    type RequestData = RequestData;

    fn new_token(&mut self, token: String, data: &Self::RequestData) {
        let Some(activation) = &self.activation else {
            return;
        };
        let Some(surface) = data
            .surface
            .as_ref()
            .and_then(|surface| self.identify(surface))
        else {
            return;
        };
        // Whether the window comes forward or merely lights up is the compositor's decision, and
        // the request is the same either way. What is different is what the application asked for,
        // and it is dropped here rather than remembered: a token is good once.
        self.wanted.remove(&surface);
        if let Some(held) = self.live.surface(surface) {
            activation.activate::<Self>(held.wl_surface(), token);
        }
    }
}

impl Dispatch<XdgActivationTokenV1, ExternalTokenRequest, Self> for WaylandState {
    fn event(
        state: &mut Self,
        proxy: &XdgActivationTokenV1,
        event: <XdgActivationTokenV1 as Proxy>::Event,
        data: &ExternalTokenRequest,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            let claimed = state.activation_link.take_in_flight(data.serial).is_some();
            if proxy.is_alive() {
                proxy.destroy();
            }
            if claimed {
                Waker::wake(
                    state.live.waker.as_ref(),
                    WakeReason::ActivationToken {
                        serial: data.serial,
                        token: ActivationToken::new(token),
                    },
                );
            }
        }
    }
}

delegate_activation!(WaylandState);
