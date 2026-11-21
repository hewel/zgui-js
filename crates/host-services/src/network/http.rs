//! Pull-driven bounded HTTP/1.1 transport and SSE streaming over origin-verified connections.
use std::{
    collections::BTreeMap,
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::Duration,
};
use bytes::Bytes;
use flate2::write::{GzDecoder, ZlibDecoder};
use http::{HeaderMap, HeaderName, HeaderValue, Method, Request};
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::{sync::Notify, time::timeout};
use url::Url;
use super::{
    NetworkError,
    budget::{ByteLease, SessionByteBudget},
    policy::{Origin, OriginPolicy, connect_verified_stream},
    sse::{DEFAULT_MAX_SSE_EVENT_BYTES, SseMessage, SseParser},
};

/// Default per-response body cap (16 MiB decompressed).
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
/// Hard ceiling for `max_response_bytes` (64 MiB).
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
/// Maximum HTTP request body size (16 MiB).
pub const MAX_REQUEST_BODY_BYTES: usize = 16 * 1024 * 1024;
/// Default request timeout (30 seconds).
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// Maximum HTTP redirect hops.
pub const MAX_REDIRECTS: u8 = 5;
/// Maximum single chunk emitted to JS on each pull (256 KiB).
const MAX_EMIT_CHUNK_BYTES: usize = 256 * 1024;
/// Bounded SSE event channel depth before applying backpressure.
const SSE_EVENT_QUEUE_DEPTH: usize = 64;

/// Owned specification for one HTTP request.
#[derive(Clone, Debug)]
pub struct HttpRequestSpec {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout_ms: u64,
    pub max_response_bytes: usize,
    pub max_redirects: u8,
}

/// Response status, final URL, and normalized response headers.
#[derive(Clone, Debug)]
pub struct HttpResponseHead {
    pub status: u16,
    pub status_text: String,
    pub url: String,
    pub redirected: bool,
    pub headers: Vec<(String, String)>,
}

/// Events delivered from an active pull-driven HTTP request.
#[derive(Debug)]
pub enum HttpEvent {
    Head(HttpResponseHead),
    Chunk { data: Vec<u8>, _lease: ByteLease },
    End,
    Error(NetworkError),
}

/// Owner-thread handle to an active HTTP request. Dropping signals immediate cancellation.
pub struct HttpHandle {
    inbox: Receiver<HttpEvent>,
    pull_notify: Arc<Notify>,
    cancelled: Arc<AtomicBool>,
}

impl HttpHandle {
    /// Requests the next bounded body chunk from the background transport task.
    pub fn request_next_chunk(&self) {
        if !self.cancelled.load(Ordering::Acquire) {
            self.pull_notify.notify_one();
        }
    }

    /// Cancels the in-flight request or open body stream without blocking.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.pull_notify.notify_one();
    }

    /// Non-blocking poll for the next HTTP event.
    pub fn try_recv(&self) -> Option<HttpEvent> {
        self.inbox.try_recv().ok()
    }
}

impl Drop for HttpHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Owned specification for one Server-Sent Events stream.
#[derive(Clone, Debug)]
pub struct EventStreamSpec {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub last_event_id: Option<String>,
    pub timeout_ms: u64,
    pub max_event_bytes: usize,
    pub max_redirects: u8,
}

/// Events delivered from an active Server-Sent Events stream.
#[derive(Debug)]
pub enum SseEvent {
    Open {
        status: u16,
        url: String,
        headers: Vec<(String, String)>,
    },
    Message {
        message: SseMessage,
        _lease: ByteLease,
    },
    Error(NetworkError),
    Close,
}

/// Owner-thread handle to an active SSE stream. Dropping cancels transport immediately.
pub struct EventStreamHandle {
    inbox: Receiver<SseEvent>,
    cancelled: Arc<AtomicBool>,
}

impl EventStreamHandle {
    /// Closes the stream without blocking.
    pub fn close(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Non-blocking poll for the next SSE event.
    pub fn try_recv(&self) -> Option<SseEvent> {
        self.inbox.try_recv().ok()
    }
}

impl Drop for EventStreamHandle {
    fn drop(&mut self) {
        self.close();
    }
}

/// Bounded writer that rejects decompressed output exceeding `limit` bytes.
struct BoundedBuffer {
    out: Vec<u8>,
    written_total: usize,
    limit: usize,
}

impl BoundedBuffer {
    fn new(limit: usize) -> Self {
        Self { out: Vec::new(), written_total: 0, limit }
    }

    fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }
}

impl Write for BoundedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let next = self.written_total.saturating_add(buf.len());
        if next > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                format!("decompressed response body exceeded limit of {} bytes", self.limit),
            ));
        }
        self.written_total = next;
        self.out.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

enum BodyDecoder {
    Identity { total: usize, limit: usize },
    Gzip(GzDecoder<BoundedBuffer>),
    Deflate(ZlibDecoder<BoundedBuffer>),
}

impl BodyDecoder {
    fn for_encoding(encoding: Option<&str>, limit: usize) -> Result<Self, NetworkError> {
        match encoding.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            None | Some("" | "identity") => Ok(Self::Identity { total: 0, limit }),
            Some("gzip" | "x-gzip") => Ok(Self::Gzip(GzDecoder::new(BoundedBuffer::new(limit)))),
            Some("deflate") => Ok(Self::Deflate(ZlibDecoder::new(BoundedBuffer::new(limit)))),
            Some(other) => Err(NetworkError::new(
                "E_PROTOCOL",
                format!("unsupported Content-Encoding: {other}"),
            )),
        }
    }

    fn push(&mut self, input: &[u8]) -> Result<Vec<u8>, NetworkError> {
        match self {
            Self::Identity { total, limit } => {
                let next = total.saturating_add(input.len());
                if next > *limit {
                    return Err(NetworkError::new(
                        "E_LIMIT",
                        format!("response body exceeded limit of {limit} bytes"),
                    ));
                }
                *total = next;
                Ok(input.to_vec())
            }
            Self::Gzip(dec) => {
                dec.write_all(input).map_err(map_decode_error)?;
                let _ = dec.flush();
                Ok(dec.get_mut().take())
            }
            Self::Deflate(dec) => {
                dec.write_all(input).map_err(map_decode_error)?;
                let _ = dec.flush();
                Ok(dec.get_mut().take())
            }
        }
    }

    fn finish(self) -> Result<Vec<u8>, NetworkError> {
        match self {
            Self::Identity { .. } => Ok(Vec::new()),
            Self::Gzip(dec) => dec.finish().map(|mut b| b.take()).map_err(map_decode_error),
            Self::Deflate(dec) => dec.finish().map(|mut b| b.take()).map_err(map_decode_error),
        }
    }
}

fn map_decode_error(err: io::Error) -> NetworkError {
    if err.kind() == io::ErrorKind::FileTooLarge {
        NetworkError::new("E_LIMIT", err.to_string())
    } else {
        NetworkError::new("E_PROTOCOL", format!("body decompression failed: {err}"))
    }
}

fn is_forbidden_request_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host" | "content-length" | "transfer-encoding" | "connection" | "proxy-authorization" | "cookie"
    )
}

fn validate_method(raw: &str) -> Result<Method, NetworkError> {
    Method::from_bytes(raw.trim().as_bytes())
        .map_err(|_| NetworkError::new("E_NETWORK_POLICY", format!("invalid HTTP method: {raw:?}")))
}

fn build_header_map(headers: &[(String, String)], strip_auth: bool) -> Result<HeaderMap, NetworkError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        if is_forbidden_request_header(name) {
            continue;
        }
        if strip_auth && name.eq_ignore_ascii_case("authorization") {
            continue;
        }
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            NetworkError::new("E_NETWORK_POLICY", format!("invalid HTTP header name: {name:?}"))
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|_| {
            NetworkError::new("E_NETWORK_POLICY", format!("invalid HTTP header value for {name:?}"))
        })?;
        map.append(header_name, header_value);
    }
    Ok(map)
}

fn collect_response_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, value) in headers {
        if name.as_str().eq_ignore_ascii_case("set-cookie") {
            // Never maintain or expose ambient cookies.
            continue;
        }
        if let Ok(v) = value.to_str() {
            grouped.entry(name.as_str().to_ascii_lowercase()).or_default().push(v.to_owned());
        }
    }
    grouped
        .into_iter()
        .map(|(k, vals)| (k, vals.join(", ")))
        .collect()
}

fn request_target_path(url: &Url) -> String {
    let mut path = url.path().to_owned();
    if path.is_empty() {
        path.push('/');
    }
    if let Some(q) = url.query() {
        path.push('?');
        path.push_str(q);
    }
    path
}

fn host_header_value(origin: &Origin) -> String {
    if origin.port() == origin.scheme().default_port() {
        origin.host().to_owned()
    } else {
        format!("{}:{}", origin.host(), origin.port())
    }
}

