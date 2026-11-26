//! Deterministic preview network scripts (`http`, `sse`, `websockets`) on the fixture virtual clock.
use super::{
    ContractFailure, FixtureRuntime, ScheduleKey, ScheduleOrder, Scheduled, State, decode_base64,
    failure,
};
use crate::schema::object_struct;
use host_services::network::{HttpResponseHead, NetworkError, SseMessage};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_SCRIPTS_PER_KIND: usize = 256;
const MAX_STEPS_PER_SCRIPT: usize = 1024;
const MAX_NETWORK_DELIVERIES: usize = 2048;

object_struct! {
    /// Scripted network error payload.
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct NetworkErrorFixture {
        pub(super) code: String,
        pub(super) message: String,
    }
}

impl NetworkErrorFixture {
    fn to_network_error(&self) -> NetworkError {
        let code: &'static str = match self.code.as_str() {
            "E_CAPABILITY" => "E_CAPABILITY",
            "E_NETWORK_POLICY" => "E_NETWORK_POLICY",
            "E_DNS_POLICY" => "E_DNS_POLICY",
            "E_TIMEOUT" => "E_TIMEOUT",
            "E_LIMIT" => "E_LIMIT",
            "E_BACKPRESSURE" => "E_BACKPRESSURE",
            "E_PROTOCOL" => "E_PROTOCOL",
            "E_CANCELLED" => "E_CANCELLED",
            "E_CLOSED" => "E_CLOSED",
            "E_FIXTURE_UNMATCHED" => "E_FIXTURE_UNMATCHED",
            _ => "E_NETWORK",
        };
        NetworkError::new(code, self.message.clone())
    }
}

object_struct! {
    /// One chunk or terminal error in a scripted HTTP response body.
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct HttpChunkStep {
        #[serde(default)]
        pub(super) at_ms: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(super) text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(super) base64: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(super) error: Option<NetworkErrorFixture>,
    }
}

impl HttpChunkStep {
    fn validate(&self) -> Result<(), String> {
        let count = usize::from(self.text.is_some())
            + usize::from(self.base64.is_some())
            + usize::from(self.error.is_some());
        if count != 1 {
            return Err("fixture http chunk must specify exactly one of text, base64, or error".into());
        }
        if let Some(b64) = &self.base64 {
            let _ = decode_base64(b64)?;
        }
        Ok(())
    }

    fn decode_bytes(&self) -> Result<Vec<u8>, String> {
        if let Some(text) = &self.text {
            Ok(text.as_bytes().to_vec())
        } else if let Some(b64) = &self.base64 {
            decode_base64(b64)
        } else {
            Ok(Vec::new())
        }
    }
}

object_struct! {
    /// Scripted HTTP response definition.
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct HttpScript {
        #[serde(default = "default_get")]
        pub(super) method: String,
        pub(super) url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(super) body: Option<String>,
        #[serde(default)]
        pub(super) repeat: bool,
        #[serde(default)]
        pub(super) at_ms: u32,
        #[serde(default = "default_status_ok")]
        pub(super) status: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(super) status_text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(super) final_url: Option<String>,
        #[serde(default)]
        pub(super) redirected: bool,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pub(super) headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(super) response_body: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub(super) chunks: Vec<HttpChunkStep>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(super) error: Option<NetworkErrorFixture>,
    }
}

fn default_get() -> String {
    "GET".into()
}

