//! Dev-only, host-owned, strictly read-only Desktop Runtime Inspector & Profiler.
//!
//! Shares the native-only bounded tree/style snapshot primitives with preview
//! inspection, pairs them with `RuntimeTelemetry`, `ResourceCounts`, and
//! `CompletedFrame` phase timings, and enforces:
//! - zero app JS getter/eval execution,
//! - mandatory redaction of secure/password fields,
//! - mandatory exclusion of `SessionLockSurface` content,
//! - explicit labeling of historical/incoherent frames,
//! - zero overhead when disabled (no `FrameProbe`, no ring buffer, no socket).

use std::{
    cell::RefCell,
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use serde::Serialize;
use serde_json::{Value, json};
use zgui::{
    platform::{WakeReason, Waker},
    reactive::prelude::GetUntracked,
    runtime::{FrameProbe, Runtime, Window},
    view::{NodeId, ViewHost},
    vocab::Role,
};

use crate::{
    preview::evidence::{Bounds, ScrollMetrics},
    session::{ResourceCounts, RuntimeTelemetry, Session, SessionRegion},
};

const MAX_RING_SAMPLES: usize = 256;
const MAX_DEV_NODES: usize = 4096;
const MAX_DEV_TEXT_BYTES: usize = 128 * 1024;
const MAX_DEV_NODE_TEXT_BYTES: usize = 4096;
const MAX_DEV_VISITS: usize = 100_000;
const MAX_REQUEST_LINE_BYTES: usize = 4 * 1024;
const MAX_RESPONSE_BYTES: usize = 512 * 1024;

/// Visibility means a positive native box with computed `visibility: visible`,
/// not pointer reachability. Shared between preview inspection and dev inspector.
pub(crate) fn native_state(window: &Window, native: NodeId) -> Option<(bool, bool)> {
    let key = zgui_view_dom::id::to_document(native)?;
    let document = window.document().borrow();
    let mut next = Some(document.store().index_of(key)?);
    let mut enabled = true;
    while let Some(index) = next {
        let core = document.store().core(index);
        enabled &= !core.ui_state().contains(zgui::vocab::UiState::DISABLED);
        next = core.parent();
    }
    let layout = window.layout().borrow();
    let visible = layout.boxes_of(key).iter().any(|box_| {
        layout.get(*box_).is_some_and(|node| {
            node.style.get_inherited_box().visibility
                == zgui_css::values::size::VisibilityValue::Visible
        }) && layout.fragments_of_box(*box_).iter().any(|fragment| {
            layout.fragment(*fragment).is_some_and(|fragment| {
                fragment.border_box.size.width.0 > 0.0 && fragment.border_box.size.height.0 > 0.0
            })
        })
    });
    Some((enabled, visible))
}

/// Read scroll metrics for a native scroll container without running JS.
pub(crate) fn scroll_metrics(window: &Window, native: NodeId, scale: f64) -> Option<ScrollMetrics> {
    let key = zgui_view_dom::id::to_document(native)?;
    let region = {
        let layout = window.layout().borrow();
        zgui_layout::scroll_region::region_of_element(&layout, key)?
    };
    let position = ViewHost::scroll_position(window.host().as_ref(), native);
    let epsilon = f64::from(zgui_layout::scroll_region::bar::EPSILON);
    let viewport_width = f64::from(region.scrollport.size.width.0);
    let viewport_height = f64::from(region.scrollport.size.height.0);
    let content_width = f64::from(region.content.width.0);
    let content_height = f64::from(region.content.height.0);
    Some(ScrollMetrics {
        offset_x: f64::from(position.offset.x.0) / scale,
        offset_y: f64::from(position.offset.y.0) / scale,
        viewport_width: viewport_width / scale,
        viewport_height: viewport_height / scale,
        content_width: content_width / scale,
        content_height: content_height / scale,
        overflow_x: region.scrolls_x && content_width > viewport_width + epsilon,
        overflow_y: region.scrolls_y && content_height > viewport_height + epsilon,
        scrolls_x: region.scrolls_x,
        scrolls_y: region.scrolls_y,
    })
}

/// Bounded native text walk across a DOM subtree.
pub(crate) fn bounded_text(
    document: &zgui_dom::Document,
    root: zgui_dom::NodeIndex,
    bytes: usize,
    visits: &mut usize,
) -> (String, bool) {
    let mut text = String::new();
    let mut pending = vec![root];
    while let Some(index) = pending.pop() {
        if *visits == 0 {
            return (text, true);
        }
        *visits -= 1;
        if let Some(own) = zgui_dom::text::node::text_of(document.store(), index) {
            let keep = own.floor_char_boundary(bytes.saturating_sub(text.len()).min(own.len()));
            text.push_str(&own[..keep]);
            if keep < own.len() {
                return (text, true);
            }
        }
        let core = document.store().core(index);
        if index != root
            && let Some(next) = core.next_sibling()
        {
            pending.push(next);
        }
        if let Some(child) = core.first_child() {
            pending.push(child);
        }
    }
    (text, false)
}

/// Whether `native` or any of its ancestors is a secure / password text control.
pub(crate) fn is_secure_node(window: &Window, native: NodeId) -> bool {
    if window
        .dom()
        .semantics(native)
        .is_some_and(|sem| sem.role == Role::PasswordInput)
    {
        return true;
    }
    let Some(key) = zgui_view_dom::id::to_document(native) else {
        return false;
    };
    let document = window.document().borrow();
    let mut next = document.store().index_of(key);
    while let Some(index) = next {
        let record = document.store().core(index);
        if zgui_dom::Node::new(record).attr("secure").is_some() {
            return true;
        }
        next = record.parent();
    }
    false
}

/// Bounded native text extraction with mandatory secure/password redaction for
/// the dev inspector (never copies raw secret text from secure subtrees).
fn bounded_redacted_text(
    window: &Window,
    document: &zgui_dom::Document,
    native: NodeId,
    root: zgui_dom::NodeIndex,
    bytes: usize,
    visits: &mut usize,
) -> (String, bool, bool) {
    if is_secure_node(window, native) {
        let (raw, truncated) = bounded_text(document, root, 1, visits);
        let redacted = if raw.is_empty() {
            String::new()
        } else {
            "[REDACTED]".to_owned()
        };
        return (redacted, truncated, true);
    }
    // Walk descendants, redacting any nested secure subtrees without exposing raw bytes.
    let mut text = String::new();
    let mut pending = vec![root];
    let mut redacted_any = false;
    while let Some(index) = pending.pop() {
        if *visits == 0 {
            return (text, true, redacted_any);
        }
        *visits -= 1;
        let record = document.store().core(index);
        if zgui_dom::Node::new(record).attr("secure").is_some() {
            redacted_any = true;
            let (raw, _) = bounded_text(document, index, 1, visits);
            if !raw.is_empty() && text.len() + "[REDACTED]".len() <= bytes {
                text.push_str("[REDACTED]");
            }
            let core = document.store().core(index);
            if index != root
                && let Some(next) = core.next_sibling()
            {
                pending.push(next);
            }
            continue;
        }
        if let Some(own) = zgui_dom::text::node::text_of(document.store(), index) {
            let keep = own.floor_char_boundary(bytes.saturating_sub(text.len()).min(own.len()));
            text.push_str(&own[..keep]);
            if keep < own.len() {
                return (text, true, redacted_any);
            }
        }
        let core = document.store().core(index);
        if index != root
            && let Some(next) = core.next_sibling()
        {
            pending.push(next);
        }
        if let Some(child) = core.first_child() {
            pending.push(child);
        }
    }
    (text, false, redacted_any)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrameSample {
    pub surface_id: u64,
    pub ordinal: u64,
    pub outcome: String,
    pub document_revision: u64,
    pub style_revision: u64,
    pub layout_revision: Option<u64>,
    pub coherent: bool,
    pub style_us: u64,
    pub layout_us: u64,
    pub paint_us: u64,
    pub total_us: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckpointSample {
    pub generation: u64,
    pub queue_depth: usize,
    pub js_entry_max_us: u64,
    pub js_heap_bytes: u64,
}

#[derive(Default)]
struct ProfilerRing {
    frames: VecDeque<FrameSample>,
    checkpoints: VecDeque<CheckpointSample>,
}

impl ProfilerRing {
    fn push_frame(&mut self, sample: FrameSample) {
        if self.frames.len() >= MAX_RING_SAMPLES {
            self.frames.pop_front();
        }
        self.frames.push_back(sample);
    }

    fn push_checkpoint(&mut self, sample: CheckpointSample) {
        if self.checkpoints.len() >= MAX_RING_SAMPLES {
            self.checkpoints.pop_front();
        }
        self.checkpoints.push_back(sample);
    }
}

/// Dev-only inspector state & bounded profiler ring buffer. Installed only when
/// explicitly enabled via `--dev-inspect`; absent (`None`) in non-dev/release runs.
pub(crate) struct DevInspector {
    ring: RefCell<ProfilerRing>,
}

impl DevInspector {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            ring: RefCell::new(ProfilerRing::default()),
        })
    }

    pub(crate) fn probe(self: &Rc<Self>) -> Rc<dyn FrameProbe> {
        Rc::new(DevFrameProbe {
            inspector: Rc::clone(self),
        })
    }

    pub(crate) fn record_checkpoint(
        &self,
        generation: u64,
        queue_depth: usize,
        telemetry: Option<RuntimeTelemetry>,
    ) {
        let (js_entry_max_us, js_heap_bytes) = telemetry.map_or((0, 0), |t| {
            (t.max_entry_duration.as_micros() as u64, t.js_heap_bytes)
        });
        self.ring.borrow_mut().push_checkpoint(CheckpointSample {
            generation,
            queue_depth,
            js_entry_max_us,
            js_heap_bytes,
        });
    }

    fn record_frame(&self, window: &Window) {
        let Some(completed) = window.completed_frame() else {
            return;
        };
        let surface_id = format!("{:?}", window.surface().id());
        // Extract numeric surface id if formatted as SurfaceId(n) or fallback to ordinal hash.
        let numeric_surface = surface_id
            .trim_start_matches("SurfaceId(")
            .trim_end_matches(')')
            .parse::<u64>()
            .unwrap_or(0);
        self.ring.borrow_mut().push_frame(FrameSample {
            surface_id: numeric_surface,
            ordinal: completed.ordinal,
            outcome: format!("{:?}", completed.outcome),
            document_revision: completed.document_revision,
            style_revision: completed.style_revision,
            layout_revision: completed.layout_revision,
            coherent: completed.layout_document_coherent,
            style_us: completed.phases.style_us,
            layout_us: completed.phases.layout_us,
            paint_us: completed.phases.paint_us,
            total_us: completed.phases.total_us,
        });
    }

    pub(crate) fn profiler_snapshot(&self) -> Value {
        let ring = self.ring.borrow();
        json!({
            "maxSamples": MAX_RING_SAMPLES,
            "frameCount": ring.frames.len(),
            "checkpointCount": ring.checkpoints.len(),
            "frames": ring.frames,
            "checkpoints": ring.checkpoints,
        })
    }

    /// Build a strictly read-only, native-only inspection payload.
    pub(crate) fn capture_snapshot(
        &self,
        runtime: &Runtime,
        session: Option<&Session>,
        include_tree: bool,
        include_profile: bool,
        surface_filter: Option<&str>,
    ) -> Value {
        let session_locked = session.is_some_and(|s| s.shell.lock_is_retained());
        let generation = session.map_or(0, |s| s.generation);
        let resources: ResourceCounts = session.map_or_else(ResourceCounts::default, Session::counts);
        let telemetry = session.and_then(Session::live_telemetry);

        let mut surfaces = Vec::new();
        if let Some(session) = session {
            for (name, region) in session.mount_regions() {
                if let Some(filter) = surface_filter
                    && filter != name
                {
                    continue;
                }
                let window = runtime
                    .windows()
                    .iter()
                    .find(|w| w.document_id() == region.region.document);
                surfaces.push(snapshot_surface(
                    &name,
                    &region,
                    window,
                    generation,
                    session_locked,
                    include_tree,
                ));
            }
        }

        let mut response = json!({
            "schemaVersion": 1,
            "devOnly": true,
            "readOnly": true,
            "generation": generation,
            "sessionLocked": session_locked,
            "resources": resources,
            "telemetry": telemetry.map(|t| json!({
                "maxEntryDurationUs": t.max_entry_duration.as_micros() as u64,
                "jsHeapBytes": t.js_heap_bytes,
                "jsHeapLiveBytes": t.js_heap_live_bytes,
                "jsStackLimitBytes": t.js_stack_limit_bytes,
            })),
            "surfaces": surfaces,
        });
        if include_profile {
            response["profiler"] = self.profiler_snapshot();
        }
        response
    }
}