struct ConnectedResponse {
    final_url: Url,
    redirected: bool,
    response: hyper::Response<hyper::body::Incoming>,
    _conn_task: tokio::task::JoinHandle<()>,
}

struct RedirectRequest<'a> {
    method: Method,
    url: Url,
    origin: Origin,
    headers: &'a [(String, String)],
    body: Vec<u8>,
    policy: &'a OriginPolicy,
    timeout: Duration,
    max_redirects: u8,
    cancelled: &'a AtomicBool,
}

async fn send_with_redirects(req_spec: RedirectRequest<'_>) -> Result<ConnectedResponse, NetworkError> {
    let RedirectRequest {
        mut method,
        mut url,
        origin: initial_origin,
        headers,
        mut body,
        policy,
        timeout: req_timeout,
        max_redirects,
        cancelled,
    } = req_spec;
    let mut origin = initial_origin.clone();
    let mut redirected = false;
    let mut hops = 0u8;

    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(NetworkError::cancelled());
        }
        let cross_origin = origin != initial_origin;
        let stream = connect_verified_stream(&origin, policy, req_timeout).await?;
        let io = TokioIo::new(stream);
        let (mut sender, conn) = timeout(req_timeout, http1::handshake(io))
            .await
            .map_err(|_| NetworkError::new("E_TIMEOUT", "HTTP handshake timed out"))?
            .map_err(|e| NetworkError::new("E_NETWORK", format!("HTTP handshake failed: {e}")))?;

        let conn_task = tokio::spawn(async move {
            let _ = conn.await;
        });

        let mut builder = Request::builder()
            .method(method.clone())
            .uri(request_target_path(&url))
            .header("host", host_header_value(&origin))
            .header("accept-encoding", "gzip, deflate");

        let header_map = build_header_map(headers, cross_origin)?;
        if let Some(target_headers) = builder.headers_mut() {
            for (k, v) in &header_map {
                target_headers.append(k, v.clone());
            }
        }

        let req = builder
            .body(Full::new(Bytes::from(body.clone())))
            .map_err(|e| NetworkError::new("E_NETWORK_POLICY", format!("invalid HTTP request: {e}")))?;

        let resp = timeout(req_timeout, sender.send_request(req))
            .await
            .map_err(|_| NetworkError::new("E_TIMEOUT", "HTTP request timed out"))?
            .map_err(|e| NetworkError::new("E_NETWORK", format!("HTTP request failed: {e}")))?;

        let status = resp.status().as_u16();
        let is_redirect = matches!(status, 301 | 302 | 303 | 307 | 308);
        if is_redirect
            && let Some(loc_val) = resp.headers().get("location").and_then(|v| v.to_str().ok())
        {
            if hops >= max_redirects {
                conn_task.abort();
                return Err(NetworkError::new(
                    "E_LIMIT",
                    format!("redirect limit ({max_redirects}) exceeded"),
                ));
            }
            let next_url = url.join(loc_val).map_err(|e| {
                NetworkError::new("E_PROTOCOL", format!("invalid redirect Location {loc_val:?}: {e}"))
            })?;
            // Re-authorize every redirect target against the sealed origin policy!
            let (authorized_url, next_origin) = policy.authorize_http(next_url.as_str())?;
            conn_task.abort();
            if status == 303 || ((status == 301 || status == 302) && method == Method::POST) {
                method = Method::GET;
                body.clear();
            }
            url = authorized_url;
            origin = next_origin;
            redirected = true;
            hops += 1;
            continue;
        }

        return Ok(ConnectedResponse {
            final_url: url,
            redirected,
            response: resp,
            _conn_task: conn_task,
        });
    }
}

