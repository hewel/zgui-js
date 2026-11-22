//! Owner-thread origin-allowlisted HTTP, SSE, and WebSocket bindings with staged deferral
//! and fair per-registration checkpoint delivery.
use super::{Mode, Work, data_property, owner, service_error};
use crate::{
    assets::{
        AssetSource,
        fixture::{FixtureRuntime, NetworkDelivery},
    },
    package::Package,
};
use host_services::network::{
    DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_MAX_SSE_EVENT_BYTES, DEFAULT_MAX_WS_MESSAGE_BYTES,
    DEFAULT_MAX_WS_OUTBOUND_BYTES, EventStreamHandle, EventStreamSpec, HttpEvent, HttpHandle,
    HttpRequestSpec, HttpResponseHead, InboundPayload, NetworkError, NetworkRuntime, OriginPolicy,
    OutboundPayload, SessionByteBudget, SseEvent, SseMessage, WebSocketEvent, WebSocketHandle,
    WebSocketSpec,
};
use rquickjs::{
    Array, Ctx, Exception, Function, IntoJs, Object, Persistent, TypedArray, Value,
    function::{Func, Opt},
};
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::{BTreeMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

const MAX_CONCURRENT_HTTP: usize = 64;
const MAX_CONCURRENT_SSE: usize = 32;
const MAX_CONCURRENT_WS: usize = 32;
const MAX_REQUEST_BODY_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_REDIRECTS: u8 = 5;

#[derive(Clone, Copy, Default)]
pub(crate) struct NetworkCounts {
    pub(crate) http_requests: usize,
    pub(crate) event_streams: usize,
    pub(crate) web_sockets: usize,
}

enum NetworkBackend {
    Live {
        runtime: OnceCell<NetworkRuntime>,
        budget: SessionByteBudget,
    },
    Preview {
        fixture: Rc<FixtureRuntime>,
    },
}

struct PendingPromise {
    resolve: Persistent<Function<'static>>,
    reject: Persistent<Function<'static>>,
}

struct HttpEntry {
    head: Option<PendingPromise>,
    chunk: Option<PendingPromise>,
    handle: Option<HttpHandle>,
    staged_pulls: usize,
}

struct SseEntry {
    on_open: Option<Persistent<Function<'static>>>,
    on_message: Persistent<Function<'static>>,
    on_error: Option<Persistent<Function<'static>>>,
    on_close: Option<Persistent<Function<'static>>>,
    handle: Option<EventStreamHandle>,
}

struct WsEntry {
    url: String,
    on_open: Option<Persistent<Function<'static>>>,
    on_message: Persistent<Function<'static>>,
    on_error: Option<Persistent<Function<'static>>>,
    on_close: Option<Persistent<Function<'static>>>,
    handle: Option<WebSocketHandle>,
    staged_bytes: usize,
    max_outbound_bytes: usize,
}

enum StagedOp {
    StartHttp { id: u64, spec: HttpRequestSpec },
    PullHttp { id: u64 },
    StartSse { id: u64, spec: EventStreamSpec },
    StartWs { id: u64, spec: WebSocketSpec },
    WsSendText { id: u64, text: String },
    WsSendBinary { id: u64, bytes: Vec<u8> },
    WsClose { id: u64, code: u16, reason: String },
}

pub(crate) enum NetworkEvent {
    HttpHead { generation: u64, id: u64, head: HttpResponseHead },
    HttpChunk { generation: u64, id: u64, data: Vec<u8> },
    HttpEnd { generation: u64, id: u64 },
    HttpError { generation: u64, id: u64, error: NetworkError },
    SseOpen { generation: u64, id: u64, status: u16, url: String, headers: Vec<(String, String)> },
    SseMessage { generation: u64, id: u64, message: SseMessage },
    SseError { generation: u64, id: u64, error: NetworkError },
    SseClose { generation: u64, id: u64 },
    WsOpen { generation: u64, id: u64, url: String, protocol: Option<String> },
    WsText { generation: u64, id: u64, text: String },
    WsBinary { generation: u64, id: u64, data: Vec<u8> },
    WsError { generation: u64, id: u64, error: NetworkError },
    WsClose { generation: u64, id: u64, code: u16, reason: String },
}

pub(super) struct NetworkState {
    backend: NetworkBackend,
    policy: OriginPolicy,
    staging: Cell<bool>,
    staged_ops: RefCell<VecDeque<StagedOp>>,
    http: RefCell<BTreeMap<u64, HttpEntry>>,
    sse: RefCell<BTreeMap<u64, SseEntry>>,
    ws: RefCell<BTreeMap<u64, WsEntry>>,
    local_events: RefCell<VecDeque<NetworkEvent>>,
    poll_cursor: Cell<u64>,
}

impl NetworkState {
    pub(super) fn new(package: Option<Rc<Package>>, source: &AssetSource) -> Self {
        let policy = package
            .as_ref()
            .and_then(|pkg| pkg.manifest.capabilities.network.to_origin_policy().ok())
            .unwrap_or_default();
        let backend = match source {
            AssetSource::Fixture(fixture) => NetworkBackend::Preview {
                fixture: fixture.clone(),
            },
            AssetSource::Live(_) => NetworkBackend::Live {
                runtime: OnceCell::new(),
                budget: SessionByteBudget::default(),
            },
        };
        Self {
            backend,
            policy,
            staging: Cell::new(false),
            staged_ops: RefCell::new(VecDeque::new()),
            http: RefCell::new(BTreeMap::new()),
            sse: RefCell::new(BTreeMap::new()),
            ws: RefCell::new(BTreeMap::new()),
            local_events: RefCell::new(VecDeque::new()),
            poll_cursor: Cell::new(0),
        }
    }

    pub(super) fn begin_staged_mount(&self) {
        self.staging.set(true);
    }

    pub(super) fn commit_staged_mount(&self, generation: u64, wake: &Arc<dyn Fn() + Send + Sync>) {
        self.staging.set(false);
        let ops: Vec<StagedOp> = self.staged_ops.borrow_mut().drain(..).collect();
        let mut woke = false;
        for op in ops {
            woke = true;
            match op {
                StagedOp::StartHttp { id, spec } => {
                    if !self.http.borrow().contains_key(&id) {
                        continue;
                    }
                    self.dispatch_http(generation, id, spec, wake);
                }
                StagedOp::PullHttp { id } => {
                    if !self.http.borrow().contains_key(&id) {
                        continue;
                    }
                    self.dispatch_http_pull(id);
                }
                StagedOp::StartSse { id, spec } => {
                    if !self.sse.borrow().contains_key(&id) {
                        continue;
                    }
                    self.dispatch_sse(generation, id, spec, wake);
                }
                StagedOp::StartWs { id, spec } => {
                    if !self.ws.borrow().contains_key(&id) {
                        continue;
                    }
                    self.dispatch_ws(generation, id, spec, wake);
                }
                StagedOp::WsSendText { id, text } => {
                    let _ = self.dispatch_ws_send_text(id, text);
                }
                StagedOp::WsSendBinary { id, bytes } => {
                    let _ = self.dispatch_ws_send_binary(id, bytes);
                }
                StagedOp::WsClose { id, code, reason } => {
                    self.dispatch_ws_close(id, code, reason);
                }
            }
        }
        if woke {
            wake();
        }
    }

    fn dispatch_http(
        &self,
        generation: u64,
        id: u64,
        spec: HttpRequestSpec,
        wake: &Arc<dyn Fn() + Send + Sync>,
    ) {
        match &self.backend {
            NetworkBackend::Live { runtime, budget } => {
                let rt = runtime.get_or_init(NetworkRuntime::new);
                match rt.http_request(spec, self.policy.clone(), budget.clone(), wake.clone()) {
                    Ok(handle) => {
                        if let Some(entry) = self.http.borrow_mut().get_mut(&id) {
                            let pulls = entry.staged_pulls;
                            entry.staged_pulls = 0;
                            for _ in 0..pulls {
                                handle.request_next_chunk();
                            }
                            entry.handle = Some(handle);
                        }
                    }
                    Err(error) => {
                        self.local_events.borrow_mut().push_back(NetworkEvent::HttpError {
                            generation,
                            id,
                            error,
                        });
                    }
                }
            }
            NetworkBackend::Preview { fixture } => {
                if let Err(failure) = fixture.start_http_fixture(
                    generation,
                    id,
                    &spec.method,
                    &spec.url,
                    &spec.body,
                    spec.max_response_bytes,
                ) {
                    self.local_events.borrow_mut().push_back(NetworkEvent::HttpError {
                        generation,
                        id,
                        error: NetworkError::new(failure.code, failure.message),
                    });
                } else if let Some(entry) = self.http.borrow_mut().get_mut(&id) {
                    let pulls = entry.staged_pulls;
                    entry.staged_pulls = 0;
                    for _ in 0..pulls {
                        let _ = fixture.pull_http_fixture_chunk(id);
                    }
                }
            }
        }
    }

    fn dispatch_http_pull(&self, id: u64) {
        match &self.backend {
            NetworkBackend::Live { .. } => {
                if let Some(entry) = self.http.borrow().get(&id)
                    && let Some(handle) = &entry.handle
                {
                    handle.request_next_chunk();
                }
            }
            NetworkBackend::Preview { fixture } => {
                let _ = fixture.pull_http_fixture_chunk(id);
            }
        }
    }

    fn dispatch_sse(
        &self,
        generation: u64,
        id: u64,
        spec: EventStreamSpec,
        wake: &Arc<dyn Fn() + Send + Sync>,
    ) {
        match &self.backend {
            NetworkBackend::Live { runtime, budget } => {
                let rt = runtime.get_or_init(NetworkRuntime::new);
                match rt.open_event_stream(spec, self.policy.clone(), budget.clone(), wake.clone()) {
                    Ok(handle) => {
                        if let Some(entry) = self.sse.borrow_mut().get_mut(&id) {
                            entry.handle = Some(handle);
                        }
                    }
                    Err(error) => {
                        self.local_events.borrow_mut().push_back(NetworkEvent::SseError {
                            generation,
                            id,
                            error,
                        });
                    }
                }
            }
            NetworkBackend::Preview { fixture } => {
                if let Err(failure) = fixture.start_sse_fixture(generation, id, &spec.url) {
                    self.local_events.borrow_mut().push_back(NetworkEvent::SseError {
                        generation,
                        id,
                        error: NetworkError::new(failure.code, failure.message),
                    });
                }
            }
        }
    }

    fn dispatch_ws(
        &self,
        generation: u64,
        id: u64,
        spec: WebSocketSpec,
        wake: &Arc<dyn Fn() + Send + Sync>,
    ) {
        match &self.backend {
            NetworkBackend::Live { runtime, budget } => {
                let rt = runtime.get_or_init(NetworkRuntime::new);
                match rt.connect_websocket(spec, self.policy.clone(), budget.clone(), wake.clone()) {
                    Ok(handle) => {
                        if let Some(entry) = self.ws.borrow_mut().get_mut(&id) {
                            entry.handle = Some(handle);
                        }
                    }
                    Err(error) => {
                        self.local_events.borrow_mut().push_back(NetworkEvent::WsError {
                            generation,
                            id,
                            error,
                        });
                    }
                }
            }
            NetworkBackend::Preview { fixture } => {
                if let Err(failure) = fixture.start_ws_fixture(generation, id, &spec.url) {
                    self.local_events.borrow_mut().push_back(NetworkEvent::WsError {
                        generation,
                        id,
                        error: NetworkError::new(failure.code, failure.message),
                    });
                }
            }
        }
    }

    fn dispatch_ws_send_text(&self, id: u64, text: String) -> Result<(), NetworkError> {
        match &self.backend {
            NetworkBackend::Live { .. } => {
                let ws = self.ws.borrow();
                let Some(entry) = ws.get(&id) else {
                    return Err(NetworkError::new("E_CLOSED", "WebSocket is closed"));
                };
                let Some(handle) = &entry.handle else {
                    return Err(NetworkError::new("E_CLOSED", "WebSocket is not connected"));
                };
                handle.try_send(OutboundPayload::Text(text)).map(|_| ())
            }
            NetworkBackend::Preview { fixture } => fixture.send_ws_fixture_text(id, text),
        }
    }

    fn dispatch_ws_send_binary(&self, id: u64, bytes: Vec<u8>) -> Result<(), NetworkError> {
        match &self.backend {
            NetworkBackend::Live { .. } => {
                let ws = self.ws.borrow();
                let Some(entry) = ws.get(&id) else {
                    return Err(NetworkError::new("E_CLOSED", "WebSocket is closed"));
                };
                let Some(handle) = &entry.handle else {
                    return Err(NetworkError::new("E_CLOSED", "WebSocket is not connected"));
                };
                handle.try_send(OutboundPayload::Binary(bytes)).map(|_| ())
            }
            NetworkBackend::Preview { fixture } => fixture.send_ws_fixture_binary(id, bytes),
        }
    }

    fn dispatch_ws_close(&self, id: u64, code: u16, reason: String) {
        if let Some(entry) = self.ws.borrow_mut().remove(&id)
            && let Some(handle) = entry.handle
        {
            handle.close(code, reason);
        }
        if let NetworkBackend::Preview { fixture } = &self.backend {
            fixture.cancel_network_fixture(id);
        }
    }

    pub(super) fn counts(&self) -> NetworkCounts {
        NetworkCounts {
            http_requests: self.http.borrow().len(),
            event_streams: self.sse.borrow().len(),
            web_sockets: self.ws.borrow().len(),
        }
    }

    pub(super) fn clear(&self, generation: u64) {
        self.staging.set(false);
        self.staged_ops.borrow_mut().clear();
        self.local_events.borrow_mut().clear();
        for (_, entry) in std::mem::take(&mut *self.http.borrow_mut()) {
            if let Some(handle) = entry.handle {
                handle.cancel();
            }
        }
        for (_, entry) in std::mem::take(&mut *self.sse.borrow_mut()) {
            if let Some(handle) = entry.handle {
                handle.close();
            }
        }
        for (_, entry) in std::mem::take(&mut *self.ws.borrow_mut()) {
            if let Some(handle) = entry.handle {
                handle.close(1001, String::new());
            }
        }
        if let NetworkBackend::Preview { fixture } = &self.backend {
            fixture.cancel_network_generation(generation);
        }
    }

    pub(super) fn has_events(&self) -> bool {
        if !self.local_events.borrow().is_empty() {
            return true;
        }
        match &self.backend {
            NetworkBackend::Preview { fixture } => fixture.has_network_delivery(),
            NetworkBackend::Live { .. } => {
                // In live mode we check if any active handle is present only when woken;
                // handles wake the owner on every pushed event.
                false
            }
        }
    }

    pub(super) fn next_event(&self, generation: u64) -> Option<NetworkEvent> {
        if let Some(ev) = self.local_events.borrow_mut().pop_front() {
            return Some(ev);
        }
        match &self.backend {
            NetworkBackend::Preview { fixture } => fixture.take_network().map(from_delivery),
            NetworkBackend::Live { .. } => self.poll_live_round_robin(generation),
        }
    }

    /// Fair per-registration round-robin poll across active HTTP, SSE, and WebSocket handles.
    fn poll_live_round_robin(&self, generation: u64) -> Option<NetworkEvent> {
        let mut ids: Vec<u64> = self
            .http
            .borrow()
            .keys()
            .chain(self.sse.borrow().keys())
            .chain(self.ws.borrow().keys())
            .copied()
            .collect();
        if ids.is_empty() {
            return None;
        }
        ids.sort_unstable();
        let after = self.poll_cursor.get();
        let split = ids.partition_point(|&id| id <= after);
        for &id in ids[split..].iter().chain(ids[..split].iter()) {
            if let Some(ev) = self
                .http
                .borrow()
                .get(&id)
                .and_then(|e| e.handle.as_ref())
                .and_then(|h| h.try_recv())
            {
                self.poll_cursor.set(id);
                return Some(match ev {
                    HttpEvent::Head(head) => NetworkEvent::HttpHead { generation, id, head },
                    HttpEvent::Chunk { data, .. } => NetworkEvent::HttpChunk { generation, id, data },
                    HttpEvent::End => NetworkEvent::HttpEnd { generation, id },
                    HttpEvent::Error(error) => NetworkEvent::HttpError { generation, id, error },
                });
            }
            if let Some(ev) = self
                .sse
                .borrow()
                .get(&id)
                .and_then(|e| e.handle.as_ref())
                .and_then(|h| h.try_recv())
            {
                self.poll_cursor.set(id);
                return Some(match ev {
                    SseEvent::Open { status, url, headers } => {
                        NetworkEvent::SseOpen { generation, id, status, url, headers }
                    }
                    SseEvent::Message { message, .. } => {
                        NetworkEvent::SseMessage { generation, id, message }
                    }
                    SseEvent::Error(error) => NetworkEvent::SseError { generation, id, error },
                    SseEvent::Close => NetworkEvent::SseClose { generation, id },
                });
            }
            let ws_ev = {
                let map = self.ws.borrow();
                map.get(&id)
                    .and_then(|e| e.handle.as_ref().and_then(|h| h.try_recv()).map(|ev| (e.url.clone(), ev)))
            };
            if let Some((url, ev)) = ws_ev {
                self.poll_cursor.set(id);
                return Some(match ev {
                    WebSocketEvent::Open { protocol } => {
                        NetworkEvent::WsOpen { generation, id, url, protocol }
                    }
                    WebSocketEvent::Message { payload: InboundPayload::Text(text), .. } => {
                        NetworkEvent::WsText { generation, id, text }
                    }
                    WebSocketEvent::Message { payload: InboundPayload::Binary(data), .. } => {
                        NetworkEvent::WsBinary { generation, id, data }
                    }
                    WebSocketEvent::Error(error) => NetworkEvent::WsError { generation, id, error },
                    WebSocketEvent::Close { code, reason, .. } => {
                        NetworkEvent::WsClose { generation, id, code, reason }
                    }
                });
            }
        }
        None
    }
}

fn from_delivery(delivery: NetworkDelivery) -> NetworkEvent {
    match delivery {
        NetworkDelivery::HttpHead { generation, id, head } => {
            NetworkEvent::HttpHead { generation, id, head }
        }
        NetworkDelivery::HttpChunk { generation, id, data } => {
            NetworkEvent::HttpChunk { generation, id, data }
        }
        NetworkDelivery::HttpEnd { generation, id } => NetworkEvent::HttpEnd { generation, id },
        NetworkDelivery::HttpError { generation, id, error } => {
            NetworkEvent::HttpError { generation, id, error }
        }
        NetworkDelivery::SseOpen { generation, id, status, url, headers } => {
            NetworkEvent::SseOpen { generation, id, status, url, headers }
        }
        NetworkDelivery::SseMessage { generation, id, message } => {
            NetworkEvent::SseMessage { generation, id, message }
        }
        NetworkDelivery::SseError { generation, id, error } => {
            NetworkEvent::SseError { generation, id, error }
        }
        NetworkDelivery::SseClose { generation, id } => NetworkEvent::SseClose { generation, id },
        NetworkDelivery::WsOpen { generation, id, url, protocol } => {
            NetworkEvent::WsOpen { generation, id, url, protocol }
        }
        NetworkDelivery::WsText { generation, id, text } => {
            NetworkEvent::WsText { generation, id, text }
        }
        NetworkDelivery::WsBinary { generation, id, data } => {
            NetworkEvent::WsBinary { generation, id, data }
        }
        NetworkDelivery::WsError { generation, id, error } => {
            NetworkEvent::WsError { generation, id, error }
        }
        NetworkDelivery::WsClose { generation, id, code, reason } => {
            NetworkEvent::WsClose { generation, id, code, reason }
        }
    }
}

fn throw_network_error(ctx: &Ctx<'_>, err: &NetworkError) -> rquickjs::Error {
    match service_error(ctx, err.code, &err.message) {
        Ok(obj) => ctx.throw(obj.into_value()),
        Err(e) => e,
    }
}

fn parse_headers<'js>(ctx: &Ctx<'js>, value: Option<Value<'js>>) -> rquickjs::Result<Vec<(String, String)>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_undefined() || value.is_null() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    if let Some(arr) = value.as_array() {
        for item in arr.iter::<Value>() {
            let item = item?;
            let pair = item
                .as_array()
                .ok_or_else(|| Exception::throw_type(ctx, "headers array entries must be [name, value] tuples"))?;
            if pair.len() != 2 {
                return Err(Exception::throw_type(ctx, "headers tuple must have length 2"));
            }
            let k: String = pair.get(0)?;
            let v: String = pair.get(1)?;
            out.push((k, v));
        }
    } else if let Some(obj) = value.as_object() {
        for entry in obj.props::<String, Value>() {
            let (k, v) = entry?;
            if v.is_undefined() || v.is_null() {
                continue;
            }
            let s: String = rquickjs::FromJs::from_js(ctx, v)?;
            out.push((k, s));
        }
    } else {
        return Err(Exception::throw_type(ctx, "headers must be an object or array of [name, value] pairs"));
    }
    Ok(out)
}

