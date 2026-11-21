//! Origin-allowlisted async network runtime for HTTP, SSE, and WebSocket transport.
//!
//! Runs on a dedicated Tokio runtime off the QuickJS thread and separate from
//! `host-services`'s synchronous asset worker pool.
pub mod budget;
pub mod http;
pub mod policy;
pub mod sse;
pub mod ws;

use std::{
    error::Error,
    fmt,
    io,
    sync::{Arc, OnceLock},
};

pub use budget::{ByteLease, DEFAULT_SESSION_NETWORK_BYTES, SessionByteBudget};
pub use http::{
    DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_TIMEOUT_MS, EventStreamHandle, EventStreamSpec,
    HttpEvent, HttpHandle, HttpRequestSpec, HttpResponseHead, MAX_REDIRECTS,
    MAX_REQUEST_BODY_BYTES, MAX_RESPONSE_BYTES, SseEvent,
};
pub use policy::{Origin, OriginPolicy, OriginScheme, is_restricted_host, is_restricted_ip};
pub use sse::{DEFAULT_MAX_SSE_EVENT_BYTES, MAX_SSE_EVENT_BYTES, SseMessage, SseParser};
pub use ws::{
    DEFAULT_MAX_WS_MESSAGE_BYTES, DEFAULT_MAX_WS_OUTBOUND_BYTES, InboundPayload,
    MAX_WS_MESSAGE_BYTES, OutboundPayload, WebSocketEvent, WebSocketHandle, WebSocketSpec,
};

/// Typed error returned across network policy, transport, backpressure, and cancellation seams.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkError {
    pub code: &'static str,
    pub message: String,
}

impl NetworkError {
    /// Constructs a new error with a stable machine-readable code.
    #[must_use]
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    /// Standard cancellation error.
    #[must_use]
    pub fn cancelled() -> Self {
        Self::new("Cancelled", "network operation was cancelled")
    }
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl Error for NetworkError {}

/// Lazily initialized dedicated async network runtime (never shares the sync asset pool).
#[derive(Clone, Default)]
pub struct NetworkRuntime {
    inner: Arc<OnceLock<tokio::runtime::Runtime>>,
}

impl NetworkRuntime {
    /// Creates a lazy handle; no OS threads are spawned until a live network operation executes.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn handle(&self) -> io::Result<&tokio::runtime::Handle> {
        if let Some(rt) = self.inner.get() {
            return Ok(rt.handle());
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("zgui-net")
            .enable_io()
            .enable_time()
            .build()?;
        let _ = self.inner.set(rt);
        Ok(self.inner.get().expect("network runtime initialized").handle())
    }

    /// Starts a pull-driven HTTP request authorized against `policy`.
    pub fn http_request(
        &self,
        spec: HttpRequestSpec,
        policy: OriginPolicy,
        budget: SessionByteBudget,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<HttpHandle, NetworkError> {
        let handle = self
            .handle()
            .map_err(|e| NetworkError::new("E_NETWORK", format!("failed to start network runtime: {e}")))?;
        http::spawn_http(handle, spec, policy, budget, wake)
    }

    /// Opens an incremental Server-Sent Events stream authorized against `policy`.
    pub fn open_event_stream(
        &self,
        spec: EventStreamSpec,
        policy: OriginPolicy,
        budget: SessionByteBudget,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<EventStreamHandle, NetworkError> {
        let handle = self
            .handle()
            .map_err(|e| NetworkError::new("E_NETWORK", format!("failed to start network runtime: {e}")))?;
        http::spawn_event_stream(handle, spec, policy, budget, wake)
    }

    /// Opens a full-duplex WebSocket connection authorized against `policy`.
    pub fn connect_websocket(
        &self,
        spec: WebSocketSpec,
        policy: OriginPolicy,
        budget: SessionByteBudget,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<WebSocketHandle, NetworkError> {
        let handle = self
            .handle()
            .map_err(|e| NetworkError::new("E_NETWORK", format!("failed to start network runtime: {e}")))?;
        ws::spawn_websocket(handle, spec, policy, budget, wake)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener},
        thread,
        time::{Duration, Instant},
    };
    use flate2::{Compression, write::GzEncoder};

    #[test]
    fn origin_normalization_and_policy_deny_insecure_private_and_wildcards() {
        let https = Origin::parse_manifest_entry("https://API.Example.COM:443").unwrap();
        assert_eq!(https.canonical(), "https://api.example.com");
        assert_eq!(https.port(), 443);

        let wss = Origin::parse_manifest_entry("wss://stream.example.com:8443").unwrap();
        assert_eq!(wss.canonical(), "wss://stream.example.com:8443");

        assert!(Origin::parse_manifest_entry("https://*.example.com").is_err());
        assert!(Origin::parse_manifest_entry("https://example.com/path").is_err());
        assert!(Origin::parse_manifest_entry("https://user:pass@example.com").is_err());

        // Insecure HTTP denied by default.
        let http_origin = Origin::parse_manifest_entry("http://example.com").unwrap();
        assert!(OriginPolicy::new([http_origin], false, false).is_err());

        // Private / loopback IPs denied by default.
        let loop_origin = Origin::parse_manifest_entry("https://127.0.0.1:8443").unwrap();
        assert!(OriginPolicy::new([loop_origin], false, false).is_err());

        assert!(is_restricted_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_restricted_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_restricted_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5))));
        assert!(is_restricted_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
        assert!(is_restricted_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        assert!(is_restricted_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_restricted_ip(IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped())));
        assert!(!is_restricted_ip(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))));

