use std::fmt;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rustc_hash::FxHashMap;

/// The session-owned encoded bytes behind opaque `zgui-blob://` image handles.
#[derive(Clone)]
pub struct ImageBlobs(Arc<Registry>);

struct Registry {
    prefix: String,
    assets: Mutex<FxHashMap<u64, Arc<[u8]>>>,
    revocations: AtomicU64,
}

static NEXT_BLOB: AtomicU64 = AtomicU64::new(1);

impl ImageBlobs {
    /// Creates an empty and independent table for a session generation.
    pub fn new(generation: u64) -> Self {
        Self(Arc::new(Registry {
            prefix: format!("zgui-blob://{generation:x}-"),
            assets: Mutex::new(FxHashMap::default()),
            revocations: AtomicU64::new(0),
        }))
    }

    /// Registers immutable encoded bytes without copying them into the JavaScript heap.
    ///
    /// # Errors
    /// Returns an error if the process has exhausted its handle identifiers.
    pub fn register(&self, bytes: Arc<[u8]>) -> io::Result<String> {
        let mut id = NEXT_BLOB.load(Ordering::Relaxed);
        loop {
            let next = id
                .checked_add(1)
                .ok_or_else(|| io::Error::other("image handle identifiers exhausted"))?;
            match NEXT_BLOB.compare_exchange_weak(id, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(current) => id = current,
            }
        }
        self.assets().insert(id, bytes);
        Ok(format!("{}{id:x}", self.0.prefix))
    }

    /// Unregisters one handle idempotently; the next native settle retires its decoded pixels.
    /// Returns `true` only on the first release of a handle owned by this table.
    pub fn release(&self, src: &str) -> bool {
        let Some(id) = self.id(src) else {
            return false;
        };
        let removed = self.assets().remove(&id);
        if removed.is_some() {
            self.0.revocations.fetch_add(1, Ordering::Release);
        }
        removed.is_some()
    }

    /// Returns how many encoded buffers remain registered in this session.
    pub fn len(&self) -> usize {
        self.assets().len()
    }

    /// Returns whether every handle has been released.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Unregisters every handle from this session and all its table clones.
    pub fn clear(&self) {
        // Drop potentially large buffers without holding the table lock.
        let removed = std::mem::take(&mut *self.assets());
        if !removed.is_empty() {
            self.0.revocations.fetch_add(1, Ordering::Release);
        }
    }

    pub(crate) fn contains(&self, src: &str) -> bool {
        self.id(src)
            .is_some_and(|id| self.assets().contains_key(&id))
    }

    pub(crate) fn lookup(&self, src: &str) -> Option<Arc<[u8]>> {
        self.assets().get(&self.id(src)?).cloned()
    }

    pub(crate) fn revocations(&self) -> u64 {
        self.0.revocations.load(Ordering::Acquire)
    }

    fn id(&self, src: &str) -> Option<u64> {
        let id = src.strip_prefix(&self.0.prefix)?;
        if id.starts_with('0') || !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return None;
        }
        u64::from_str_radix(id, 16).ok()
    }

    fn assets(&self) -> MutexGuard<'_, FxHashMap<u64, Arc<[u8]>>> {
        self.0.assets.lock().expect("image handle table poisoned")
    }
}

impl fmt::Debug for ImageBlobs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageBlobs")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}