fn parse_body(ctx: &Ctx<'_>, value: Option<Value<'_>>) -> rquickjs::Result<Vec<u8>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_undefined() || value.is_null() {
        return Ok(Vec::new());
    }
    let bytes = if let Some(s) = value.as_string() {
        s.to_string()?.into_bytes()
    } else if let Ok(ta) = TypedArray::<u8>::from_value(value) {
        let raw = ta
            .as_raw()
            .ok_or_else(|| Exception::throw_type(ctx, "request body Uint8Array is detached"))?;
        // SAFETY: `ta` keeps the JS backing buffer alive during this copy.
        unsafe { raw.as_ref().to_vec() }
    } else {
        return Err(Exception::throw_type(ctx, "request body must be a string or Uint8Array"));
    };
    if bytes.len() > MAX_REQUEST_BODY_BYTES {
        return Err(throw_network_error(
            ctx,
            &NetworkError::new(
                "E_LIMIT",
                format!("request body exceeds limit of {MAX_REQUEST_BODY_BYTES} bytes"),
            ),
        ));
    }
    Ok(bytes)
}

fn headers_to_js<'js>(ctx: &Ctx<'js>, headers: &[(String, String)]) -> rquickjs::Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    for (k, v) in headers {
        obj.prop(k.as_str(), data_property(v.as_str()))?;
    }
    Ok(obj)
}