struct DevFrameProbe {
    inspector: Rc<DevInspector>,
}

impl FrameProbe for DevFrameProbe {
    fn frame_ended(&self, window: &Window) {
        self.inspector.record_frame(window);
    }

    fn describe(&self) -> &str {
        "zgui-js dev inspector frame probe"
    }
}

fn snapshot_surface(
    name: &str,
    region: &SessionRegion,
    window: Option<&Window>,
    generation: u64,
    session_locked: bool,
    include_tree: bool,
) -> Value {
    // Strictly forbid capture of SessionLockSurface content or any surface while session-locked.
    if region.is_lock_surface() || session_locked {
        return json!({
            "surface": name,
            "generation": generation,
            "excluded": true,
            "reason": "session-lock-surface-forbidden",
            "nodes": [],
        });
    }
    let Some(window) = window else {
        return json!({
            "surface": name,
            "generation": generation,
            "coherenceStatus": "no-window",
            "coherent": false,
            "nodes": [],
        });
    };

    let live_doc_rev = window.dom().revision();
    let live_style_rev = window.style_revision();
    let completed = window.completed_frame();

    let (coherence_status, coherent, frame_meta) = match completed {
        None => (
            "no-completed-frame",
            false,
            json!({
                "frameOrdinal": null,
                "frameDocumentRevision": null,
                "liveDocumentRevision": live_doc_rev,
                "consumedStyleRevision": null,
                "liveStyleRevision": live_style_rev,
                "consumedLayoutRevision": null,
            }),
        ),
        Some(frame) => {
            let is_coherent = frame.layout_document_coherent
                && frame.document_revision == live_doc_rev
                && frame.style_revision == live_style_rev;
            let status = if is_coherent {
                "coherent"
            } else {
                "historical-incoherent"
            };
            (
                status,
                is_coherent,
                json!({
                    "frameOrdinal": frame.ordinal,
                    "outcome": format!("{:?}", frame.outcome),
                    "frameDocumentRevision": frame.document_revision,
                    "liveDocumentRevision": live_doc_rev,
                    "consumedStyleRevision": frame.style_revision,
                    "liveStyleRevision": live_style_rev,
                    "consumedLayoutRevision": frame.layout_revision,
                    "phases": {
                        "styleUs": frame.phases.style_us,
                        "layoutUs": frame.phases.layout_us,
                        "paintUs": frame.phases.paint_us,
                        "totalUs": frame.phases.total_us,
                    },
                }),
            )
        }
    };

    let mut payload = json!({
        "surface": name,
        "surfaceId": format!("{:?}", region.surface),
        "generation": generation,
        "excluded": false,
        "coherent": coherent,
        "coherenceStatus": coherence_status,
        "frame": frame_meta,
    });

    if !include_tree {
        return payload;
    }

    let owned = region.bridge.snapshot(MAX_DEV_NODES);
    let document = window.document().borrow();
    let focused = region.region.host.focused().get_untracked();
    let scale = window.surface().scale_factor();
    let mut text_budget = MAX_DEV_TEXT_BYTES;
    let mut visit_budget = MAX_DEV_VISITS;
    let mut text_truncated = false;
    let mut nodes = Vec::with_capacity(owned.nodes.len());

    for node in &owned.nodes {
        let index = zgui_view_dom::id::resolve(&document, node.native);
        let (text, truncated, redacted) = bounded_redacted_text(
            window,
            &document,
            node.native,
            index,
            text_budget.min(MAX_DEV_NODE_TEXT_BYTES),
            &mut visit_budget,
        );
        text_budget = text_budget.saturating_sub(text.len());
        text_truncated |= truncated;

        // Only attach frame layout bounds when the completed frame is coherent with live DOM;
        // never silently join live tree nodes to stale/incoherent layout geometry.
        let bounds_logical = if coherent {
            ViewHost::window_box(window.host().as_ref(), node.native).map(|bounds| Bounds {
                x: f64::from(bounds.origin.x.0) / scale,
                y: f64::from(bounds.origin.y.0) / scale,
                width: f64::from(bounds.size.width.0) / scale,
                height: f64::from(bounds.size.height.0) / scale,
            })
        } else {
            None
        };
        let state = native_state(window, node.native);
        let style_source = window
            .dom()
            .attribute(node.native, zgui::view::AttrName::new("data-style-src"));
        let role = window
            .dom()
            .semantics(node.native)
            .map(|sem| format!("{:?}", sem.role));

        nodes.push(json!({
            "token": format!("node-{}", node.token),
            "parent": node.parent.map(|t| format!("node-{t}")),
            "kind": node.kind,
            "testId": node.test_id,
            "role": role,
            "text": text,
            "textTruncated": truncated,
            "redacted": redacted,
            "source": style_source,
            "state": {
                "enabled": state.map(|v| v.0),
                "visible": state.map(|v| v.1),
                "focused": focused == Some(node.native),
            },
            "boundsLogical": bounds_logical,
            "scrollMetrics": if coherent { scroll_metrics(window, node.native, scale) } else { None },
        }));
    }

    payload["metadataRevision"] = json!(owned.metadata_revision);
    payload["truncation"] = json!({
        "nodes": owned.truncated,
        "text": text_truncated,
        "maxNodes": MAX_DEV_NODES,
        "maxTextBytes": MAX_DEV_TEXT_BYTES,
    });
    payload["nodes"] = Value::Array(nodes);
    payload
}

