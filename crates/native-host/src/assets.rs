//! Cohesive live/fixture text-asset seam: one provider contract for request
//! admission, delivery, cancellation and retirement. `AssetSource::bind` is the
//! only place that selects live workers versus deterministic fixtures.
use std::{cell::{Cell, RefCell}, rc::Rc, sync::{Arc, atomic::AtomicBool, mpsc}};

use host_services::{Completion, Executor, Request, ServiceError, SubmitError};

use crate::package::Package;

pub(crate) mod fixture;

pub(crate) const MAX_SAFE_ID: u64 = 9_007_199_254_740_991;
const MAX_REQUESTS: usize = 64;
const MAX_ASSET_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContractFailure {
    pub code: &'static str,
    pub message: String,
}
impl std::fmt::Display for ContractFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ContractFailure {}

/// Selects the authority a session generation binds to. Live access runs the
/// shared worker pool; fixtures keep every call on the owner thread. A fixture
/// path never spawns a live executor or touches live disk.
#[derive(Clone)]
pub(crate) enum AssetSource {
    Live(host_services::Executor),
    Fixture(Rc<fixture::FixtureRuntime>),
}
impl AssetSource {
    /// Bind one provider instance to a session generation. `wake` is the
    /// owner-thread publication signal for live completions.
    pub(crate) fn bind(&self, package: Option<Rc<Package>>, generation: u64, wake: Arc<dyn Fn() + Send + Sync>) -> Box<dyn AssetProvider> {
        match self {
            Self::Live(executor) => {
                let (sender, inbox) = mpsc::sync_channel(MAX_REQUESTS);
                Box::new(LiveAssets {
                    package, generation, executor: executor.clone(),
                    sender: RefCell::new(Some(sender)), inbox: RefCell::new(Some(inbox)),
                    pending_completion: RefCell::new(None), next_id: Cell::new(1), wake,
                })
            }
            Self::Fixture(runtime) => Box::new(FixtureAssets { package, generation, runtime: runtime.clone() }),
        }
    }
}

/// Provider rejection, classified for the JavaScript exception mapping.
#[derive(Debug)]
pub(crate) enum AssetError { Type(String), Range(String), Contract(ContractFailure) }