impl Work {
    pub(crate) fn begin_staged_network(&self) {
        self.network.begin_staged_mount();
    }

    pub(crate) fn commit_staged_network(&self) {
        self.network.commit_staged_mount(self.generation, &self.wake);
    }

    pub(crate) fn network_counts(&self) -> NetworkCounts {
        self.network.counts()
    }

    pub(crate) fn has_network_events(&self) -> bool {
        self.network.has_events()
    }

    pub(crate) fn next_network_event(&self) -> Option<NetworkEvent> {
        if self.mode.get() != Mode::Mounted {
            return None;
        }
        self.network.next_event(self.generation)
    }

    pub(super) fn install_network<'js>(self: &Rc<Self>, services: &Object<'js>) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        services.prop(
            "httpRequest",
            data_property(Func::from(move |ctx: Ctx<'js>, options: Object<'js>| {
                owner(&weak, &ctx)?.start_http(&ctx, options)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "openEventStream",
            data_property(Func::from(
                move |ctx: Ctx<'js>, options: Object<'js>, callbacks: Object<'js>| {
                    owner(&weak, &ctx)?.start_sse(&ctx, options, callbacks)
                },
            )),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "connectWebSocket",
            data_property(Func::from(
                move |ctx: Ctx<'js>, options: Object<'js>, callbacks: Object<'js>| {
                    owner(&weak, &ctx)?.start_ws(&ctx, options, callbacks)
                },
            )),
        )?;
        Ok(())
    }

    fn start_http<'js>(self: &Rc<Self>, ctx: &Ctx<'js>, options: Object<'js>) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        if self.network.http.borrow().len() >= MAX_CONCURRENT_HTTP {
            return Err(throw_network_error(
                ctx,
                &NetworkError::new("E_LIMIT", "concurrent HTTP request limit reached"),
            ));
        }
        let url: String = options.get("url")?;
        let method = options
            .get::<_, Option<String>>("method")?
            .unwrap_or_else(|| "GET".into());
        let headers = parse_headers(ctx, options.get::<_, Option<Value>>("headers")?)?;
        let body = parse_body(ctx, options.get::<_, Option<Value>>("body")?)?;
        let timeout_ms = options
            .get::<_, Option<f64>>("timeoutMs")?
            .map(|n| n.clamp(100.0, 300_000.0) as u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS);
        let max_response_bytes = options
            .get::<_, Option<f64>>("maxResponseBytes")?
            .map(|n| n.max(1.0) as usize)
            .unwrap_or(DEFAULT_MAX_RESPONSE_BYTES);
        let max_redirects = options
            .get::<_, Option<f64>>("maxRedirects")?
            .map(|n| n.clamp(0.0, 10.0) as u8)
            .unwrap_or(DEFAULT_MAX_REDIRECTS);

        // Pre-DNS / pre-fixture origin policy authorization (user gesture never grants network authority).
        self.network
            .policy
            .authorize_http(&url)
            .map_err(|err| throw_network_error(ctx, &err))?;

        let id = self.id(ctx)?;
        let (head_promise, resolve, reject) = ctx.promise()?;
        let spec = HttpRequestSpec {
            method,
            url,
            headers,
            body,
            timeout_ms,
            max_response_bytes,
            max_redirects,
        };
        self.network.http.borrow_mut().insert(
            id,
            HttpEntry {
                head: Some(PendingPromise {
                    resolve: Persistent::save(ctx, resolve),
                    reject: Persistent::save(ctx, reject),
                }),
                chunk: None,
                handle: None,
                staged_pulls: 0,
            },
        );
        if self.network.staging.get() {
            self.network
                .staged_ops
                .borrow_mut()
                .push_back(StagedOp::StartHttp { id, spec });
        } else {
            self.network.dispatch_http(self.generation, id, spec, &self.wake);
        }

        let handle_obj = Object::new(ctx.clone())?;
        handle_obj.prop("headPromise", data_property(head_promise))?;
        let weak = Rc::downgrade(self);
        handle_obj.prop(
            "readChunk",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                owner(&weak, &ctx)?.pull_http_chunk(&ctx, id)
            })),
        )?;
        let weak = Rc::downgrade(self);
        handle_obj.prop(
            "cancel",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                owner(&weak, &ctx)?.cancel_http(&ctx, id)
            })),
        )?;
        Ok(handle_obj)
    }

    fn pull_http_chunk<'js>(&self, ctx: &Ctx<'js>, id: u64) -> rquickjs::Result<Value<'js>> {
        let mut map = self.network.http.borrow_mut();
        let Some(entry) = map.get_mut(&id) else {
            let (promise, resolve, _) = ctx.promise()?;
            resolve.call::<_, ()>((Value::new_null(ctx.clone()),))?;
            return Ok(promise.into_value());
        };
        if entry.chunk.is_some() {
            return Err(throw_network_error(
                ctx,
                &NetworkError::new("E_PROTOCOL", "concurrent readChunk calls on the same HTTP response are not allowed"),
            ));
        }
        let (promise, resolve, reject) = ctx.promise()?;
        entry.chunk = Some(PendingPromise {
            resolve: Persistent::save(ctx, resolve),
            reject: Persistent::save(ctx, reject),
        });
        if self.network.staging.get() {
            entry.staged_pulls += 1;
            drop(map);
            self.network
                .staged_ops
                .borrow_mut()
                .push_back(StagedOp::PullHttp { id });
        } else {
            drop(map);
            self.network.dispatch_http_pull(id);
        }
        Ok(promise.into_value())
    }

    fn cancel_http<'js>(&self, ctx: &Ctx<'js>, id: u64) -> rquickjs::Result<()> {
        let removed = self.network.http.borrow_mut().remove(&id);
        let Some(entry) = removed else {
            return Ok(());
        };
        if let Some(handle) = entry.handle {
            handle.cancel();
        }
        if let NetworkBackend::Preview { fixture } = &self.network.backend {
            fixture.cancel_network_fixture(id);
        }
        let err = service_error(ctx, "E_CANCELLED", "HTTP request cancelled")?;
        if let Some(head) = entry.head {
            let _ = head.reject.restore(ctx)?.call::<_, ()>((err.clone(),));
        }
        if let Some(chunk) = entry.chunk {
            let _ = chunk.reject.restore(ctx)?.call::<_, ()>((err,));
        }
        Ok(())
    }

    fn start_sse<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        options: Object<'js>,
        callbacks: Object<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        if self.network.sse.borrow().len() >= MAX_CONCURRENT_SSE {
            return Err(throw_network_error(
                ctx,
                &NetworkError::new("E_LIMIT", "concurrent event stream limit reached"),
            ));
        }
        let url: String = options.get("url")?;
        let headers = parse_headers(ctx, options.get::<_, Option<Value>>("headers")?)?;
        let last_event_id = options.get::<_, Option<String>>("lastEventId")?;
        let timeout_ms = options
            .get::<_, Option<f64>>("timeoutMs")?
            .map(|n| n.clamp(100.0, 300_000.0) as u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS);
        let max_event_bytes = options
            .get::<_, Option<f64>>("maxEventBytes")?
            .map(|n| n.max(1.0) as usize)
            .unwrap_or(DEFAULT_MAX_SSE_EVENT_BYTES);
        let max_redirects = options
            .get::<_, Option<f64>>("maxRedirects")?
            .map(|n| n.clamp(0.0, 10.0) as u8)
            .unwrap_or(DEFAULT_MAX_REDIRECTS);

        let on_message: Function = callbacks.get("onMessage")?;
        let on_open: Option<Function> = callbacks.get("onOpen")?;
        let on_error: Option<Function> = callbacks.get("onError")?;
        let on_close: Option<Function> = callbacks.get("onClose")?;

        self.network
            .policy
            .authorize_http(&url)
            .map_err(|err| throw_network_error(ctx, &err))?;

        let id = self.id(ctx)?;
        let spec = EventStreamSpec {
            url,
            headers,
            last_event_id,
            timeout_ms,
            max_event_bytes,
            max_redirects,
        };
        self.network.sse.borrow_mut().insert(
            id,
            SseEntry {
                on_open: on_open.map(|f| Persistent::save(ctx, f)),
                on_message: Persistent::save(ctx, on_message),
                on_error: on_error.map(|f| Persistent::save(ctx, f)),
                on_close: on_close.map(|f| Persistent::save(ctx, f)),
                handle: None,
            },
        );
        if self.network.staging.get() {
            self.network
                .staged_ops
                .borrow_mut()
                .push_back(StagedOp::StartSse { id, spec });
        } else {
            self.network.dispatch_sse(self.generation, id, spec, &self.wake);
        }

        let handle_obj = Object::new(ctx.clone())?;
        let weak = Rc::downgrade(self);
        handle_obj.prop(
            "close",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                if let Ok(owner) = owner(&weak, &ctx) {
                    owner.close_sse(id);
                }
                Ok::<(), rquickjs::Error>(())
            })),
        )?;
        Ok(handle_obj)
    }

    fn close_sse(&self, id: u64) {
        if let Some(entry) = self.network.sse.borrow_mut().remove(&id)
            && let Some(handle) = entry.handle
        {
            handle.close();
        }
        if let NetworkBackend::Preview { fixture } = &self.network.backend {
            fixture.cancel_network_fixture(id);
        }
    }

    fn start_ws<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        options: Object<'js>,
        callbacks: Object<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        if self.network.ws.borrow().len() >= MAX_CONCURRENT_WS {
            return Err(throw_network_error(
                ctx,
                &NetworkError::new("E_LIMIT", "concurrent WebSocket limit reached"),
            ));
        }
        let url: String = options.get("url")?;
        let headers = parse_headers(ctx, options.get::<_, Option<Value>>("headers")?)?;
        let mut protocols = Vec::new();
        if let Some(arr) = options.get::<_, Option<Array>>("protocols")? {
            for item in arr.iter::<String>() {
                protocols.push(item?);
            }
        }
        let timeout_ms = options
            .get::<_, Option<f64>>("timeoutMs")?
            .map(|n| n.clamp(100.0, 300_000.0) as u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS);
        let max_message_bytes = options
            .get::<_, Option<f64>>("maxMessageBytes")?
            .map(|n| n.max(1.0) as usize)
            .unwrap_or(DEFAULT_MAX_WS_MESSAGE_BYTES);
        let max_outbound_bytes = options
            .get::<_, Option<f64>>("maxOutboundBytes")?
            .map(|n| n.max(1.0) as usize)
            .unwrap_or(DEFAULT_MAX_WS_OUTBOUND_BYTES);

        let on_message: Function = callbacks.get("onMessage")?;
        let on_open: Option<Function> = callbacks.get("onOpen")?;
        let on_error: Option<Function> = callbacks.get("onError")?;
        let on_close: Option<Function> = callbacks.get("onClose")?;

        self.network
            .policy
            .authorize_ws(&url)
            .map_err(|err| throw_network_error(ctx, &err))?;

        let id = self.id(ctx)?;
        let spec = WebSocketSpec {
            url: url.clone(),
            headers,
            protocols,
            timeout_ms,
            max_message_bytes,
            max_outbound_bytes,
        };
        self.network.ws.borrow_mut().insert(
            id,
            WsEntry {
                url,
                on_open: on_open.map(|f| Persistent::save(ctx, f)),
                on_message: Persistent::save(ctx, on_message),
                on_error: on_error.map(|f| Persistent::save(ctx, f)),
                on_close: on_close.map(|f| Persistent::save(ctx, f)),
                handle: None,
                staged_bytes: 0,
                max_outbound_bytes,
            },
        );
        if self.network.staging.get() {
            self.network
                .staged_ops
                .borrow_mut()
                .push_back(StagedOp::StartWs { id, spec });
        } else {
            self.network.dispatch_ws(self.generation, id, spec, &self.wake);
        }

        let handle_obj = Object::new(ctx.clone())?;
        let weak = Rc::downgrade(self);
        handle_obj.prop(
            "sendText",
            data_property(Func::from(move |ctx: Ctx<'js>, text: String| {
                owner(&weak, &ctx)?.ws_send_text(&ctx, id, text)
            })),
        )?;
        let weak = Rc::downgrade(self);
        handle_obj.prop(
            "sendBinary",
            data_property(Func::from(move |ctx: Ctx<'js>, data: Value<'js>| {
                owner(&weak, &ctx)?.ws_send_binary(&ctx, id, data)
            })),
        )?;
        let weak = Rc::downgrade(self);
        handle_obj.prop(
            "close",
            data_property(Func::from(
                move |ctx: Ctx<'js>, code: Opt<f64>, reason: Opt<String>| {
                    if let Ok(owner) = owner(&weak, &ctx) {
                        let code = code.0.map(|n| n.clamp(1000.0, 4999.0) as u16).unwrap_or(1000);
                        let reason = reason.0.unwrap_or_default();
                        owner.ws_close(id, code, reason);
                    }
                    Ok::<(), rquickjs::Error>(())
                },
            )),
        )?;
        Ok(handle_obj)
    }

    fn ws_send_text(&self, ctx: &Ctx<'_>, id: u64, text: String) -> rquickjs::Result<()> {
        if self.network.staging.get() {
            let mut map = self.network.ws.borrow_mut();
            let Some(entry) = map.get_mut(&id) else {
                return Err(throw_network_error(ctx, &NetworkError::new("E_CLOSED", "WebSocket is closed")));
            };
            if entry.staged_bytes.saturating_add(text.len()) > entry.max_outbound_bytes {
                return Err(throw_network_error(
                    ctx,
                    &NetworkError::new("E_BACKPRESSURE", "WebSocket outbound send queue is full"),
                ));
            }
            entry.staged_bytes += text.len();
            drop(map);
            self.network
                .staged_ops
                .borrow_mut()
                .push_back(StagedOp::WsSendText { id, text });
            return Ok(());
        }
        self.network
            .dispatch_ws_send_text(id, text)
            .map_err(|err| throw_network_error(ctx, &err))
    }

    fn ws_send_binary(&self, ctx: &Ctx<'_>, id: u64, value: Value<'_>) -> rquickjs::Result<()> {
        let ta = TypedArray::<u8>::from_value(value)
            .map_err(|_| Exception::throw_type(ctx, "WebSocket binary payload must be a Uint8Array"))?;
        let raw = ta
            .as_raw()
            .ok_or_else(|| Exception::throw_type(ctx, "WebSocket binary Uint8Array is detached"))?;
        // SAFETY: `ta` keeps the JS backing buffer alive during this copy.
        let bytes = unsafe { raw.as_ref().to_vec() };
        if self.network.staging.get() {
            let mut map = self.network.ws.borrow_mut();
            let Some(entry) = map.get_mut(&id) else {
                return Err(throw_network_error(ctx, &NetworkError::new("E_CLOSED", "WebSocket is closed")));
            };
            if entry.staged_bytes.saturating_add(bytes.len()) > entry.max_outbound_bytes {
                return Err(throw_network_error(
                    ctx,
                    &NetworkError::new("E_BACKPRESSURE", "WebSocket outbound send queue is full"),
                ));
            }
            entry.staged_bytes += bytes.len();
            drop(map);
            self.network
                .staged_ops
                .borrow_mut()
                .push_back(StagedOp::WsSendBinary { id, bytes });
            return Ok(());
        }
        self.network
            .dispatch_ws_send_binary(id, bytes)
            .map_err(|err| throw_network_error(ctx, &err))
    }

    fn ws_close(&self, id: u64, code: u16, reason: String) {
        if self.network.staging.get() {
            self.network.ws.borrow_mut().remove(&id);
            self.network
                .staged_ops
                .borrow_mut()
                .push_back(StagedOp::WsClose { id, code, reason });
        } else {
            self.network.dispatch_ws_close(id, code, reason);
        }
    }

    /// Settles one network event, checking session generation and live registration
    /// immediately before invoking JS so retired or cancelled streams receive zero callbacks.
    pub(crate) fn settle_network_event<'js>(
        &self,
        ctx: &Ctx<'js>,
        event: NetworkEvent,
    ) -> rquickjs::Result<()> {
        if self.mode.get() != Mode::Mounted {
            return Ok(());
        }
        match event {
            NetworkEvent::HttpHead { generation, id, head } => {
                if generation != self.generation {
                    return Ok(());
                }
                let pending = self
                    .network
                    .http
                    .borrow_mut()
                    .get_mut(&id)
                    .and_then(|entry| entry.head.take());
                let Some(pending) = pending else {
                    return Ok(());
                };
                let obj = Object::new(ctx.clone())?;
                obj.prop("status", data_property(head.status as i32))?;
                obj.prop("statusText", data_property(head.status_text.as_str()))?;
                obj.prop("url", data_property(head.url.as_str()))?;
                obj.prop("redirected", data_property(head.redirected))?;
                obj.prop("headers", data_property(headers_to_js(ctx, &head.headers)?))?;
                pending.resolve.restore(ctx)?.call::<_, ()>((obj,))
            }
            NetworkEvent::HttpChunk { generation, id, data } => {
                if generation != self.generation {
                    return Ok(());
                }
                let pending = self
                    .network
                    .http
                    .borrow_mut()
                    .get_mut(&id)
                    .and_then(|entry| entry.chunk.take());
                let Some(pending) = pending else {
                    return Ok(());
                };
                let array = TypedArray::<u8>::new(ctx.clone(), data)?;
                pending.resolve.restore(ctx)?.call::<_, ()>((array,))
            }
            NetworkEvent::HttpEnd { generation, id } => {
                if generation != self.generation {
                    return Ok(());
                }
                let removed = self.network.http.borrow_mut().remove(&id);
                let Some(entry) = removed else {
                    return Ok(());
                };
                if let Some(chunk) = entry.chunk {
                    chunk.resolve.restore(ctx)?.call::<_, ()>((Value::new_null(ctx.clone()),))?;
                }
                Ok(())
            }
            NetworkEvent::HttpError { generation, id, error } => {
                if generation != self.generation {
                    return Ok(());
                }
                let removed = self.network.http.borrow_mut().remove(&id);
                let Some(entry) = removed else {
                    return Ok(());
                };
                let js_err = service_error(ctx, error.code, &error.message)?;
                if let Some(head) = entry.head {
                    head.reject.restore(ctx)?.call::<_, ()>((js_err.clone(),))?;
                }
                if let Some(chunk) = entry.chunk {
                    chunk.reject.restore(ctx)?.call::<_, ()>((js_err,))?;
                }
                Ok(())
            }
            NetworkEvent::SseOpen { generation, id, status, url, headers } => {
                if generation != self.generation {
                    return Ok(());
                }
                let cb = self
                    .network
                    .sse
                    .borrow()
                    .get(&id)
                    .and_then(|e| e.on_open.as_ref().map(|f| f.clone().restore(ctx)))
                    .transpose()?;
                let Some(cb) = cb else {
                    return Ok(());
                };
                let info = Object::new(ctx.clone())?;
                info.prop("status", data_property(status as i32))?;
                info.prop("url", data_property(url.as_str()))?;
                info.prop("headers", data_property(headers_to_js(ctx, &headers)?))?;
                cb.call::<_, ()>((info,))
            }
            NetworkEvent::SseMessage { generation, id, message } => {
                if generation != self.generation {
                    return Ok(());
                }
                let cb = self
                    .network
                    .sse
                    .borrow()
                    .get(&id)
                    .map(|e| e.on_message.clone().restore(ctx))
                    .transpose()?;
                let Some(cb) = cb else {
                    return Ok(());
                };
                let obj = Object::new(ctx.clone())?;
                obj.prop("event", data_property(message.event.as_str()))?;
                obj.prop("data", data_property(message.data.as_str()))?;
                if let Some(ev_id) = message.id {
                    obj.prop("id", data_property(ev_id.as_str()))?;
                }
                if let Some(retry_ms) = message.retry_ms {
                    obj.prop("retryMs", data_property(retry_ms as f64))?;
                }
                cb.call::<_, ()>((obj,))
            }
            NetworkEvent::SseError { generation, id, error } => {
                if generation != self.generation {
                    return Ok(());
                }
                let removed = self.network.sse.borrow_mut().remove(&id);
                let Some(entry) = removed else {
                    return Ok(());
                };
                if let Some(on_error) = entry.on_error {
                    let js_err = service_error(ctx, error.code, &error.message)?;
                    on_error.restore(ctx)?.call::<_, ()>((js_err,))?;
                }
                Ok(())
            }
            NetworkEvent::SseClose { generation, id } => {
                if generation != self.generation {
                    return Ok(());
                }
                let removed = self.network.sse.borrow_mut().remove(&id);
                let Some(entry) = removed else {
                    return Ok(());
                };
                if let Some(on_close) = entry.on_close {
                    on_close.restore(ctx)?.call::<_, ()>(())?;
                }
                Ok(())
            }
            NetworkEvent::WsOpen { generation, id, url, protocol } => {
                if generation != self.generation {
                    return Ok(());
                }
                let cb = self
                    .network
                    .ws
                    .borrow()
                    .get(&id)
                    .and_then(|e| e.on_open.as_ref().map(|f| f.clone().restore(ctx)))
                    .transpose()?;
                let Some(cb) = cb else {
                    return Ok(());
                };
                let info = Object::new(ctx.clone())?;
                info.prop("url", data_property(url.as_str()))?;
                let proto = match protocol {
                    Some(p) => p.into_js(ctx)?,
                    None => Value::new_null(ctx.clone()),
                };
                info.prop("protocol", data_property(proto))?;
                cb.call::<_, ()>((info,))
            }
            NetworkEvent::WsText { generation, id, text } => {
                if generation != self.generation {
                    return Ok(());
                }
                let cb = self
                    .network
                    .ws
                    .borrow()
                    .get(&id)
                    .map(|e| e.on_message.clone().restore(ctx))
                    .transpose()?;
                let Some(cb) = cb else {
                    return Ok(());
                };
                let msg = Object::new(ctx.clone())?;
                msg.prop("kind", data_property("text"))?;
                msg.prop("data", data_property(text.as_str()))?;
                cb.call::<_, ()>((msg,))
            }
            NetworkEvent::WsBinary { generation, id, data } => {
                if generation != self.generation {
                    return Ok(());
                }
                let cb = self
                    .network
                    .ws
                    .borrow()
                    .get(&id)
                    .map(|e| e.on_message.clone().restore(ctx))
                    .transpose()?;
                let Some(cb) = cb else {
                    return Ok(());
                };
                let msg = Object::new(ctx.clone())?;
                msg.prop("kind", data_property("binary"))?;
                let bytes = TypedArray::<u8>::new(ctx.clone(), data)?;
                msg.prop("data", data_property(bytes))?;
                cb.call::<_, ()>((msg,))
            }
            NetworkEvent::WsError { generation, id, error } => {
                if generation != self.generation {
                    return Ok(());
                }
                let removed = self.network.ws.borrow_mut().remove(&id);
                let Some(entry) = removed else {
                    return Ok(());
                };
                if let Some(on_error) = entry.on_error {
                    let js_err = service_error(ctx, error.code, &error.message)?;
                    on_error.restore(ctx)?.call::<_, ()>((js_err,))?;
                }
                Ok(())
            }
            NetworkEvent::WsClose { generation, id, code, reason } => {
                if generation != self.generation {
                    return Ok(());
                }
                let removed = self.network.ws.borrow_mut().remove(&id);
                let Some(entry) = removed else {
                    return Ok(());
                };
                if let Some(on_close) = entry.on_close {
                    let info = Object::new(ctx.clone())?;
                    info.prop("code", data_property(code as i32))?;
                    info.prop("reason", data_property(reason.as_str()))?;
                    on_close.restore(ctx)?.call::<_, ()>((info,))?;
                }
                Ok(())
            }
        }
    }
}
