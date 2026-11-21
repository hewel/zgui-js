//! Incremental WHATWG Server-Sent Events parser with bounded incomplete-event buffering.
use super::NetworkError;

/// Default maximum bytes retained in an incomplete SSE event before aborting with `E_LIMIT`.
pub const DEFAULT_MAX_SSE_EVENT_BYTES: usize = 256 * 1024;
/// Hard ceiling for `max_event_bytes`.
pub const MAX_SSE_EVENT_BYTES: usize = 4 * 1024 * 1024;

/// One parsed SSE message dispatched on a blank line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseMessage {
    pub event: String,
    pub data: String,
    pub id: Option<String>,
    pub retry_ms: Option<u64>,
}

/// Incremental line/event state machine for `text/event-stream` payloads.
#[derive(Debug)]
pub struct SseParser {
    max_event_bytes: usize,
    line_buf: Vec<u8>,
    pending_cr: bool,
    stripped_bom: bool,
    event_type: String,
    data_buf: String,
    has_data: bool,
    last_event_id: Option<String>,
    retry_ms: Option<u64>,
}

impl SseParser {
    /// Creates an incremental parser bounded to `max_event_bytes` of buffered state.
    #[must_use]
    pub fn new(max_event_bytes: usize) -> Self {
        Self {
            max_event_bytes: max_event_bytes.clamp(1024, MAX_SSE_EVENT_BYTES),
            line_buf: Vec::new(),
            pending_cr: false,
            stripped_bom: false,
            event_type: String::new(),
            data_buf: String::new(),
            has_data: false,
            last_event_id: None,
            retry_ms: None,
        }
    }

    /// Last non-NUL `id:` field seen on the stream.
    #[must_use]
    pub fn last_event_id(&self) -> Option<&str> {
        self.last_event_id.as_deref()
    }

    /// Feeds raw bytes and appends any completed SSE events to `out`.
    pub fn push(&mut self, mut chunk: &[u8], out: &mut Vec<SseMessage>) -> Result<(), NetworkError> {
        if !self.stripped_bom && !chunk.is_empty() {
            self.stripped_bom = true;
            if let Some(rest) = chunk.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
                chunk = rest;
            }
        }

        for &byte in chunk {
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\r' {
                self.pending_cr = true;
                self.finish_line(out)?;
            } else if byte == b'\n' {
                self.finish_line(out)?;
            } else {
                self.line_buf.push(byte);
                self.check_budget()?;
            }
        }
        Ok(())
    }

    fn buffered_bytes(&self) -> usize {
        self.line_buf.len() + self.event_type.len() + self.data_buf.len()
    }

    fn check_budget(&self) -> Result<(), NetworkError> {
        if self.buffered_bytes() > self.max_event_bytes {
            return Err(NetworkError::new(
                "E_LIMIT",
                format!("SSE event buffer exceeded limit of {} bytes", self.max_event_bytes),
            ));
        }
        Ok(())
    }

    fn finish_line(&mut self, out: &mut Vec<SseMessage>) -> Result<(), NetworkError> {
        if self.line_buf.is_empty() {
            if self.has_data {
                if self.data_buf.ends_with('\n') {
                    self.data_buf.pop();
                }
                let event = if self.event_type.is_empty() {
                    "message".to_owned()
                } else {
                    std::mem::take(&mut self.event_type)
                };
                let data = std::mem::take(&mut self.data_buf);
                self.has_data = false;
                out.push(SseMessage {
                    event,
                    data,
                    id: self.last_event_id.clone(),
                    retry_ms: self.retry_ms.take(),
                });
            } else {
                self.event_type.clear();
            }
            return Ok(());
        }

        let line = String::from_utf8_lossy(&self.line_buf).into_owned();
        self.line_buf.clear();

        if line.starts_with(':') {
            return Ok(());
        }

        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line.as_str(), ""),
        };

        match field {
            "event" => {
                self.event_type.clear();
                self.event_type.push_str(value);
            }
            "data" => {
                self.has_data = true;
                self.data_buf.push_str(value);
                self.data_buf.push('\n');
            }
            "id" => {
                if !value.contains('\0') {
                    self.last_event_id = Some(value.to_owned());
                }
            }
            "retry"
                if !value.is_empty()
                    && value.bytes().all(|b| b.is_ascii_digit())
                    && let Ok(ms) = value.parse::<u64>() =>
            {
                self.retry_ms = Some(ms);
            }
            _ => {}
        }
        self.check_budget()
    }
}