        // Authorized exact origin passes; different port or host fails with E_CAPABILITY.
        let policy = OriginPolicy::new([https, wss], false, false).unwrap();
        assert!(policy.authorize_http("https://api.example.com/v1/items").is_ok());
        assert_eq!(
            policy.authorize_http("https://api.example.com:8443/v1").unwrap_err().code,
            "E_CAPABILITY"
        );
        assert_eq!(
            policy.authorize_http("http://api.example.com/v1").unwrap_err().code,
            "E_CAPABILITY"
        );
        assert!(policy.authorize_ws("wss://stream.example.com:8443/live").is_ok());
        assert_eq!(
            policy.authorize_ws("https://api.example.com/live").unwrap_err().code,
            "E_NETWORK_POLICY"
        );
    }

    #[test]
    fn sse_parser_handles_chunked_multiline_events_and_enforces_incomplete_buffer_limit() {
        let mut parser = SseParser::new(1024);
        let mut events = Vec::new();
        parser.push(b"\xEF\xBB\xBF: keepalive\r\nevent: update\nid: 42\nretry: 1500\ndata: line 1\r\ndata: line 2\n\n", &mut events).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "update");
        assert_eq!(events[0].data, "line 1\nline 2");
        assert_eq!(events[0].id.as_deref(), Some("42"));
        assert_eq!(events[0].retry_ms, Some(1500));

        // Exceed incomplete buffer limit -> E_LIMIT.
        let big = vec![b'a'; 2000];
        let err = parser.push(&big, &mut events).unwrap_err();
        assert_eq!(err.code, "E_LIMIT");
    }

    #[test]
    fn live_http_gzip_cap_and_redirect_reauthorization_on_loopback() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for idx in 0..2 {
                let (mut sock, _) = listener.accept().unwrap();
                let mut req_buf = [0u8; 2048];
                let _ = sock.read(&mut req_buf);
                if idx == 0 {
                    // 1. Gzip-compressed 64 KiB payload to test bounded decompression cap.
                    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
                    gz.write_all(&vec![b'X'; 65536]).unwrap();
                    let body = gz.finish().unwrap();
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    sock.write_all(header.as_bytes()).unwrap();
                    sock.write_all(&body).unwrap();
                } else {
                    // 2. Redirect to an unauthorized origin -> must be rejected with E_CAPABILITY.
                    let resp = "HTTP/1.1 302 Found\r\nLocation: https://evil.example.com/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    sock.write_all(resp.as_bytes()).unwrap();
                }
            }
        });

        let origin = Origin::parse_manifest_entry(&format!("http://127.0.0.1:{}", addr.port())).unwrap();
        let policy = OriginPolicy::new([origin], true, true).unwrap();
        let runtime = NetworkRuntime::new();
        let budget = SessionByteBudget::default();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});

        // Test 1: Gzip bomb exceeds max_response_bytes = 4096 -> E_LIMIT.
        let handle = runtime
            .http_request(
                HttpRequestSpec {
                    method: "GET".into(),
                    url: format!("http://127.0.0.1:{}/bomb", addr.port()),
                    headers: vec![],
                    body: vec![],
                    timeout_ms: 5000,
                    max_response_bytes: 4096,
                    max_redirects: 3,
                },
                policy.clone(),
                budget.clone(),
                wake.clone(),
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_limit = false;
        while Instant::now() < deadline {
            if let Some(ev) = handle.try_recv() {
                match ev {
                    HttpEvent::Head(_) => handle.request_next_chunk(),
                    HttpEvent::Chunk { .. } => handle.request_next_chunk(),
                    HttpEvent::Error(e) => {
                        assert_eq!(e.code, "E_LIMIT");
                        saw_limit = true;
                        break;
                    }
                    HttpEvent::End => break,
                }
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
        assert!(saw_limit, "expected gzip response exceeding max_response_bytes to fail with E_LIMIT");
        assert_eq!(budget.used(), 0, "all byte leases must release cleanly");

        // Test 2: Redirect to unallowlisted origin -> E_CAPABILITY.
        let redir = runtime
            .http_request(
                HttpRequestSpec {
                    method: "GET".into(),
                    url: format!("http://127.0.0.1:{}/redir", addr.port()),
                    headers: vec![],
                    body: vec![],
                    timeout_ms: 5000,
                    max_response_bytes: 65536,
                    max_redirects: 3,
                },
                policy,
                budget,
                wake,
            )
            .unwrap();
        let mut saw_cap = false;
        let deadline2 = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline2 {
            if let Some(ev) = redir.try_recv() {
                if let HttpEvent::Error(e) = ev {
                    assert_eq!(e.code, "E_CAPABILITY");
                    saw_cap = true;
                    break;
                }
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
        assert!(saw_cap, "expected unauthorized redirect to fail with E_CAPABILITY");
        server.join().unwrap();
    }
}
