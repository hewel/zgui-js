//! Strict resource workers do not depend on frame/reactive polling.
use super::*;
use std::{collections::BTreeMap, sync::mpsc};
use std::io;

use crate::image_policy::files::{self, FileAccess};

use super::input::Input;

pub(super) struct Tasks {
    ready: Rc<RefCell<Vec<(SourceKey, u64, Arrival)>>>,
    policy: crate::ImagePolicy,
    strict: RefCell<Option<Batch>>,
    live: RefCell<Vec<zgui_reactive::Task>>,
    active: Arc<std::sync::atomic::AtomicBool>,
    seen_blob_revocations: std::cell::Cell<u64>,
    #[cfg(test)]
    gates: RefCell<std::collections::VecDeque<mpsc::Receiver<()>>>,
}

struct Batch {
    sender: mpsc::Sender<(u64, SourceKey, u64, Arrival)>,
    receiver: mpsc::Receiver<(u64, SourceKey, u64, Arrival)>,
    waiting: usize,
    next: u64,
    arrived: BTreeMap<u64, (SourceKey, u64, Arrival)>,
    work: mpsc::Sender<Box<dyn FnOnce() + Send>>,
}

fn canonical_source(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('/') && !name.contains([':', '\\', '?', '#'])
        && name.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

impl Tasks {
    pub(super) fn new(policy: crate::ImagePolicy) -> Rc<Self> {
        let strict = policy.strict.then(|| {
            let (sender, receiver) = mpsc::channel();
            let (work, jobs) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
            let jobs = Arc::new(std::sync::Mutex::new(jobs));
            for _ in 0..2 {
                let jobs = Arc::clone(&jobs);
                std::thread::spawn(move || loop {
                    let job = jobs.lock().expect("resource job queue poisoned").recv();
                    match job { Ok(job) => job(), Err(_) => break }
                });
            }
            Batch { sender, receiver, waiting: 0, next: 0, arrived: BTreeMap::new(), work }
        });
        Rc::new(Self {
            ready: Rc::new(RefCell::new(Vec::new())), policy, strict: RefCell::new(strict),
            live: RefCell::new(Vec::new()),
            active: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            seen_blob_revocations: std::cell::Cell::new(0),
            #[cfg(test)]
            gates: RefCell::new(std::collections::VecDeque::new()),
        })
    }

    pub(super) fn strict(&self) -> bool { self.policy.strict }

    pub(super) fn restricted(&self) -> bool { self.policy.assets.is_some() }

    pub(super) fn svg_fonts(&self) -> Option<Arc<crate::image_policy::SvgFonts>> {
        self.policy.svg_fonts.clone()
    }

    pub(super) fn clear_blobs(&self) {
        if let Some(blobs) = &self.policy.blobs { blobs.clear(); }
    }

    pub(super) fn blobs_were_revoked(&self) -> bool {
        self.policy.blobs.as_ref().is_some_and(|blobs| {
            let revocations = blobs.revocations();
            self.seen_blob_revocations.replace(revocations) != revocations
        })
    }

    pub(super) fn owns_blob(&self, uri: &str) -> bool {
        self.policy.blobs.as_ref().is_some_and(|blobs| blobs.contains(uri))
    }

    pub(super) fn cancel(&self) {
        self.active.store(false, std::sync::atomic::Ordering::Release);
        for task in self.live.borrow_mut().drain(..) { task.cancel(); }
        self.ready.borrow_mut().clear();
        self.strict.borrow_mut().take();
    }

    pub(super) fn declarations(&self, limit: usize) -> (Vec<String>, u64) {
        let Some(assets) = &self.policy.assets else { return (Vec::new(), 0); };
        (assets.keys().take(limit).cloned().collect(), assets.len().saturating_sub(limit) as u64)
    }

    pub(super) fn policy_error(&self, key: &SourceKey) -> Option<io::Error> {
        match key {
            SourceKey::File(uri) => (!self.policy.files.permits(uri)).then(|| files::denied(uri)),
            SourceKey::Blob(uri) => (!self.owns_blob(uri)).then(|| missing_blob(uri)),
            SourceKey::Path(name) if self.policy.assets.as_ref().is_some_and(|assets| {
                !canonical_source(name) || !assets.contains_key(name)
            }) => Some(undeclared(name)),
            SourceKey::Bytes(name) if self.restricted() => Some(undeclared(name)),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(super) fn hold_next(&self) -> mpsc::Sender<()> {
        let (release, gate) = mpsc::channel();
        self.gates.borrow_mut().push_back(gate);
        release
    }
    pub(super) fn looks_scalable(&self, key: &SourceKey) -> bool {
        match self.input(key) {
            Input::Dynamic(bytes) => super::input::is_svg_bytes(&bytes),
            _ => false,
        }
    }

    pub(super) fn input(&self, key: &SourceKey) -> Input {
        match key {
            SourceKey::File(uri) => match &self.policy.files {
                access @ FileAccess::Live(_) => Input::External(access.clone()),
                access => access.fixture(uri).map_or_else(|| Input::Denied(files::denied(uri)), Input::Dynamic),
            },
            SourceKey::Blob(uri) => self.policy.blobs.as_ref().and_then(|blobs| blobs.lookup(uri))
                .map_or_else(|| Input::Denied(missing_blob(uri)), Input::Dynamic),
            SourceKey::Path(name) => match &self.policy.assets {
                None => Input::UnrestrictedPath,
                Some(assets) => assets.get(name).filter(|_| canonical_source(name)).cloned()
                    .map_or_else(|| Input::Denied(undeclared(name)), Input::Package),
            },
            SourceKey::Bytes(name) if self.restricted() => Input::Denied(undeclared(name)),
            SourceKey::Bytes(url) => zgui_image::bytes_for_url(url).map_or_else(
                || Input::Denied(io::Error::new(io::ErrorKind::NotFound,
                    "the ImageBytes handle was dropped before the work ran")),
                Input::Registered,
            ),
        }
    }

    pub(super) fn start(self: &Rc<Self>, key: SourceKey, generation: u64, work: impl FnOnce(&SourceKey) -> Arrival + Send + 'static) {
        #[cfg(test)]
        let work = {
            let gate = self.gates.borrow_mut().pop_front();
            move |key: &SourceKey| {
                if let Some(gate) = gate { gate.recv().expect("test must release the native image worker"); }
                work(key)
            }
        };
        if let Some(batch) = self.strict.borrow_mut().as_mut() {
            batch.next += 1;
            let ticket = batch.next;
            batch.waiting += 1;
            let sender = batch.sender.clone();
            let active = Arc::clone(&self.active);
            // Starting here, rather than through spawn_local, avoids a throwaway RAF frame.
            let job = Box::new(move || {
                if !active.load(std::sync::atomic::Ordering::Acquire) { return; }
                let arrival = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&key)))
                    .unwrap_or_else(|_| Arrival::Probe(Err(zgui_image::DecodeError::Io(std::io::Error::other("native image worker panicked")))));
                if active.load(std::sync::atomic::Ordering::Acquire) {
                    let _ = sender.send((ticket, key, generation, arrival));
                }
            });
            batch.work.send(job).expect("native image worker queue disconnected");
        } else {
            let completed = Rc::clone(&self.ready);
            let work = zgui_reactive::blocking(move || {
                let arrival = work(&key);
                (key, generation, arrival)
            });
            let task = zgui_reactive::spawn_local(async move {
                let result = work.await;
                completed.borrow_mut().push(result);
            });
            let mut live = self.live.borrow_mut();
            live.retain(|task| !task.is_finished());
            live.push(task);
        }
    }

    pub(super) fn start_local(
        self: &Rc<Self>,
        key: SourceKey,
        generation: u64,
        future: impl std::future::Future<Output = Arrival> + 'static,
    ) {
        let completed = Rc::clone(&self.ready);
        let task = zgui_reactive::spawn_local(async move {
            let arrival = future.await;
            completed.borrow_mut().push((key, generation, arrival));
        });
        let mut live = self.live.borrow_mut();
        live.retain(|task| !task.is_finished());
        live.push(task);
    }

    pub(super) fn poll(&self) -> bool {
        let mut strict = self.strict.borrow_mut();
        let Some(batch) = strict.as_mut() else { return true; };
        while let Ok((ticket, key, generation, arrival)) = batch.receiver.try_recv() {
            batch.waiting -= 1;
            batch.arrived.insert(ticket, (key, generation, arrival));
        }
        if batch.waiting != 0 { return false; }
        // A partial batch is never observable, regardless of OS scheduling or codec speed.
        self.ready.borrow_mut().extend(std::mem::take(&mut batch.arrived).into_values());
        true
    }

    pub(super) fn has_ready(&self) -> bool { !self.ready.borrow().is_empty() }
    pub(super) fn take(&self) -> Vec<(SourceKey, u64, Arrival)> {
        self.live.borrow_mut().retain(|task| !task.is_finished());
        std::mem::take(&mut *self.ready.borrow_mut())
    }
}

