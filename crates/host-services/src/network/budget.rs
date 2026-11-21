//! Session-wide native byte-retention budget with RAII leases.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Default maximum native bytes retained across all active network streams in one session.
pub const DEFAULT_SESSION_NETWORK_BYTES: usize = 16 * 1024 * 1024;

/// Shared atomic byte-retention ledger for one session generation.
#[derive(Clone, Debug)]
pub struct SessionByteBudget {
    limit: usize,
    used: Arc<AtomicUsize>,
}

impl SessionByteBudget {
    /// Creates a new budget capped at `limit` bytes.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self { limit, used: Arc::new(AtomicUsize::new(0)) }
    }

    /// Returns the configured byte limit.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Returns currently retained bytes.
    #[must_use]
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    /// Atomically reserves `bytes`, returning an RAII lease or `None` when the cap would be exceeded.
    pub fn try_reserve(&self, bytes: usize) -> Option<ByteLease> {
        if bytes == 0 {
            return Some(ByteLease { budget: self.clone(), bytes: 0 });
        }
        let mut current = self.used.load(Ordering::Acquire);
        loop {
            let next = current.checked_add(bytes)?;
            if next > self.limit {
                return None;
            }
            match self.used.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(ByteLease { budget: self.clone(), bytes }),
                Err(observed) => current = observed,
            }
        }
    }
}

impl Default for SessionByteBudget {
    fn default() -> Self {
        Self::new(DEFAULT_SESSION_NETWORK_BYTES)
    }
}

/// RAII reservation that automatically releases its bytes back to [`SessionByteBudget`] on drop.
#[derive(Debug)]
pub struct ByteLease {
    budget: SessionByteBudget,
    bytes: usize,
}

impl ByteLease {
    /// Returns the reserved byte count.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Releases a portion of the lease early.
    pub fn shrink(&mut self, release_bytes: usize) {
        let delta = release_bytes.min(self.bytes);
        if delta > 0 {
            self.bytes -= delta;
            self.budget.used.fetch_sub(delta, Ordering::AcqRel);
        }
    }
}

impl Drop for ByteLease {
    fn drop(&mut self) {
        if self.bytes > 0 {
            self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
            self.bytes = 0;
        }
    }
}
