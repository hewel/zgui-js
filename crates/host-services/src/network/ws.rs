//! Full-duplex WebSocket transport over origin-verified TCP/TLS streams with explicit backpressure.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};
use futures_util::{SinkExt, StreamExt};
use tokio::{
    sync::mpsc as async_mpsc,
    time::timeout,
};
use tokio_tungstenite::{
    client_async_with_config,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        http::{HeaderName, HeaderValue},
        protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
    },
};
use super::{
    NetworkError,
    budget::{ByteLease, SessionByteBudget},
    policy::{OriginPolicy, connect_verified_stream},
};

/// Default maximum reassembled inbound WebSocket message (1 MiB).
pub const DEFAULT_MAX_WS_MESSAGE_BYTES: usize = 1024 * 1024;
/// Hard ceiling for `max_message_bytes` (8 MiB).
pub const MAX_WS_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
/// Default maximum queued outbound WebSocket bytes (1 MiB).
pub const DEFAULT_MAX_WS_OUTBOUND_BYTES: usize = 1024 * 1024;
/// Hard ceiling for queued outbound messages.
const MAX_OUTBOUND_FRAMES: usize = 64;
/// Bounded inbound event channel depth.
const INBOUND_QUEUE_DEPTH: usize = 64;

/// Owned specification for opening a WebSocket connection.
#[derive(Clone, Debug)]
pub struct WebSocketSpec {
    pub url: String,
    pub protocols: Vec<String>,
    pub headers: Vec<(String, String)>,
    pub timeout_ms: u64,
    pub max_message_bytes: usize,
    pub max_outbound_bytes: usize,
}

/// Outbound WebSocket payload.
#[derive(Debug)]
pub enum OutboundPayload {
    Text(String),
    Binary(Vec<u8>),
}

impl OutboundPayload {
    /// Byte length of the outbound frame payload.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::Binary(b) => b.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

enum OutboundCmd {
    Send { payload: OutboundPayload, _lease: ByteLease },
    Close { code: u16, reason: String },
}

/// Inbound WebSocket message payload.
#[derive(Debug)]
pub enum InboundPayload {
    Text(String),
    Binary(Vec<u8>),
}

/// Events delivered from an active WebSocket connection.
#[derive(Debug)]
pub enum WebSocketEvent {
    Open { protocol: Option<String> },
    Message { payload: InboundPayload, _lease: ByteLease },
    Error(NetworkError),
    Close { code: u16, reason: String, clean: bool },
}

/// Owner-thread handle to a full-duplex WebSocket with explicit send backpressure.
pub struct WebSocketHandle {
    inbox: Receiver<WebSocketEvent>,
    outbound: async_mpsc::Sender<OutboundCmd>,
    buffered_bytes: Arc<AtomicUsize>,
    max_outbound_bytes: usize,
    budget: SessionByteBudget,
    closed: Arc<AtomicBool>,
}

impl WebSocketHandle {
    /// Queues a text or binary frame, failing immediately with `E_BACKPRESSURE` when bounds are full.
    pub fn try_send(&self, payload: OutboundPayload) -> Result<usize, NetworkError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(NetworkError::new("E_NETWORK", "WebSocket is already closed"));
        }
        let len = payload.len();
        let current = self.buffered_bytes.load(Ordering::Acquire);
        if current.saturating_add(len) > self.max_outbound_bytes {
            return Err(NetworkError::new(
                "E_BACKPRESSURE",
                format!(
                    "WebSocket outbound buffer full ({current} + {len} > {} bytes)",
                    self.max_outbound_bytes
                ),
            ));
        }
        let Some(lease) = self.budget.try_reserve(len) else {
            return Err(NetworkError::new(
                "E_LIMIT",
                "session network byte retention budget exceeded",
            ));
        };
        let next = self.buffered_bytes.fetch_add(len, Ordering::AcqRel) + len;
        if let Err(err) = self.outbound.try_send(OutboundCmd::Send { payload, _lease: lease }) {
            self.buffered_bytes.fetch_sub(len, Ordering::AcqRel);
            return Err(match err {
                async_mpsc::error::TrySendError::Full(_) => {
                    NetworkError::new("E_BACKPRESSURE", "WebSocket outbound frame queue is full")
                }
                async_mpsc::error::TrySendError::Closed(_) => {
                    NetworkError::new("E_NETWORK", "WebSocket transport has closed")
                }
            });
        }
        Ok(next)
    }

    /// Current bytes queued for outbound transmission.
    #[must_use]
    pub fn buffered_amount(&self) -> usize {
        self.buffered_bytes.load(Ordering::Acquire)
    }

    /// Initiates a clean close handshake without blocking.
    pub fn close(&self, code: u16, reason: String) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            let _ = self.outbound.try_send(OutboundCmd::Close { code, reason });
        }
    }

    /// Non-blocking poll for the next inbound WebSocket event.
    pub fn try_recv(&self) -> Option<WebSocketEvent> {
        self.inbox.try_recv().ok()
    }
}