/// Spawns a pull-driven HTTP request on the dedicated Tokio runtime.
pub fn spawn_http(
    runtime: &tokio::runtime::Handle,
    spec: HttpRequestSpec,
    policy: OriginPolicy,
    budget: SessionByteBudget,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<HttpHandle, NetworkError> {
    if spec.body.len() > MAX_REQUEST_BODY_BYTES {
        return Err(NetworkError::new(
            "E_LIMIT",
            format!("request body exceeds {} bytes", MAX_REQUEST_BODY_BYTES),
        ));
    }
    let method = validate_method(&spec.method)?;
    let (url, origin) = policy.authorize_http(&spec.url)?;
    let _ = build_header_map(&spec.headers, false)?;

    let (tx, rx) = mpsc::sync_channel::<HttpEvent>(4);
    let pull_notify = Arc::new(Notify::new());
    let cancelled = Arc::new(AtomicBool::new(false));
    let pull_rx = pull_notify.clone();
    let cancel_flag = cancelled.clone();

    let max_bytes = spec.max_response_bytes.clamp(1, MAX_RESPONSE_BYTES);
    let req_timeout = Duration::from_millis(spec.timeout_ms.clamp(100, 300_000));
    let max_redirects = spec.max_redirects.min(MAX_REDIRECTS);

    runtime.spawn(async move {
        let send = |tx: &SyncSender<HttpEvent>, wake: &Arc<dyn Fn() + Send + Sync>, ev: HttpEvent| -> bool {
            let ok = tx.send(ev).is_ok();
            if ok {
                wake();
            }
            ok
        };

        let connected = match send_with_redirects(RedirectRequest {
            method,
            url,
            origin,
            headers: &spec.headers,
            body: spec.body,
            policy: &policy,
            timeout: req_timeout,
            max_redirects,
            cancelled: &cancel_flag,
        })
        .await
        {
            Ok(c) => c,
            Err(err) => {
                if !cancel_flag.load(Ordering::Acquire) {
                    let _ = send(&tx, &wake, HttpEvent::Error(err));
                }
                return;
            }
        };

        let encoding = connected
            .response
            .headers()
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let mut decoder = match BodyDecoder::for_encoding(encoding.as_deref(), max_bytes) {
            Ok(d) => Some(d),
            Err(err) => {
                let _ = send(&tx, &wake, HttpEvent::Error(err));
                return;
            }
        };

        let head = HttpResponseHead {
            status: connected.response.status().as_u16(),
            status_text: connected
                .response
                .status()
                .canonical_reason()
                .unwrap_or("")
                .to_owned(),
            url: connected.final_url.to_string(),
            redirected: connected.redirected,
            headers: collect_response_headers(connected.response.headers()),
        };

        if !send(&tx, &wake, HttpEvent::Head(head)) {
            return;
        }

        let mut body = connected.response.into_body();
        let mut pending: Vec<u8> = Vec::new();
        let mut eof = false;

        loop {
            if cancel_flag.load(Ordering::Acquire) {
                return;
            }
            // Pull-driven backpressure: wait until JS requests the next chunk.
            pull_rx.notified().await;
            if cancel_flag.load(Ordering::Acquire) {
                return;
            }

            while pending.is_empty() && !eof {
                match timeout(req_timeout, body.frame()).await {
                    Ok(Some(Ok(frame))) => {
                        if let Ok(data) = frame.into_data()
                            && !data.is_empty()
                            && let Some(dec) = decoder.as_mut()
                        {
                            match dec.push(&data) {
                                Ok(decoded) => pending.extend_from_slice(&decoded),
                                Err(err) => {
                                    let _ = send(&tx, &wake, HttpEvent::Error(err));
                                    return;
                                }
                            }
                        }
                    }
                    Ok(Some(Err(err))) => {
                        let _ = send(
                            &tx,
                            &wake,
                            HttpEvent::Error(NetworkError::new("E_NETWORK", format!("body read failed: {err}"))),
                        );
                        return;
                    }
                    Ok(None) => {
                        eof = true;
                        if let Some(dec) = decoder.take() {
                            match dec.finish() {
                                Ok(tail) => pending.extend_from_slice(&tail),
                                Err(err) => {
                                    let _ = send(&tx, &wake, HttpEvent::Error(err));
                                    return;
                                }
                            }
                        }
                        break;
                    }
                    Err(_) => {
                        let _ = send(
                            &tx,
                            &wake,
                            HttpEvent::Error(NetworkError::new("E_TIMEOUT", "response body read timed out")),
                        );
                        return;
                    }
                }
            }

            if !pending.is_empty() {
                let take_len = pending.len().min(MAX_EMIT_CHUNK_BYTES);
                let chunk: Vec<u8> = pending.drain(..take_len).collect();
                let Some(lease) = budget.try_reserve(chunk.len()) else {
                    let _ = send(
                        &tx,
                        &wake,
                        HttpEvent::Error(NetworkError::new(
                            "E_LIMIT",
                            "session network byte retention budget exceeded",
                        )),
                    );
                    return;
                };
                if !send(&tx, &wake, HttpEvent::Chunk { data: chunk, _lease: lease }) {
                    return;
                }
            } else if eof {
                let _ = send(&tx, &wake, HttpEvent::End);
                return;
            }
        }
    });

    Ok(HttpHandle { inbox: rx, pull_notify, cancelled })
}

/// Spawns an incremental Server-Sent Events stream on the dedicated Tokio runtime.
pub fn spawn_event_stream(
    runtime: &tokio::runtime::Handle,
    spec: EventStreamSpec,
    policy: OriginPolicy,
    budget: SessionByteBudget,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<EventStreamHandle, NetworkError> {
    let (url, origin) = policy.authorize_http(&spec.url)?;
    let mut headers = spec.headers;
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("accept")) {
        headers.push(("accept".into(), "text/event-stream".into()));
    }
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("cache-control")) {
        headers.push(("cache-control".into(), "no-cache".into()));
    }
    if let Some(id) = spec.last_event_id
        && !id.is_empty()
        && !id.contains('\r')
        && !id.contains('\n')
    {
        headers.push(("last-event-id".into(), id));
    }
    let _ = build_header_map(&headers, false)?;

    let (tx, rx) = mpsc::sync_channel::<SseEvent>(SSE_EVENT_QUEUE_DEPTH);
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel_flag = cancelled.clone();
    let req_timeout = Duration::from_millis(spec.timeout_ms.clamp(100, 300_000));
    let max_redirects = spec.max_redirects.min(MAX_REDIRECTS);
    let max_event_bytes = if spec.max_event_bytes == 0 {
        DEFAULT_MAX_SSE_EVENT_BYTES
    } else {
        spec.max_event_bytes
    };

    runtime.spawn(async move {
        let send = |ev: SseEvent| -> bool {
            let ok = tx.send(ev).is_ok();
            if ok {
                wake();
            }
            ok
        };

        let connected = match send_with_redirects(RedirectRequest {
            method: Method::GET,
            url,
            origin,
            headers: &headers,
            body: Vec::new(),
            policy: &policy,
            timeout: req_timeout,
            max_redirects,
            cancelled: &cancel_flag,
        })
        .await
        {
            Ok(c) => c,
            Err(err) => {
                if !cancel_flag.load(Ordering::Acquire) {
                    let _ = send(SseEvent::Error(err));
                }
                return;
            }
        };

        let status = connected.response.status().as_u16();
        if !(200..300).contains(&status) {
            let _ = send(SseEvent::Error(NetworkError::new(
                "E_NETWORK",
                format!("SSE endpoint returned HTTP status {status}"),
            )));
            return;
        }

        let encoding = connected
            .response
            .headers()
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut decoder = match BodyDecoder::for_encoding(encoding.as_deref(), usize::MAX) {
            Ok(d) => d,
            Err(err) => {
                let _ = send(SseEvent::Error(err));
                return;
            }
        };

        if !send(SseEvent::Open {
            status,
            url: connected.final_url.to_string(),
            headers: collect_response_headers(connected.response.headers()),
        }) {
            return;
        }

        let mut parser = SseParser::new(max_event_bytes);
        let mut body = connected.response.into_body();
        let mut parsed = Vec::new();

        while let Some(frame_res) = body.frame().await {
            if cancel_flag.load(Ordering::Acquire) {
                return;
            }
            let frame = match frame_res {
                Ok(f) => f,
                Err(err) => {
                    let _ = send(SseEvent::Error(NetworkError::new(
                        "E_NETWORK",
                        format!("SSE stream read failed: {err}"),
                    )));
                    return;
                }
            };
            let Ok(data) = frame.into_data() else { continue };
            if data.is_empty() {
                continue;
            }
            let decoded = match decoder.push(&data) {
                Ok(d) => d,
                Err(err) => {
                    let _ = send(SseEvent::Error(err));
                    return;
                }
            };
            if let Err(err) = parser.push(&decoded, &mut parsed) {
                let _ = send(SseEvent::Error(err));
                return;
            }
            for message in parsed.drain(..) {
                let byte_size = message.event.len()
                    + message.data.len()
                    + message.id.as_ref().map_or(0, String::len);
                let Some(lease) = budget.try_reserve(byte_size) else {
                    let _ = send(SseEvent::Error(NetworkError::new(
                        "E_LIMIT",
                        "session network byte retention budget exceeded",
                    )));
                    return;
                };
                if !send(SseEvent::Message { message, _lease: lease }) {
                    return;
                }
            }
        }

        if !cancel_flag.load(Ordering::Acquire) {
            let _ = send(SseEvent::Close);
        }
    });

    Ok(EventStreamHandle { inbox: rx, cancelled })
}
