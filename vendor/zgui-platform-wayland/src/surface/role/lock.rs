//! Disposable application content above a platform-owned lock surface.

use std::sync::{Mutex, PoisonError};

use wayland_client::protocol::{wl_subsurface::WlSubsurface, wl_surface::WlSurface};

/// A claim's subsurface. Dropping or detaching it does not affect the lock role underneath.
#[derive(Debug)]
pub struct LockContent {
    wl: WlSurface,
    child: Mutex<Option<WlSubsurface>>,
}

impl LockContent {
    pub(crate) fn new(wl: WlSurface, child: WlSubsurface) -> Self {
        child.set_desync();
        Self {
            wl,
            child: Mutex::new(Some(child)),
        }
    }

    pub(crate) fn wl_surface(&self) -> &WlSurface {
        &self.wl
    }

    pub(crate) fn hide(&self) {
        if self.child.lock().expect("lock content poisoned").is_some() {
            // This is an ordinary subsurface; the locked parent always has a non-null buffer.
            self.wl.attach(None, 0, 0);
            self.wl.commit();
        }
    }

    pub(crate) fn detach(&self) {
        // Even poisoned content must be unmapped so the platform fallback is exposed.
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(child) = child.take() {
            // Unmap immediately, even if a renderer still holds the wl_surface handle.
            child.destroy();
        }
    }
}

impl Drop for LockContent {
    fn drop(&mut self) {
        let child = self.child.get_mut().unwrap_or_else(PoisonError::into_inner);
        if let Some(child) = child.take() {
            child.destroy();
        }
        self.wl.destroy();
    }
}