impl Drop for WebSocketHandle {
    fn drop(&mut self) {
        self.close(1001, String::new());
    }
}

/// Spawns a full-duplex WebSocket connection on the dedicated Tokio runtime.
pub fn spawn_websocket(
    runtime: &tokio::runtime::Handle,
    spec: WebSocketSpec,
    policy: OriginPolicy,
    budget: SessionByteBudget,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<WebSocketHandle, NetworkError> {
    let (url, origin) = policy.authorize_ws(&spec.url)?;
    let max_message_bytes = if spec.max_message_bytes == 0 {
        DEFAULT_MAX_WS_MESSAGE_BYTES
    } else {
        spec.max_message_bytes.clamp(1024, MAX_WS_MESSAGE_BYTES)
    };
    let max_outbound_bytes = if spec.max_outbound_bytes == 0 {
        DEFAULT_MAX_WS_OUTBOUND_BYTES
    } else {
        spec.max_outbound_bytes.clamp(1024, MAX_WS_MESSAGE_BYTES)
    };
    let connect_timeout = Duration::from_millis(spec.timeout_ms.clamp(100, 300_000));

    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| NetworkError::new("E_NETWORK_POLICY", format!("invalid WebSocket URL: {e}")))?;
    if !spec.protocols.is_empty() {
        let joined = spec.protocols.join(", ");
        let val = HeaderValue::from_str(&joined).map_err(|_| {
            NetworkError::new("E_NETWORK_POLICY", "invalid Sec-WebSocket-Protocol value")
        })?;
        request.headers_mut().insert("sec-websocket-protocol", val);
    }
    for (name, value) in &spec.headers {
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "host" | "cookie" | "upgrade" | "connection" | "sec-websocket-key" | "sec-websocket-version"
        ) {
            continue;
        }
        let hn = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            NetworkError::new("E_NETWORK_POLICY", format!("invalid header name: {name:?}"))
        })?;
        let hv = HeaderValue::from_str(value).map_err(|_| {
            NetworkError::new("E_NETWORK_POLICY", format!("invalid header value for {name:?}"))
        })?;
        request.headers_mut().append(hn, hv);
    }

    let (inbox_tx, inbox_rx) = mpsc::sync_channel::<WebSocketEvent>(INBOUND_QUEUE_DEPTH);
    let (out_tx, mut out_rx) = async_mpsc::channel::<OutboundCmd>(MAX_OUTBOUND_FRAMES);
    let buffered_bytes = Arc::new(AtomicUsize::new(0));
    let buffered_bg = buffered_bytes.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let closed_bg = closed.clone();
    let budget_bg = budget.clone();

    runtime.spawn(async move {
        let send_ev = |ev: WebSocketEvent| -> bool {
            let ok = inbox_tx.send(ev).is_ok();
            if ok {
                wake();
            }
            ok
        };

        let stream = match connect_verified_stream(&origin, &policy, connect_timeout).await {
            Ok(s) => s,
            Err(err) => {
                if !closed_bg.load(Ordering::Acquire) {
                    let _ = send_ev(WebSocketEvent::Error(err));
                    let _ = send_ev(WebSocketEvent::Close {
                        code: 1006,
                        reason: "connection failed".into(),
                        clean: false,
                    });
                }
                return;
            }
        };

        let mut ws_config = WebSocketConfig::default();
        ws_config.max_message_size = Some(max_message_bytes);
        ws_config.max_frame_size = Some(max_message_bytes);

        let (ws_stream, resp) = match timeout(
            connect_timeout,
            client_async_with_config(request, stream, Some(ws_config)),
        )
        .await
        {
            Ok(Ok(pair)) => pair,
            Ok(Err(err)) => {
                let _ = send_ev(WebSocketEvent::Error(NetworkError::new(
                    "E_NETWORK",
                    format!("WebSocket handshake failed: {err}"),
                )));
                let _ = send_ev(WebSocketEvent::Close {
                    code: 1006,
                    reason: "handshake failed".into(),
                    clean: false,
                });
                return;
            }
            Err(_) => {
                let _ = send_ev(WebSocketEvent::Error(NetworkError::new(
                    "E_TIMEOUT",
                    "WebSocket handshake timed out",
                )));
                let _ = send_ev(WebSocketEvent::Close {
                    code: 1006,
                    reason: "handshake timeout".into(),
                    clean: false,
                });
                return;
            }
        };

        let protocol = resp
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        if !send_ev(WebSocketEvent::Open { protocol }) {
            return;
        }

        let (mut sink, mut reader) = ws_stream.split();
        let mut close_info: Option<(u16, String, bool)> = None;

        loop {
            tokio::select! {
                cmd = out_rx.recv() => {
                    match cmd {
                        Some(OutboundCmd::Send { payload, _lease }) => {
                            let len = payload.len();
                            let msg = match payload {
                                OutboundPayload::Text(s) => Message::Text(s.into()),
                                OutboundPayload::Binary(b) => Message::Binary(b.into()),
                            };
                            let res = sink.send(msg).await;
                            buffered_bg.fetch_sub(len, Ordering::AcqRel);
                            drop(_lease);
                            if let Err(err) = res {
                                let _ = send_ev(WebSocketEvent::Error(NetworkError::new(
                                    "E_NETWORK",
                                    format!("WebSocket send failed: {err}"),
                                )));
                                close_info = Some((1006, "send error".into(), false));
                                break;
                            }
                        }
                        Some(OutboundCmd::Close { code, reason }) => {
                            let frame = CloseFrame {
                                code: CloseCode::from(code),
                                reason: reason.clone().into(),
                            };
                            let _ = sink.send(Message::Close(Some(frame))).await;
                            let _ = sink.close().await;
                            close_info = Some((code, reason, true));
                            break;
                        }
                        None => {
                            let _ = sink.close().await;
                            break;
                        }
                    }
                }
                incoming = reader.next() => {
                    match incoming {
                        Some(Ok(Message::Text(text))) => {
                            let text_str = text.to_string();
                            let Some(lease) = budget_bg.try_reserve(text_str.len()) else {
                                let _ = send_ev(WebSocketEvent::Error(NetworkError::new(
                                    "E_LIMIT",
                                    "session network byte retention budget exceeded",
                                )));
                                close_info = Some((1009, "budget exceeded".into(), false));
                                break;
                            };
                            if !send_ev(WebSocketEvent::Message {
                                payload: InboundPayload::Text(text_str),
                                _lease: lease,
                            }) {
                                break;
                            }
                        }
                        Some(Ok(Message::Binary(bytes))) => {
                            let data = bytes.to_vec();
                            let Some(lease) = budget_bg.try_reserve(data.len()) else {
                                let _ = send_ev(WebSocketEvent::Error(NetworkError::new(
                                    "E_LIMIT",
                                    "session network byte retention budget exceeded",
                                )));
                                close_info = Some((1009, "budget exceeded".into(), false));
                                break;
                            };
                            if !send_ev(WebSocketEvent::Message {
                                payload: InboundPayload::Binary(data),
                                _lease: lease,
                            }) {
                                break;
                            }
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            let _ = sink.send(Message::Pong(payload)).await;
                        }
                        Some(Ok(Message::Pong(_) | Message::Frame(_))) => {}
                        Some(Ok(Message::Close(frame))) => {
                            let (code, reason) = frame
                                .map(|f| (u16::from(f.code), f.reason.to_string()))
                                .unwrap_or((1005, String::new()));
                            close_info = Some((code, reason, true));
                            break;
                        }
                        Some(Err(err)) => {
                            let code = if err.to_string().contains("Space limit") || err.to_string().contains("too long") {
                                "E_LIMIT"
                            } else {
                                "E_NETWORK"
                            };
                            let _ = send_ev(WebSocketEvent::Error(NetworkError::new(
                                code,
                                format!("WebSocket receive failed: {err}"),
                            )));
                            close_info = Some((1006, err.to_string(), false));
                            break;
                        }
                        None => {
                            close_info = Some((1006, String::new(), false));
                            break;
                        }
                    }
                }
            }
        }

        closed_bg.store(true, Ordering::Release);
        if let Some((code, reason, clean)) = close_info {
            let _ = send_ev(WebSocketEvent::Close { code, reason, clean });
        }
    });

    Ok(WebSocketHandle {
        inbox: inbox_rx,
        outbound: out_tx,
        buffered_bytes,
        max_outbound_bytes,
        budget,
        closed,
    })
}