/// How the owner closed out a completion for provider bookkeeping.
pub(crate) enum Delivery { Delivered { failed: bool }, Ignored(&'static str) }

/// One session generation's text-asset authority. Adapters hold no JS roots;
/// the owner keeps Promise resolve/reject pairs and cancellation flags.
pub(crate) trait AssetProvider {
    /// Validate the JS call shape before string conversion.
    fn validate_arguments(&self, count: usize, string_path: bool) -> Result<(), AssetError>;
    /// Admission checks that run before an ID is allocated.
    fn validate_request(&self, path: &str, creation_allowed: bool, pending: usize) -> Result<(), AssetError>;
    /// The next registration ID. Fixture IDs come from the run-wide counter
    /// shared with native timer and frame registrations; live IDs are per
    /// session and bounded by `MAX_SAFE_ID`.
    fn registration_id(&self) -> Option<u64>;
    /// Submit request `id`. `Ok(None)` accepts it; `Ok(Some(_))` settles the
    /// promise immediately with the returned service error and stores no
    /// pending request; `Err` throws synchronously.
    fn submit(&self, id: u64, path: String, cancelled: Arc<AtomicBool>) -> Result<Option<ServiceError>, AssetError>;
    fn next_completion(&self) -> Option<Completion>;
    fn has_completions(&self) -> bool;
    fn cancel(&self, id: u64);
    /// Record how the owner closed out a completion.
    fn settled(&self, id: u64, delivery: Delivery);
    /// Release this generation's registrations. Idempotent; the fixture
    /// runtime's history survives, only this generation is cancelled.
    fn stop(&self);
    fn contract_failure(&self) -> Option<ContractFailure>;
}

/// Bounded worker-queue adapter: package-declared disk reads on the shared
/// executor with a per-generation completion inbox.
struct LiveAssets {
    package: Option<Rc<Package>>,
    generation: u64,
    executor: Executor,
    sender: RefCell<Option<mpsc::SyncSender<Completion>>>,
    inbox: RefCell<Option<mpsc::Receiver<Completion>>>,
    pending_completion: RefCell<Option<Completion>>,
    next_id: Cell<u64>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl AssetProvider for LiveAssets {
    fn validate_arguments(&self, _count: usize, _string_path: bool) -> Result<(), AssetError> {
        // Live arguments use the ordinary FromJs conversion.
        Ok(())
    }
    fn validate_request(&self, path: &str, creation_allowed: bool, pending: usize) -> Result<(), AssetError> {
        if !creation_allowed { return Err(AssetError::Type("host work is unavailable during preflight or disposal".into())); }
        let Some(package) = &self.package else { return Err(AssetError::Type("asset requests require a loaded package".into())); };
        if !package.manifest.assets.iter().any(|asset| asset == path) { return Err(AssetError::Range("asset is not declared in this package".into())); }
        if pending >= MAX_REQUESTS { return Err(AssetError::Range("outstanding request limit reached".into())); }
        Ok(())
    }
    fn registration_id(&self) -> Option<u64> {
        let id = self.next_id.get();
        if id > MAX_SAFE_ID { return None; }
        self.next_id.set(id + 1);
        Some(id)
    }
    fn submit(&self, id: u64, path: String, cancelled: Arc<AtomicBool>) -> Result<Option<ServiceError>, AssetError> {
        let Some(package) = &self.package else { return Err(AssetError::Type("asset requests require a loaded package".into())); };
        let Some(reply) = self.sender.borrow().as_ref().cloned() else {
            return Ok(Some(ServiceError { code: "Stopped", message: SubmitError::Stopped.to_string() }));
        };
        match self.executor.submit(Request {
            generation: self.generation, request_id: id, root: package.directory.clone(), path,
            max_bytes: MAX_ASSET_BYTES, cancelled, reply, wake: self.wake.clone(),
        }) {
            Ok(()) => Ok(None),
            Err(SubmitError::QueueFull) => Ok(Some(ServiceError { code: "Busy", message: SubmitError::QueueFull.to_string() })),
            Err(SubmitError::Stopped) => Ok(Some(ServiceError { code: "Stopped", message: SubmitError::Stopped.to_string() })),
        }
    }
    fn next_completion(&self) -> Option<Completion> {
        self.pending_completion.borrow_mut().take().or_else(|| self.inbox.borrow().as_ref()?.try_recv().ok())
    }
    fn has_completions(&self) -> bool {
        if self.pending_completion.borrow().is_some() { return true; }
        let completion = self.inbox.borrow().as_ref().and_then(|inbox| inbox.try_recv().ok());
        let pending = completion.is_some();
        *self.pending_completion.borrow_mut() = completion;
        pending
    }
    fn cancel(&self, _id: u64) {
        // Live cancellation rides the request's shared flag; the worker checks
        // it before reading and before replying.
    }
    fn settled(&self, _id: u64, _delivery: Delivery) {
        // Live has no trace ledger; ignored or duplicated completions expire
        // with the request they describe.
    }
    fn stop(&self) {
        // Retired workers must not block the shared executor on a full old inbox.
        self.inbox.borrow_mut().take();
        self.sender.borrow_mut().take();
        self.pending_completion.borrow_mut().take();
    }
    fn contract_failure(&self) -> Option<ContractFailure> { None }
}

/// Owner-thread adapter: deterministic asset behavior, scheduled state events
/// and the exact call ledger on the run-wide `FixtureRuntime`.
struct FixtureAssets {
    package: Option<Rc<Package>>,
    generation: u64,
    runtime: Rc<fixture::FixtureRuntime>,
}
impl FixtureAssets {
    fn malformed(&self, message: impl Into<String>) -> AssetError {
        AssetError::Contract(self.runtime.malformed(message))
    }
}
impl AssetProvider for FixtureAssets {
    fn validate_arguments(&self, count: usize, string_path: bool) -> Result<(), AssetError> {
        if count != 1 || !string_path { return Err(self.malformed("readTextAsset requires exactly one string path")); }
        Ok(())
    }
    fn validate_request(&self, _path: &str, creation_allowed: bool, pending: usize) -> Result<(), AssetError> {
        self.runtime.require_assets().map_err(AssetError::Contract)?;
        if !creation_allowed { return Err(self.malformed("readTextAsset is unavailable during preflight or disposal")); }
        if pending >= MAX_REQUESTS { return Err(self.malformed("outstanding request limit reached")); }
        Ok(())
    }
    fn registration_id(&self) -> Option<u64> { self.runtime.registration_id() }
    fn submit(&self, id: u64, path: String, _cancelled: Arc<AtomicBool>) -> Result<Option<ServiceError>, AssetError> {
        self.runtime.read_asset(self.generation, id, &path, self.package.as_deref()).map_err(AssetError::Contract)?;
        Ok(None)
    }
    fn next_completion(&self) -> Option<Completion> { self.runtime.take() }
    fn has_completions(&self) -> bool { self.runtime.has_delivery() }
    fn cancel(&self, id: u64) { self.runtime.cancel(id); }
    fn settled(&self, id: u64, delivery: Delivery) {
        match delivery {
            Delivery::Delivered { failed } => self.runtime.delivered(id, failed),
            Delivery::Ignored(reason) => self.runtime.ignored(id, reason),
        }
    }
    // Retire only this generation's registrations; the ledger and scheduled
    // events are run-wide state and survive session replacement.
    fn stop(&self) { self.runtime.cancel_generation(self.generation); }
    fn contract_failure(&self) -> Option<ContractFailure> { self.runtime.contract_failure() }
}