/// Request queued from a dev inspector socket client to the UI thread.
pub(crate) struct InspectRequest {
    pub include_tree: bool,
    pub include_profile: bool,
    pub surface: Option<String>,
    reply: mpsc::SyncSender<String>,
}

impl InspectRequest {
    pub(crate) fn reply(self, payload: &Value) {
        let mut encoded = serde_json::to_string(payload)
            .unwrap_or_else(|_| "{\"error\":\"failed to serialize inspector snapshot\"}".into());
        if encoded.len() > MAX_RESPONSE_BYTES {
            encoded = json!({
                "error": "inspector snapshot exceeded response byte budget",
                "maxBytes": MAX_RESPONSE_BYTES,
            })
            .to_string();
        }
        let _ = self.reply.send(encoded);
    }
}

/// Host-owned, strictly read-only Unix socket server for `--dev-inspect`.
pub(crate) struct DevInspectorServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    rx: mpsc::Receiver<InspectRequest>,
    thread: Option<thread::JoinHandle<()>>,
}

impl DevInspectorServer {
    pub(crate) fn start(path: PathBuf, waker: Arc<dyn Waker>) -> std::io::Result<Self> {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let (tx, rx) = mpsc::sync_channel::<InspectRequest>(8);

        let handle = thread::Builder::new()
            .name("zgui-dev-inspector".into())
            .spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            handle_client(stream, &tx, &waker);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(15));
                        }
                        Err(_) => break,
                    }
                }
            })?;

        Ok(Self {
            path,
            stop,
            rx,
            thread: Some(handle),
        })
    }

    pub(crate) fn try_recv(&self) -> Option<InspectRequest> {
        self.rx.try_recv().ok()
    }

    pub(crate) fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for DevInspectorServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn handle_client(mut stream: UnixStream, tx: &mpsc::SyncSender<InspectRequest>, waker: &Arc<dyn Waker>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut line = String::new();
    let Ok(cloned) = stream.try_clone() else { return; };
    let mut reader = BufReader::new(cloned);
    let Ok(read_bytes) = reader.read_line(&mut line) else { return; };
    if read_bytes == 0 || read_bytes > MAX_REQUEST_LINE_BYTES {
        let _ = writeln!(stream, "{{\"error\":\"invalid or oversized inspector request\"}}");
        return;
    }
    let parsed: Result<Value, _> = serde_json::from_str(line.trim());
    let Ok(req) = parsed else {
        let _ = writeln!(stream, "{{\"error\":\"inspector request must be JSON\"}}");
        return;
    };
    let op = req.get("op").and_then(Value::as_str).unwrap_or("snapshot");
    let (include_tree, include_profile) = match op {
        "snapshot" => (true, true),
        "tree" => (true, false),
        "profile" => (false, true),
        _ => {
            let _ = writeln!(
                stream,
                "{{\"error\":\"read-only dev inspector: unsupported operation\"}}"
            );
            return;
        }
    };
    let surface = req
        .get("surface")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    if tx
        .try_send(InspectRequest {
            include_tree,
            include_profile,
            surface,
            reply: reply_tx,
        })
        .is_err()
    {
        let _ = writeln!(stream, "{{\"error\":\"inspector queue full\"}}");
        return;
    }
    waker.wake(WakeReason::AppWork);
    match reply_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(payload) => {
            let _ = writeln!(stream, "{payload}");
        }
        Err(_) => {
            let _ = writeln!(stream, "{{\"error\":\"inspector snapshot timed out\"}}");
        }
    }
}

