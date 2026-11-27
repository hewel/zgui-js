//! The token of an input event, for a request the desktop grants only against one.

use core::fmt::{self, Debug};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::surface::SurfaceId;

/// Which press of the user a request answers.
///
/// Some desktops grant a request only when it quotes an input event the user caused: a pop-up
/// that takes the pointer and the keyboard, an interactive move, a claim on a selection. This is
/// the opaque token a backend gives such an event. Read it with
/// [`Surface::input_serial`](crate::Surface::input_serial) in the handler of the press that opens
/// the pop-up, and pass it on with the request.
///
/// A desktop that needs no token never gives one, and a request that carries one there ignores
/// it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputSerial(u32);

impl InputSerial {
    /// The token with the given raw value.
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw value, for the backend that gave it.
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl Debug for InputSerial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "InputSerial({})", self.0)
    }
}

/// Unforgeable proof of a physical press delivered on a specific seat generation and surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InputGrant {
    seat_generation: u64,
    surface: SurfaceId,
    serial: InputSerial,
}

impl InputGrant {
    /// Records a physical press grant for `surface` on `seat_generation` with `serial`.
    #[must_use]
    pub const fn new(seat_generation: u64, surface: SurfaceId, serial: InputSerial) -> Self {
        Self {
            seat_generation,
            surface,
            serial,
        }
    }

    /// Which adopted-seat incarnation produced this grant.
    #[must_use]
    pub const fn seat_generation(self) -> u64 {
        self.seat_generation
    }

    /// Which surface received the physical press.
    #[must_use]
    pub const fn surface(self) -> SurfaceId {
        self.surface
    }

    /// The platform serial of the physical press.
    #[must_use]
    pub const fn serial(self) -> InputSerial {
        self.serial
    }
}

/// Connection-unique identifier correlating an external activation token request to its completion.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActivationTokenSerial(u64);

impl ActivationTokenSerial {
    /// Creates an activation token request identifier from `raw`.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw identifier value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl Debug for ActivationTokenSerial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ActivationTokenSerial({})", self.0)
    }
}

/// Monotonic generator of [`ActivationTokenSerial`] identifiers.
#[derive(Debug)]
pub struct ActivationTokenSerials {
    next: AtomicU64,
}

impl Default for ActivationTokenSerials {
    fn default() -> Self {
        Self::new()
    }
}

impl ActivationTokenSerials {
    /// Creates a generator starting at `1`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }

    /// Allocates the next unique [`ActivationTokenSerial`].
    #[must_use]
    pub fn take(&self) -> ActivationTokenSerial {
        ActivationTokenSerial(self.next.fetch_add(1, Ordering::Relaxed))
    }
}

/// Opaque single-use external activation token string whose [`Debug`] form is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct ActivationToken(String);

impl ActivationToken {
    /// Wraps an exported activation token string.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// Borrows the raw token string for IPC or environment delivery.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns the raw token string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl Debug for ActivationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ActivationToken(<redacted>)")
    }
}

/// Handle to an in-flight external activation token request on a platform surface.
pub struct PendingActivationToken {
    serial: ActivationTokenSerial,
    cancel: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl PendingActivationToken {
    /// Pairs `serial` with an optional backend cancellation hook that destroys any protocol object.
    #[must_use]
    pub fn new(
        serial: ActivationTokenSerial,
        cancel: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Self {
        Self { serial, cancel }
    }

    /// Identifier matched against [`WakeReason::ActivationToken`](crate::WakeReason::ActivationToken).
    #[must_use]
    pub const fn serial(&self) -> ActivationTokenSerial {
        self.serial
    }

    /// Abandons the pending token request and releases any underlying protocol resource.
    pub fn cancel(&self) {
        if let Some(cancel) = &self.cancel {
            cancel();
        }
    }
}

impl Debug for PendingActivationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingActivationToken")
            .field("serial", &self.serial)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::{ActivationToken, ActivationTokenSerials, InputGrant, InputSerial};
    use crate::surface::SurfaceId;

    #[test]
    fn a_serial_round_trips_and_compares() {
        assert_eq!(InputSerial::new(41).raw(), 41);
        assert_ne!(InputSerial::new(1), InputSerial::new(2));
        assert_eq!(format!("{:?}", InputSerial::new(3)), "InputSerial(3)");
    }

    #[test]
    fn activation_token_redacts_debug_and_generators_advance() {
        let token = ActivationToken::new("secret-token-123");
        assert_eq!(format!("{token:?}"), "ActivationToken(<redacted>)");
        assert_eq!(token.as_str(), "secret-token-123");
        assert_eq!(token.into_string(), "secret-token-123");

        let serials = ActivationTokenSerials::new();
        assert_eq!(serials.take().raw(), 1);
        assert_eq!(serials.take().raw(), 2);

        let grant = InputGrant::new(3, SurfaceId::new(9), InputSerial::new(77));
        assert_eq!(grant.seat_generation(), 3);
        assert_eq!(grant.surface(), SurfaceId::new(9));
        assert_eq!(grant.serial(), InputSerial::new(77));
    }
}