impl Drop for Tasks {
    fn drop(&mut self) { self.cancel(); }
}

fn undeclared(name: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!("image source is not a declared package asset: {name}"))
}

pub(super) fn missing_blob(uri: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("image handle is released or not owned by this session: {uri}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_real_decodes_publish_as_one_complete_launch_ordered_batch() {
        let tasks = Tasks::new(crate::ImagePolicy::package_only([]));
        let (release, delayed) = mpsc::channel();
        let (finished, second_finished) = mpsc::channel();
        let raster = include_bytes!("../../tests/fixtures/red2x2.png");
        tasks.start(SourceKey::of("first.png"), 1, move |_| {
            delayed.recv().unwrap();
            Arrival::Variant(64, 1, zgui_image::decode_scaled(raster, 64, zgui_image::Limits::default())
                .map(|decoded| DecodedVariant { decoded, mips: Vec::new() }))
        });
        tasks.start(SourceKey::of("second.png"), 2, move |_| {
            let decoded = zgui_image::decode_scaled(raster, 64, zgui_image::Limits::default());
            finished.send(()).unwrap();
            Arrival::Variant(64, 2, decoded.map(|decoded| DecodedVariant { decoded, mips: Vec::new() }))
        });
        second_finished.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        assert!(!tasks.poll(), "the first decode is still deliberately delayed");
        assert!(tasks.take().is_empty(), "partial worker arrival must be unobservable");
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !tasks.poll() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        let published = tasks.take();
        assert_eq!(published.iter().map(|(key, _, _)| key.name()).collect::<Vec<_>>(), ["first.png", "second.png"]);
        for (_, _, arrival) in published {
            let Arrival::Variant(_, _, Ok(decoded)) = arrival else { panic!("real native decode failed"); };
            assert_eq!((decoded.decoded.size.width, decoded.decoded.size.height), (2, 2));
            assert_eq!(&decoded.decoded.texels[..4], &[255, 0, 0, 255]);
        }
    }
}