/// CLI client helper for `native-host inspect --socket <path> [--op snapshot|tree|profile] [--surface <name>]`.
pub(crate) fn run_cli_inspect(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut socket: Option<PathBuf> = None;
    let mut op = "snapshot".to_owned();
    let mut surface: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--socket" => {
                i += 1;
                socket = Some(PathBuf::from(
                    args.get(i).ok_or("--socket requires a path")?,
                ));
            }
            "--op" => {
                i += 1;
                op = args.get(i).ok_or("--op requires snapshot, tree, or profile")?.clone();
            }
            "--surface" => {
                i += 1;
                surface = Some(args.get(i).ok_or("--surface requires a surface name")?.clone());
            }
            other => return Err(format!("unknown inspect option: {other}").into()),
        }
        i += 1;
    }
    let path = socket.ok_or("inspect requires --socket <path>")?;
    let response = query_socket(&path, &op, surface.as_deref())?;
    println!("{response}");
    Ok(())
}

pub(crate) fn query_socket(
    path: &Path,
    op: &str,
    surface: Option<&str>,
) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut req = json!({ "op": op });
    if let Some(surface) = surface {
        req["surface"] = json!(surface);
    }
    writeln!(stream, "{req}")?;
    stream.flush()?;
    let mut response = String::new();
    let mut reader = BufReader::new(stream);
    reader.read_line(&mut response)?;
    Ok(response.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        session::{SessionModule, SessionPolicy},
        session_host::{ImageChoice, RecoveryPolicy, Root, RuntimeSurface, SessionHost, SurfaceOpacity},
    };
    use zgui::{
        platform::{AppHandler, PlatformCapabilities, PlatformCx, SurfaceEvent},
        view::AttrName,
    };
    use zgui_platform_headless::Headless;

    #[test]
    fn profiler_ring_buffer_is_strictly_bounded() {
        let inspector = DevInspector::new();
        for i in 0..400u64 {
            inspector.record_checkpoint(1, i as usize, None);
            inspector.ring.borrow_mut().push_frame(FrameSample {
                surface_id: 1,
                ordinal: i + 1,
                outcome: "Presented".into(),
                document_revision: i,
                style_revision: 1,
                layout_revision: Some(i),
                coherent: true,
                style_us: 10,
                layout_us: 20,
                paint_us: 30,
                total_us: 60,
            });
        }
        let snap = inspector.profiler_snapshot();
        assert_eq!(snap["frameCount"], MAX_RING_SAMPLES);
        assert_eq!(snap["checkpointCount"], MAX_RING_SAMPLES);
        assert_eq!(snap["frames"][0]["ordinal"], 400 - MAX_RING_SAMPLES as u64 + 1);
        assert_eq!(snap["checkpoints"][0]["queueDepth"], 400 - MAX_RING_SAMPLES);
    }

    #[test]
    fn inspector_redacts_secure_fields_labels_incoherent_frames_and_excludes_locked_sessions() {
        let source = r#"
            export const hostApiVersion = 1;
            export function mount(root) {
                const pubLabel = __zgui.createElement("label");
                __zgui.insertNode(pubLabel, __zgui.createTextNode("Public Greeting"), null);
                __zgui.setProperty(pubLabel, "testId", "public-text");
                __zgui.insertNode(root, pubLabel, null);

                const secretField = __zgui.createElement("field");
                __zgui.setProperty(secretField, "secure", true);
                __zgui.setProperty(secretField, "testId", "secret-input");
                __zgui.setProperty(secretField, "value", "SuperSecretPassword123!");
                __zgui.insertNode(root, secretField, null);

                return () => {};
            }
        "#;
        let inspector = DevInspector::new();
        let pending = Rc::new(RefCell::new(Vec::new()));
        let roots = pending.clone();
        let app = zgui::app()
            .with_title("inspector-test")
            .with_size(320.0, 200.0)
            .with_renderer(Box::new(|_, target| {
                let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
                zgui::render::Renderer::configure(&mut renderer, target);
                Ok(Box::new(renderer))
            }))
            .with_probe(inspector.probe());
        let mut runtime = app
            .into_handler(move || Root(roots.clone()))
            .expect("handler")
            .into_runtime();
        let platform = Headless::new().with_capabilities(PlatformCapabilities::none());
        runtime.surfaces_available(&platform);

        let policy = SessionPolicy::live(host_services::Executor::new(1, 16).expect("exec"));
        let mut host = SessionHost::new(
            pending,
            SessionModule::inline("inspector-test", source, vec![]),
            policy,
            RecoveryPolicy::Propagate,
            SurfaceOpacity::Opaque,
        );
        let retry: Rc<dyn Fn(zgui::view::DocumentId, u64)> = Rc::new(|_, _| {});
        {
            let mut surface = RuntimeSurface::new(&mut runtime, &platform, ImageChoice::Live);
            host.attach(&mut surface, platform.waker(), retry).expect("attach");
        }
        // Pump one frame so CompletedFrame is populated cleanly.
        let win_surface = runtime.windows()[0].surface().id();
        runtime.surface_event(&platform, win_surface, SurfaceEvent::RedrawRequested);
        platform.virtual_clock().advance(Duration::from_millis(20));
        runtime.surface_event(&platform, win_surface, SurfaceEvent::RedrawRequested);

        let session = host.sessions().next().expect("active session").clone();
        let snap = inspector.capture_snapshot(&runtime, Some(&session), true, true, None);
        let serialized = serde_json::to_string(&snap).expect("serialize");

        // 1. Public text is visible, raw password is NEVER leaked.
        assert!(serialized.contains("Public Greeting"), "expected public text in snapshot: {serialized}");
        assert!(
            !serialized.contains("SuperSecretPassword123!"),
            "secure password leaked into dev inspector snapshot: {serialized}"
        );
        let nodes = snap["surfaces"][0]["nodes"].as_array().expect("nodes array");
        let secret_node = nodes
            .iter()
            .find(|n| n["testId"] == "secret-input")
            .expect("secret-input node present");
        assert_eq!(secret_node["redacted"], true);
        assert_ne!(secret_node["text"], "SuperSecretPassword123!");

        // 2. Frame profiler recorded phase timings cleanly.
        assert!(snap["profiler"]["frameCount"].as_u64().unwrap_or(0) >= 1);

        // 3. Mutate DOM after completed frame -> snapshot explicitly labels historical/incoherent
        //    and refuses to join live tree nodes to stale layout boxes.
        let doc_root = session.primary().expect("primary").region.root;
        session
            .primary()
            .expect("primary")
            .region
            .dom
            .set_attribute(doc_root, AttrName::new("data-mutated"), Some("1"));
        let incoherent = inspector.capture_snapshot(&runtime, Some(&session), true, false, None);
        assert_eq!(incoherent["surfaces"][0]["coherent"], false);
        assert_eq!(incoherent["surfaces"][0]["coherenceStatus"], "historical-incoherent");
        for node in incoherent["surfaces"][0]["nodes"].as_array().unwrap() {
            assert!(node["boundsLogical"].is_null(), "incoherent snapshot must not join stale bounds");
        }

        // 4. Locking the session forbids capturing any surface content.
        let _ = session.update_lock_state(zgui::platform::SessionLockState::Locked);
        let locked = inspector.capture_snapshot(&runtime, Some(&session), true, false, None);
        assert_eq!(locked["sessionLocked"], true);
        assert_eq!(locked["surfaces"][0]["excluded"], true);
        assert_eq!(locked["surfaces"][0]["reason"], "session-lock-surface-forbidden");
        assert!(locked["surfaces"][0]["nodes"].as_array().unwrap().is_empty());
        assert!(!serde_json::to_string(&locked).unwrap().contains("Public Greeting"));

        let mut surface = RuntimeSurface::new(&mut runtime, &platform, ImageChoice::Live);
        host.stop(&mut surface);
    }

    #[test]
    fn readonly_socket_server_serves_snapshots_and_rejects_write_ops() {
        let dir = std::env::temp_dir();
        let sock = dir.join(format!("zgui-dev-inspect-test-{}.sock", std::process::id()));
        let platform = Headless::new();
        let mut server = DevInspectorServer::start(sock.clone(), platform.waker()).expect("start server");

        let sock_for_client = sock.clone();
        let client = thread::spawn(move || {
            let bad = query_socket(&sock_for_client, "eval", None).expect("eval reply");
            let ok = query_socket(&sock_for_client, "profile", None).expect("profile reply");
            (bad, ok)
        });

        // Wait briefly for the valid request to arrive on the host side and answer it.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut answered = false;
        while std::time::Instant::now() < deadline {
            if let Some(req) = server.try_recv() {
                assert!(!req.include_tree);
                assert!(req.include_profile);
                req.reply(&json!({ "readOnly": true, "profiler": { "frameCount": 0 } }));
                answered = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(answered, "expected server to receive valid profile request");
        let (bad, ok) = client.join().expect("client finished");
        assert!(bad.contains("unsupported operation"), "write/eval op must be rejected: {bad}");
        assert!(ok.contains("\"readOnly\":true"), "valid read-only op must return JSON: {ok}");
        server.stop();
        assert!(!sock.exists(), "socket file cleaned up on stop");
    }
}
