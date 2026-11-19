//! Compositor session-lock lifecycle, independent of application surfaces.

/// The status of the platform's session-lock request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionLockState {
    /// No acquired or pending lock owned by this platform connection.
    #[default]
    Unlocked,
    /// The compositor has not yet confirmed or refused acquisition.
    Pending,
    /// The compositor confirmed that ordinary content is hidden on every output.
    Locked,
    /// The compositor refused acquisition before confirming a lock.
    Refused,
    /// The compositor terminated a previously acquired lock on its own authority.
    Finished,
    /// An explicit unlock was sent but acknowledgement is not yet known.
    Unlocking,
}

impl SessionLockState {
    /// The stable lowercase status used by host APIs and preview evidence.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unlocked => "unlocked",
            Self::Pending => "pending",
            Self::Locked => "locked",
            Self::Refused => "refused",
            Self::Finished => "finished",
            Self::Unlocking => "unlocking",
        }
    }

    /// Whether the host must preserve lock coverage and avoid normal desktop behavior.
    ///
    /// This includes pending acquisition, an acquired or finished lock, and an unlock
    /// awaiting acknowledgement. Refusal and acknowledged unlock do not retain the lock.
    #[must_use]
    pub const fn is_retained(self) -> bool {
        match self {
            Self::Pending | Self::Locked | Self::Finished | Self::Unlocking => true,
            Self::Unlocked | Self::Refused => false,
        }
    }
}