fn default_status_ok() -> u16 {
    200
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum SseStep {
    Message {
        at_ms: u32,
        #[serde(default = "default_sse_event")]
        event: String,
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_ms: Option<u64>,
    },
    Error {
        at_ms: u32,
        code: String,
        message: String,
    },
    Close {
        at_ms: u32,
    },
}

fn default_sse_event() -> String {
    "message".into()
}

impl SseStep {
    fn at_ms(&self) -> u32 {
        match self {
            Self::Message { at_ms, .. } | Self::Error { at_ms, .. } | Self::Close { at_ms } => *at_ms,
        }
    }
}

object_struct! {
    /// Scripted Server-Sent Events stream definition.
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct SseScript {
        pub(super) url: String,
        #[serde(default)]
        pub(super) repeat: bool,
        #[serde(default)]
        pub(super) at_ms: u32,
        #[serde(default = "default_status_ok")]
        pub(super) status: u16,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pub(super) headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub(super) events: Vec<SseStep>,
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum WsStep {
    Text {
        at_ms: u32,
        data: String,
    },
    Binary {
        at_ms: u32,
        base64: String,
    },
    Error {
        at_ms: u32,
        code: String,
        message: String,
    },
    Close {
        at_ms: u32,
        #[serde(default = "default_ws_close_code")]
        code: u16,
        #[serde(default)]
        reason: String,
    },
}

fn default_ws_close_code() -> u16 {
    1000
}

impl WsStep {
    fn at_ms(&self) -> u32 {
        match self {
            Self::Text { at_ms, .. }
            | Self::Binary { at_ms, .. }
            | Self::Error { at_ms, .. }
            | Self::Close { at_ms, .. } => *at_ms,
        }
    }
}

object_struct! {
    /// Scripted WebSocket session definition.
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct WebSocketScript {
        pub(super) url: String,
        #[serde(default)]
        pub(super) repeat: bool,
        #[serde(default)]
        pub(super) at_ms: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(super) protocol: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(super) outbound_queue_limit: Option<usize>,
        #[serde(default)]
        pub(super) stall_sends: bool,
        #[serde(default)]
        pub(super) echo_sends: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub(super) expected_sends: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub(super) events: Vec<WsStep>,
    }
}

object_struct! {
    /// Deterministic preview network provider (`services.network`).
    #[derive(Default)]
    #[serde(default, rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct NetworkFixture {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub(super) http: Vec<HttpScript>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub(super) sse: Vec<SseScript>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub(super) websockets: Vec<WebSocketScript>,
    }
}

impl NetworkFixture {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.http.len() > MAX_SCRIPTS_PER_KIND
            || self.sse.len() > MAX_SCRIPTS_PER_KIND
            || self.websockets.len() > MAX_SCRIPTS_PER_KIND
        {
            return Err("fixture services.network exceeds script limit".into());
        }
        for script in &self.http {
            if script.url.is_empty() || script.method.is_empty() {
                return Err("fixture services.network.http requires non-empty method and url".into());
            }
            if script.chunks.len() > MAX_STEPS_PER_SCRIPT {
                return Err("fixture services.network.http exceeds chunk limit".into());
            }
            if script.response_body.is_some() && !script.chunks.is_empty() {
                return Err("fixture services.network.http cannot set both responseBody and chunks".into());
            }
            for chunk in &script.chunks {
                chunk.validate()?;
            }
        }
        for script in &self.sse {
            if script.url.is_empty() {
                return Err("fixture services.network.sse requires non-empty url".into());
            }
            if script.events.len() > MAX_STEPS_PER_SCRIPT {
                return Err("fixture services.network.sse exceeds event limit".into());
            }
        }
        for script in &self.websockets {
            if script.url.is_empty() {
                return Err("fixture services.network.websockets requires non-empty url".into());
            }
            if script.events.len() > MAX_STEPS_PER_SCRIPT {
                return Err("fixture services.network.websockets exceeds event limit".into());
            }
            for step in &script.events {
                if let WsStep::Binary { base64, .. } = step {
                    let _ = decode_base64(base64)?;
                }
            }
        }
        Ok(())
    }
}

/// Delivered network event ready for `Work::network` checkpoint dispatch.
#[derive(Debug)]
pub(crate) enum NetworkDelivery {
    HttpHead {
        generation: u64,
        id: u64,
        head: HttpResponseHead,
    },
    HttpChunk {
        generation: u64,
        id: u64,
        data: Vec<u8>,
    },
    HttpEnd {
        generation: u64,
        id: u64,
    },
    HttpError {
        generation: u64,
        id: u64,
        error: NetworkError,
    },
    SseOpen {
        generation: u64,
        id: u64,
        status: u16,
        url: String,
        headers: Vec<(String, String)>,
    },
    SseMessage {
        generation: u64,
        id: u64,
        message: SseMessage,
    },
    SseError {
        generation: u64,
        id: u64,
        error: NetworkError,
    },
    SseClose {
        generation: u64,
        id: u64,
    },
    WsOpen {
        generation: u64,
        id: u64,
        url: String,
        protocol: Option<String>,
    },
    WsText {
        generation: u64,
        id: u64,
        text: String,
    },
    WsBinary {
        generation: u64,
        id: u64,
        data: Vec<u8>,
    },
    WsError {
        generation: u64,
        id: u64,
        error: NetworkError,
    },
    WsClose {
        generation: u64,
        id: u64,
        code: u16,
        reason: String,
    },
}

impl NetworkDelivery {
    pub(crate) fn generation(&self) -> u64 {
        match self {
            Self::HttpHead { generation, .. }
            | Self::HttpChunk { generation, .. }
            | Self::HttpEnd { generation, .. }
            | Self::HttpError { generation, .. }
            | Self::SseOpen { generation, .. }
            | Self::SseMessage { generation, .. }
            | Self::SseError { generation, .. }
            | Self::SseClose { generation, .. }
            | Self::WsOpen { generation, .. }
            | Self::WsText { generation, .. }
            | Self::WsBinary { generation, .. }
            | Self::WsError { generation, .. }
            | Self::WsClose { generation, .. } => *generation,
        }
    }

    pub(crate) fn id(&self) -> u64 {
        match self {
            Self::HttpHead { id, .. }
            | Self::HttpChunk { id, .. }
            | Self::HttpEnd { id, .. }
            | Self::HttpError { id, .. }
            | Self::SseOpen { id, .. }
            | Self::SseMessage { id, .. }
            | Self::SseError { id, .. }
            | Self::SseClose { id, .. }
            | Self::WsOpen { id, .. }
            | Self::WsText { id, .. }
            | Self::WsBinary { id, .. }
            | Self::WsError { id, .. }
            | Self::WsClose { id, .. } => *id,
        }
    }
}

pub(super) enum NetworkJob {
    Delivery(NetworkDelivery),
}

struct ActiveHttp {
    generation: u64,
    remaining_chunks: VecDeque<HttpChunkStep>,
    chunk_seq: usize,
    max_response_bytes: usize,
    delivered_bytes: usize,
    closed: bool,
}

struct ActiveWs {
    generation: u64,
    outbound_queue_limit: usize,
    queued_sends: usize,
    stall_sends: bool,
    echo_sends: bool,
    expected_sends: Option<VecDeque<String>>,
    closed: bool,
}

#[derive(Default)]
pub(super) struct NetworkState {
    consumed_http: BTreeSet<usize>,
    consumed_sse: BTreeSet<usize>,
    consumed_ws: BTreeSet<usize>,
    active_http: BTreeMap<u64, ActiveHttp>,
    active_sse: BTreeMap<u64, u64>,
    active_ws: BTreeMap<u64, ActiveWs>,
    deliveries: VecDeque<NetworkDelivery>,
}

impl NetworkState {
    pub(super) fn delivery_count(&self) -> usize {
        self.deliveries.len()
    }

    pub(super) fn dispose(&mut self) {
        self.active_http.clear();
        self.active_sse.clear();
        self.active_ws.clear();
        self.deliveries.clear();
    }
}

impl FixtureRuntime {
    pub(crate) fn start_http_fixture(
        &self,
        generation: u64,
        request_id: u64,
        method: &str,
        url: &str,
        body: &[u8],
        max_response_bytes: usize,
    ) -> Result<(), ContractFailure> {
        let mut state = self.state.borrow_mut();
        self.admit(&mut state, "network", "httpRequest", request_id, generation)?;
        let Some(provider) = &self.definition.services.network else {
            return Err(self.record_failure(
                &mut state,
                failure("E_FIXTURE_UNMATCHED", format!("preview fixture has no services.network for HTTP {method} {url}")),
            ));
        };
        let body_utf8 = std::str::from_utf8(body).ok();
        let matched = provider.http.iter().enumerate().find(|(idx, script)| {
            (script.repeat || !state.network.consumed_http.contains(idx))
                && script.method.eq_ignore_ascii_case(method)
                && script.url == url
                && script.body.as_deref().is_none_or(|expected| body_utf8 == Some(expected))
        });
        let Some((idx, script)) = matched else {
            return Err(self.record_failure(
                &mut state,
                failure("E_FIXTURE_UNMATCHED", format!("unmatched fixture HTTP request: {method} {url}")),
            ));
        };
        if !script.repeat {
            state.network.consumed_http.insert(idx);
        }
        self.record(&mut state, "network.httpRequest", json!({
            "requestId": request_id,
            "method": method,
            "url": url,
            "bodyBytes": body.len(),
        }));
        let now = self.environment.now_us();
        let deadline = now.saturating_add(u64::from(script.at_ms) * 1000);
        if let Some(err) = &script.error {
            state.scheduled.insert(
                ScheduleKey {
                    deadline_us: deadline,
                    order: ScheduleOrder::Call { id: request_id, index: 0 },
                },
                Scheduled::Network {
                    generation,
                    job: NetworkJob::Delivery(NetworkDelivery::HttpError {
                        generation,
                        id: request_id,
                        error: err.to_network_error(),
                    }),
                },
            );
            return Ok(());
        }

        let mut remaining_chunks = VecDeque::new();
        if let Some(resp_body) = &script.response_body {
            remaining_chunks.push_back(HttpChunkStep {
                at_ms: 0,
                text: Some(resp_body.clone()),
                base64: None,
                error: None,
            });
        } else {
            for chunk in &script.chunks {
                remaining_chunks.push_back(chunk.clone());
            }
        }
        state.network.active_http.insert(
            request_id,
            ActiveHttp {
                generation,
                remaining_chunks,
                chunk_seq: 1,
                max_response_bytes,
                delivered_bytes: 0,
                closed: false,
            },
        );
        let head = HttpResponseHead {
            status: script.status,
            status_text: script
                .status_text
                .clone()
                .unwrap_or_else(|| if script.status == 200 { "OK".into() } else { String::new() }),
            url: script.final_url.clone().unwrap_or_else(|| url.to_owned()),
            redirected: script.redirected,
            headers: script
                .headers
                .iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
                .collect(),
        };
        state.scheduled.insert(
            ScheduleKey {
                deadline_us: deadline,
                order: ScheduleOrder::Call { id: request_id, index: 0 },
            },
            Scheduled::Network {
                generation,
                job: NetworkJob::Delivery(NetworkDelivery::HttpHead {
                    generation,
                    id: request_id,
                    head,
                }),
            },
        );
        Ok(())
    }

    pub(crate) fn pull_http_fixture_chunk(&self, request_id: u64) -> Result<(), ContractFailure> {
        let mut state = self.state.borrow_mut();
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        let Some(active) = state.network.active_http.get_mut(&request_id) else {
            return Ok(());
        };
        if active.closed {
            return Ok(());
        }
        let generation = active.generation;
        let seq = active.chunk_seq;
        active.chunk_seq += 1;
        let now = self.environment.now_us();

        let Some(step) = active.remaining_chunks.pop_front() else {
            active.closed = true;
            state.scheduled.insert(
                ScheduleKey {
                    deadline_us: now,
                    order: ScheduleOrder::Call { id: request_id, index: seq },
                },
                Scheduled::Network {
                    generation,
                    job: NetworkJob::Delivery(NetworkDelivery::HttpEnd {
                        generation,
                        id: request_id,
                    }),
                },
            );
            return Ok(());
        };

        let deadline = now.saturating_add(u64::from(step.at_ms) * 1000);
        let delivery = if let Some(err) = &step.error {
            active.closed = true;
            NetworkDelivery::HttpError {
                generation,
                id: request_id,
                error: err.to_network_error(),
            }
        } else {
            let decoded = step.decode_bytes();
            let bytes = match decoded {
                Ok(b) => b,
                Err(e) => return Err(self.record_failure(&mut state, failure("E_FIXTURE_CONTRACT", e))),
            };
            let next_total = active.delivered_bytes.saturating_add(bytes.len());
            if active.max_response_bytes > 0 && next_total > active.max_response_bytes {
                active.closed = true;
                NetworkDelivery::HttpError {
                    generation,
                    id: request_id,
                    error: NetworkError::new(
                        "E_LIMIT",
                        format!("response body exceeded limit of {} bytes", active.max_response_bytes),
                    ),
                }
            } else {
                active.delivered_bytes = next_total;
                NetworkDelivery::HttpChunk {
                    generation,
                    id: request_id,
                    data: bytes,
                }
            }
        };
        state.scheduled.insert(
            ScheduleKey {
                deadline_us: deadline,
                order: ScheduleOrder::Call { id: request_id, index: seq },
            },
            Scheduled::Network {
                generation,
                job: NetworkJob::Delivery(delivery),
            },
        );
        Ok(())
    }

    pub(crate) fn start_sse_fixture(
        &self,
        generation: u64,
        request_id: u64,
        url: &str,
    ) -> Result<(), ContractFailure> {
        let mut state = self.state.borrow_mut();
        self.admit(&mut state, "network", "openEventStream", request_id, generation)?;
        let Some(provider) = &self.definition.services.network else {
            return Err(self.record_failure(
                &mut state,
                failure("E_FIXTURE_UNMATCHED", format!("preview fixture has no services.network for SSE {url}")),
            ));
        };
        let matched = provider.sse.iter().enumerate().find(|(idx, script)| {
            (script.repeat || !state.network.consumed_sse.contains(idx)) && script.url == url
        });
        let Some((idx, script)) = matched else {
            return Err(self.record_failure(
                &mut state,
                failure("E_FIXTURE_UNMATCHED", format!("unmatched fixture SSE stream: {url}")),
            ));
        };
        if !script.repeat {
            state.network.consumed_sse.insert(idx);
        }
        state.network.active_sse.insert(request_id, generation);
        self.record(&mut state, "network.openEventStream", json!({
            "requestId": request_id,
            "url": url,
        }));
        let now = self.environment.now_us();
        let open_deadline = now.saturating_add(u64::from(script.at_ms) * 1000);
        state.scheduled.insert(
            ScheduleKey {
                deadline_us: open_deadline,
                order: ScheduleOrder::Call { id: request_id, index: 0 },
            },
            Scheduled::Network {
                generation,
                job: NetworkJob::Delivery(NetworkDelivery::SseOpen {
                    generation,
                    id: request_id,
                    status: script.status,
                    url: url.to_owned(),
                    headers: script
                        .headers
                        .iter()
                        .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
                        .collect(),
                }),
            },
        );
        for (i, step) in script.events.iter().enumerate() {
            let step_deadline = open_deadline.saturating_add(u64::from(step.at_ms()) * 1000);
            let delivery = match step {
                SseStep::Message { event, data, id, retry_ms, .. } => NetworkDelivery::SseMessage {
                    generation,
                    id: request_id,
                    message: SseMessage {
                        event: event.clone(),
                        data: data.clone(),
                        id: id.clone(),
                        retry_ms: *retry_ms,
                    },
                },
                SseStep::Error { code, message, .. } => NetworkDelivery::SseError {
                    generation,
                    id: request_id,
                    error: NetworkErrorFixture {
                        code: code.clone(),
                        message: message.clone(),
                    }
                    .to_network_error(),
                },
                SseStep::Close { .. } => NetworkDelivery::SseClose {
                    generation,
                    id: request_id,
                },
            };
            state.scheduled.insert(
                ScheduleKey {
                    deadline_us: step_deadline,
                    order: ScheduleOrder::Call { id: request_id, index: i + 1 },
                },
                Scheduled::Network {
                    generation,
                    job: NetworkJob::Delivery(delivery),
                },
            );
        }
        Ok(())
    }

    pub(crate) fn start_ws_fixture(
        &self,
        generation: u64,
        request_id: u64,
        url: &str,
    ) -> Result<(), ContractFailure> {
        let mut state = self.state.borrow_mut();
        self.admit(&mut state, "network", "connectWebSocket", request_id, generation)?;
        let Some(provider) = &self.definition.services.network else {
            return Err(self.record_failure(
                &mut state,
                failure("E_FIXTURE_UNMATCHED", format!("preview fixture has no services.network for WS {url}")),
            ));
        };
        let matched = provider.websockets.iter().enumerate().find(|(idx, script)| {
            (script.repeat || !state.network.consumed_ws.contains(idx)) && script.url == url
        });
        let Some((idx, script)) = matched else {
            return Err(self.record_failure(
                &mut state,
                failure("E_FIXTURE_UNMATCHED", format!("unmatched fixture WebSocket: {url}")),
            ));
        };
        if !script.repeat {
            state.network.consumed_ws.insert(idx);
        }
        state.network.active_ws.insert(
            request_id,
            ActiveWs {
                generation,
                outbound_queue_limit: script.outbound_queue_limit.unwrap_or(64).max(1),
                queued_sends: 0,
                stall_sends: script.stall_sends,
                echo_sends: script.echo_sends,
                expected_sends: script.expected_sends.as_ref().map(|v| v.iter().cloned().collect()),
                closed: false,
            },
        );
        self.record(&mut state, "network.connectWebSocket", json!({
            "requestId": request_id,
            "url": url,
        }));
        let now = self.environment.now_us();
        let open_deadline = now.saturating_add(u64::from(script.at_ms) * 1000);
        state.scheduled.insert(
            ScheduleKey {
                deadline_us: open_deadline,
                order: ScheduleOrder::Call { id: request_id, index: 0 },
            },
            Scheduled::Network {
                generation,
                job: NetworkJob::Delivery(NetworkDelivery::WsOpen {
                    generation,
                    id: request_id,
                    url: url.to_owned(),
                    protocol: script.protocol.clone(),
                }),
            },
        );
        for (i, step) in script.events.iter().enumerate() {
            let step_deadline = open_deadline.saturating_add(u64::from(step.at_ms()) * 1000);
            let delivery = match step {
                WsStep::Text { data, .. } => NetworkDelivery::WsText {
                    generation,
                    id: request_id,
                    text: data.clone(),
                },
                WsStep::Binary { base64, .. } => {
                    let bytes = decode_base64(base64)
                        .map_err(|e| self.record_failure(&mut state, failure("E_FIXTURE_CONTRACT", e)))?;
                    NetworkDelivery::WsBinary {
                        generation,
                        id: request_id,
                        data: bytes,
                    }
                }
                WsStep::Error { code, message, .. } => NetworkDelivery::WsError {
                    generation,
                    id: request_id,
                    error: NetworkErrorFixture {
                        code: code.clone(),
                        message: message.clone(),
                    }
                    .to_network_error(),
                },
                WsStep::Close { code, reason, .. } => NetworkDelivery::WsClose {
                    generation,
                    id: request_id,
                    code: *code,
                    reason: reason.clone(),
                },
            };
            state.scheduled.insert(
                ScheduleKey {
                    deadline_us: step_deadline,
                    order: ScheduleOrder::Call { id: request_id, index: i + 1 },
                },
                Scheduled::Network {
                    generation,
                    job: NetworkJob::Delivery(delivery),
                },
            );
        }
        Ok(())
    }

    pub(crate) fn send_ws_fixture_text(
        &self,
        request_id: u64,
        text: String,
    ) -> Result<(), NetworkError> {
        let mut state = self.state.borrow_mut();
        let Some(ws) = state.network.active_ws.get_mut(&request_id) else {
            return Err(NetworkError::new("E_CLOSED", "WebSocket is closed"));
        };
        if ws.closed {
            return Err(NetworkError::new("E_CLOSED", "WebSocket is closed"));
        }
        if ws.stall_sends && ws.queued_sends >= ws.outbound_queue_limit {
            return Err(NetworkError::new(
                "E_BACKPRESSURE",
                "WebSocket outbound send queue is full",
            ));
        }
        if ws.stall_sends {
            ws.queued_sends += 1;
        }
        if let Some(expected) = ws.expected_sends.as_mut() {
            match expected.pop_front() {
                Some(want) if want == text => {}
                other => {
                    let msg = format!(
                        "WebSocket send mismatch: expected {other:?}, got {text:?}"
                    );
                    let _ = self.record_failure(&mut state, failure("E_FIXTURE_CONTRACT", msg.clone()));
                    return Err(NetworkError::new("E_FIXTURE_CONTRACT", msg));
                }
            }
        }
        let echo = ws.echo_sends;
        let generation = ws.generation;
        self.record(&mut state, "network.wsSend", json!({
            "requestId": request_id,
            "kind": "text",
            "bytes": text.len(),
        }));
        if echo {
            state.network.deliveries.push_back(NetworkDelivery::WsText {
                generation,
                id: request_id,
                text,
            });
        }
        Ok(())
    }

    pub(crate) fn send_ws_fixture_binary(
        &self,
        request_id: u64,
        bytes: Vec<u8>,
    ) -> Result<(), NetworkError> {
        let mut state = self.state.borrow_mut();
        let Some(ws) = state.network.active_ws.get_mut(&request_id) else {
            return Err(NetworkError::new("E_CLOSED", "WebSocket is closed"));
        };
        if ws.closed {
            return Err(NetworkError::new("E_CLOSED", "WebSocket is closed"));
        }
        if ws.stall_sends && ws.queued_sends >= ws.outbound_queue_limit {
            return Err(NetworkError::new(
                "E_BACKPRESSURE",
                "WebSocket outbound send queue is full",
            ));
        }
        if ws.stall_sends {
            ws.queued_sends += 1;
        }
        let echo = ws.echo_sends;
        let generation = ws.generation;
        self.record(&mut state, "network.wsSend", json!({
            "requestId": request_id,
            "kind": "binary",
            "bytes": bytes.len(),
        }));
        if echo {
            state.network.deliveries.push_back(NetworkDelivery::WsBinary {
                generation,
                id: request_id,
                data: bytes,
            });
        }
        Ok(())
    }

    pub(crate) fn cancel_network_fixture(&self, request_id: u64) {
        let mut state = self.state.borrow_mut();
        state.network.active_http.remove(&request_id);
        state.network.active_sse.remove(&request_id);
        if let Some(mut ws) = state.network.active_ws.remove(&request_id) {
            ws.closed = true;
        }
        state.scheduled.retain(|_, item| {
            !matches!(item, Scheduled::Network { job: NetworkJob::Delivery(d), .. } if d.id() == request_id)
        });
        state.network.deliveries.retain(|d| d.id() != request_id);
        self.record(&mut state, "network.cancel", json!({ "requestId": request_id }));
    }

    pub(crate) fn cancel_network_generation(&self, generation: u64) {
        let mut state = self.state.borrow_mut();
        state.network.active_http.retain(|_, v| v.generation != generation);
        state.network.active_sse.retain(|_, g| *g != generation);
        state.network.active_ws.retain(|_, v| v.generation != generation);
        state.scheduled.retain(|_, item| {
            !matches!(item, Scheduled::Network { generation: g, .. } if *g == generation)
        });
        state.network.deliveries.retain(|d| d.generation() != generation);
    }

    pub(super) fn enqueue_network(
        &self,
        state: &mut State,
        generation: u64,
        job: NetworkJob,
    ) -> Result<(), ContractFailure> {
        let NetworkJob::Delivery(delivery) = job;
        let id = delivery.id();
        let active = match &delivery {
            NetworkDelivery::HttpHead { .. }
            | NetworkDelivery::HttpChunk { .. }
            | NetworkDelivery::HttpEnd { .. }
            | NetworkDelivery::HttpError { .. } => {
                // Allow standalone error scheduled before active_http insertion too.
                state.network.active_http.get(&id).is_none_or(|a| a.generation == generation)
            }
            NetworkDelivery::SseOpen { .. }
            | NetworkDelivery::SseMessage { .. }
            | NetworkDelivery::SseError { .. }
            | NetworkDelivery::SseClose { .. } => {
                state.network.active_sse.get(&id).copied() == Some(generation)
            }
            NetworkDelivery::WsOpen { .. }
            | NetworkDelivery::WsText { .. }
            | NetworkDelivery::WsBinary { .. }
            | NetworkDelivery::WsError { .. }
            | NetworkDelivery::WsClose { .. } => {
                state.network.active_ws.get(&id).is_some_and(|w| w.generation == generation && !w.closed)
            }
        };
        if !active {
            return Ok(());
        }
        if state.network.deliveries.len() >= MAX_NETWORK_DELIVERIES {
            return Err(self.record_failure(
                state,
                failure("E_FIXTURE_CONTRACT", "fixture network delivery queue exceeded limit"),
            ));
        }
        state.network.deliveries.push_back(delivery);
        Ok(())
    }

    pub(crate) fn take_network(&self) -> Option<NetworkDelivery> {
        self.state.borrow_mut().network.deliveries.pop_front()
    }

    pub(crate) fn pending_network_deliveries(&self, generation: u64) -> usize {
        self.state.borrow().network.deliveries.iter().filter(|event| event.generation() == generation).count()
    }

    pub(crate) fn has_network_delivery(&self, generation: u64) -> bool {
        self.state.borrow().network.deliveries.iter().any(|event| event.generation() == generation)
    }
}
