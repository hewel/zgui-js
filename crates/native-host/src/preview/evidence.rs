//! Typed capture evidence: one identity and validation contract for native
//! tree/frame pairs, immutable captures and retained failure artifacts.
use std::{collections::HashMap,path::Path};

use serde::{Deserialize,Serialize};
use serde_json::{Map,Value,json};

use super::{Error,output::{self,Output},scenario};

const FAILURE_TREE: &str = "failure/tree.json";
const FAILURE_FRAME: &str = "failure/frame.json";
const FAILURE_PIXELS: &str = "failure/last-valid.png";
const EVIDENCE_DIR: &str = "evidence";
const CAPTURES_DIR: &str = "captures";
const NAMED_CAPTURES_DIR: &str = "captures/surfaces";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct FrameStamp {
    pub(super) ordinal: u64,
    pub(super) time_us: u64,
    pub(super) draw_id: u64,
    pub(super) outcome: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct Presentation {
    pub(super) frame: FrameStamp,
    pub(super) sequence: u64,
    pub(super) configuration_id: u64,
    /// The presented draw; serialized as null before the first presentation.
    pub(super) draw_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) style_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) document_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) metadata_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) resource_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) generation: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct NodeState {
    pub(super) enabled: Option<bool>,
    pub(super) visible: Option<bool>,
    pub(super) focused: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct Bounds {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct NodeEvidence {
    pub(super) token: String,
    #[serde(default)]
    pub(super) parent: Option<String>,
    pub(super) surface: String,
    pub(super) generation: u64,
    pub(super) kind: String,
    pub(super) ownership: String,
    pub(super) text: String,
    #[serde(default)]
    pub(super) text_truncated: bool,
    #[serde(default)]
    pub(super) test_id: Option<String>,
    #[serde(default)]
    pub(super) test_id_truncated: bool,
    #[serde(default)]
    pub(super) role: Option<String>,
    #[serde(default)]
    pub(super) name: Option<String>,
    /// Explicit compiler metadata only; absent source positions are never synthesized.
    #[serde(default)]
    pub(super) source: Value,
    #[serde(default)]
    pub(super) value: Option<String>,
    #[serde(default)]
    pub(super) name_truncated: bool,
    #[serde(default)]
    pub(super) value_truncated: bool,
    pub(super) state: NodeState,
    #[serde(default)]
    pub(super) bounds_logical: Option<Bounds>,
    #[serde(default)]
    pub(super) bounds_device: Option<Bounds>,
    #[serde(default)]
    pub(super) bounds_kind: String,
    #[serde(default)]
    pub(super) pointer_reachability_verified: bool,
    #[serde(flatten)]
    pub(super) extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct TreeEvidence {
    pub(super) schema_version: u32,
    pub(super) build_id: String,
    pub(super) generation: u64,
    pub(super) surface: String,
    pub(super) frame_ordinal: u64,
    pub(super) metadata_revision: u64,
    pub(super) presentation_sequence: u64,
    #[serde(default, deserialize_with = "optional_revision", skip_serializing_if = "Option::is_none")]
    pub(super) style_revision: Option<u64>,
    #[serde(default, deserialize_with = "optional_revision", skip_serializing_if = "Option::is_none")]
    pub(super) resource_revision: Option<u64>,
    #[serde(default, deserialize_with = "optional_revision", skip_serializing_if = "Option::is_none")]
    pub(super) surface_configuration: Option<u64>,
    pub(super) nodes: Vec<NodeEvidence>,
    #[serde(flatten)]
    pub(super) extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(super) struct FrameEvidence {
    pub(super) schema_version: u32,
    pub(super) build_id: String,
    pub(super) generation: u64,
    pub(super) surface: String,
    pub(super) metadata_revision: u64,
    #[serde(default, deserialize_with = "optional_revision", skip_serializing_if = "Option::is_none")]
    pub(super) style_revision: Option<u64>,
    #[serde(default, deserialize_with = "optional_revision", skip_serializing_if = "Option::is_none")]
    pub(super) resource_revision: Option<u64>,
    #[serde(default, deserialize_with = "optional_revision", skip_serializing_if = "Option::is_none")]
    pub(super) surface_configuration: Option<u64>,
    pub(super) validation_frame: FrameStamp,
    pub(super) pixel_presentation: Presentation,
    pub(super) layout_document_coherent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) capture_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tree_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) png_digest: Option<String>,
    #[serde(flatten)]
    pub(super) extra: Map<String, Value>,
}

/// A present-but-null or mistyped revision is a malformed identity, never an absent one.
fn optional_revision<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(deserializer).map(Some)
}

/// Native identity the owning frame stamped into its snapshot; inspect builds the
/// typed tree directly instead of mutating serialized fields afterwards.
pub(super) struct CaptureIdentity {
    pub(super) build_id: String,
    pub(super) presentation_sequence: u64,
    pub(super) resource_revision: Option<u64>,
    pub(super) surface_configuration: Option<u64>,
    pub(super) style_revision: Option<u64>,
}

/// Bounded, validated capture identifier owning its artifact paths.
#[derive(Clone)]
pub(super) struct CapturePaths {
    id: String,
    surface: String,
}
impl CapturePaths {
    pub(super) fn new(id: &str, surface: &str) -> Result<Self, Error> {
        if !scenario::safe_id(id) || !scenario::safe_id(surface) {
            return Err(Error::new("E_SCHEMA", 2, "invalid capture ID or surface name"));
        }
        Ok(Self { id: id.to_owned(), surface: surface.to_owned() })
    }
    pub(super) fn id(&self) -> &str { &self.id }
    pub(super) fn surface(&self) -> &str { &self.surface }
    fn stem(&self) -> String {
        if self.surface == "main" { format!("{CAPTURES_DIR}/{}", self.id) }
        else { format!("{NAMED_CAPTURES_DIR}/{}/{}", self.surface, self.id) }
    }
    pub(super) fn png(&self) -> String { format!("{}.png", self.stem()) }
    pub(super) fn tree(&self) -> String { format!("{}.tree.json", self.stem()) }
    pub(super) fn frame(&self) -> String { format!("{}.frame.json", self.stem()) }
    pub(super) fn from_artifact(path: &str, suffix: &str) -> Result<Self, Error> {
        let stem = path.strip_suffix(suffix).ok_or_else(|| io("invalid capture artifact path"))?;
        let identity = if let Some(stem) = stem.strip_prefix("captures/surfaces/") {
            stem.split_once('/').filter(|(surface, _)| *surface != "main")
        } else {
            stem.strip_prefix("captures/").map(|id| ("main", id))
        };
        let (surface, id) = identity.ok_or_else(|| io("invalid capture artifact path"))?;
        Self::new(id, surface).map_err(|_| io("invalid capture artifact identity"))
    }
    pub(super) fn summary(&self) -> Value {
        let mut summary = json!({"captureId":self.id,"png":self.png(),"frame":self.frame(),"tree":self.tree()});
        if self.surface != "main" { summary["surface"] = json!(self.surface); }
        summary
    }
}

/// Evidence stems are monotonic identities, never caller-chosen names.
pub(super) fn checkpoint_paths(generation: u64, ordinal: u64, named_sequence: Option<u64>) -> (String, String) {
    let stem = if let Some(sequence) = named_sequence {
        format!("{EVIDENCE_DIR}/generation-{generation:010}-checkpoint-{sequence:010}")
    } else {
        format!("{EVIDENCE_DIR}/generation-{generation:010}-frame-{ordinal:010}")
    };
    (format!("{stem}.tree.json"), format!("{stem}.frame.json"))
}

impl TreeEvidence {
    /// Pair agreement between this tree and its owning frame: schema, build,
    /// generation, surface, ordinals, metadata and revision identities.
    pub(super) fn validate_frame(&self, frame: &FrameEvidence) -> Result<(), Error> {
        if self.schema_version != 1 || frame.schema_version != 1 || self.build_id.is_empty()
            || self.build_id != frame.build_id || self.generation == 0 || self.generation != frame.generation
            || !scenario::safe_id(&self.surface) || self.surface != frame.surface
            || self.frame_ordinal != frame.validation_frame.ordinal
            || self.presentation_sequence != frame.pixel_presentation.sequence
            || self.metadata_revision != frame.metadata_revision
            || self.detail("checkpointSequence") != frame.detail("checkpointSequence")
            || (self.surface != "main" && self.detail("checkpointSequence").as_u64().is_none_or(|sequence| sequence == 0)) {
            return Err(Error::new("E_OUTPUT_IO", 8, "capture, tree and terminal run identities disagree"));
        }
        let native_resources = frame.extra.get("resources").and_then(|resources| resources.get("requiredAssetsReady")).is_some();
        for (tree_revision, frame_revision) in
            [(self.resource_revision, frame.resource_revision), (self.surface_configuration, frame.surface_configuration)] {
            if (native_resources || tree_revision.is_some() || frame_revision.is_some())
                && (tree_revision.is_none() || tree_revision != frame_revision) {
                return Err(Error::new("E_OUTPUT_IO", 8, "capture resource/surface configuration identities disagree"));
            }
        }
        let tree_style = self.style_revision;
        let frame_style = frame.style_revision;
        if (tree_style.is_some() || frame_style.is_some())
            && (tree_style.is_none() || tree_style != frame_style) {
            return Err(Error::new("E_OUTPUT_IO", 8, "capture style identities disagree"));
        }
        if let Some(revision) = frame.resource_revision
            && frame.extra.get("resources").and_then(|resources| resources.get("resourceRevision")).and_then(Value::as_u64) != Some(revision) {
            return Err(Error::new("E_OUTPUT_IO", 8, "capture readiness belongs to another resource revision"));
        }
        Ok(())
    }

    /// Optional detail payload owned by the producing module; a missing key is
    /// an absent observation, not a panic.
    pub(super) fn detail(&self, name: &str) -> &Value {
        self.extra.get(name).unwrap_or(&Value::Null)
    }

    /// Parent indices after a complete parent-first topology check. Node
    /// surface and generation must agree with the owning tree.
    pub(super) fn validate_topology(&self) -> Result<Vec<Option<usize>>, Error> {
        if self.nodes.is_empty() || self.nodes.len() > 4096 {
            return Err(Error::new("E_OUTPUT_LIMIT", 8, "saved tree exceeds the bounded native snapshot contract"));
        }
        let mut by_token = HashMap::with_capacity(self.nodes.len());
        let mut parents = Vec::with_capacity(self.nodes.len());
        for (index, node) in self.nodes.iter().enumerate() {
            if node.token.len() > 64 || by_token.contains_key(node.token.as_str()) {
                return Err(Error::new("E_OUTPUT_IO", 8, "saved tree has invalid or duplicate tokens"));
            }
            if node.surface != self.surface || node.generation != self.generation {
                return Err(Error::new("E_OUTPUT_IO", 8, "saved node identity disagrees with its tree"));
            }
            let parent = match &node.parent {
                Some(token) => Some(*by_token.get(token.as_str()).ok_or_else(|| Error::new("E_OUTPUT_IO", 8, "saved tree is not parent-first"))?),
                None => None,
            };
            by_token.insert(node.token.as_str(), index);
            parents.push(parent);
        }
        Ok(parents)
    }
}

impl FrameEvidence {
    /// Immutable capture agreement: the terminal result names the same build at
    /// a generation not before this capture, and the recorded digests are exact.
    pub(super) fn validate_capture(&self, tree: &TreeEvidence, capture: &CapturePaths, tree_digest: &str, result: &Value) -> Result<(), Error> {
        if result["schemaVersion"] != 1 || result["command"] != "run"
            || self.capture_id.as_deref() != Some(capture.id()) || self.tree_digest.as_deref() != Some(tree_digest)
            || self.surface != capture.surface()
            || !self.layout_document_coherent || result["build"]["buildId"].as_str() != Some(tree.build_id.as_str())
            || result["generation"].as_u64().is_none_or(|current| tree.generation > current) {
            return Err(Error::new("E_OUTPUT_IO", 8, "capture, tree and terminal run identities disagree"));
        }
        tree.validate_topology()?;
        tree.validate_frame(self)
    }

    /// Optional detail payload owned by the producing module; a missing key is
    /// an absent observation, not a panic.
    pub(super) fn detail(&self, name: &str) -> &Value {
        self.extra.get(name).unwrap_or(&Value::Null)
    }

    /// Readiness certification issues for an offline comparison. Core stamps are
    /// typed; the opaque readiness/resource payloads stay untouched in `extra`.
    pub(super) fn readiness_issues(&self, side: &str, issues: &mut Vec<Value>) {
        let readiness = self.detail("readiness");
        for field in ["appReady", "currentWorkSettled", "frameCurrent", "nativeAssetReadinessAvailable"] {
            if readiness[field] != true {
                issues.push(json!({"side":side,"field":format!("readiness.{field}"),"reason":"capture is not certified current"}));
            }
        }
        let resources = self.detail("resources");
        let pending = resources["pendingIds"].as_array();
        let allowed = self.detail("allowedPendingAssets").as_array();
        let ready = match (pending, allowed) {
            (Some(pending), Some(allowed)) => {
                let pending_ids: std::collections::HashSet<_> = pending.iter().filter_map(Value::as_str).collect();
                let allowed_ids: std::collections::HashSet<_> = allowed.iter().filter_map(Value::as_str).collect();
                pending_ids.len() == pending.len() && allowed_ids.len() == allowed.len() && pending_ids == allowed_ids
                    && resources["allowedPendingAssets"] == *self.detail("allowedPendingAssets")
                    && resources["requiredAssetsReady"] == true
            }
            _ => false,
        };
        if !ready || self.detail("timingDeterministic") != true || self.detail("requiredStateRevision").as_u64().is_none()
            || self.detail("requiredStateRevision") != self.detail("validatedStateRevision")
            || readiness["timeUs"].as_u64().is_none()
            || self.detail("scale").as_f64().is_none_or(|scale| !scale.is_finite() || scale <= 0.0) {
            issues.push(json!({"side":side,"field":"readiness","reason":"capture timing, resource allowance or validated state is incomplete"}));
        }
    }
}

/// How fresh the retained evidence is relative to the run's terminal failure.
pub(super) struct HistoricalStatus {
    pub(super) satisfies_failure_revision: Option<bool>,
    pub(super) readiness: Option<Value>,
    /// Live first publication uses "failure-tree"; recovered completion uses "historical-tree".
    pub(super) tree_role: &'static str,
}

/// The single historical publication for live and recovered evidence. The whole
/// pair and every retained artifact are validated before any mutation; existing
/// bytes are kept, and a conflicting partial pair is an error.
pub(super) fn publish_historical(output: &mut Output, tree: &TreeEvidence, frame: &FrameEvidence,
    pixels: Option<&[u8]>, status: HistoricalStatus) -> Result<Value, Error> {
    tree.validate_frame(frame)?;
    tree.validate_topology()?;
    let tree_path = output.root().join(FAILURE_TREE);
    let frame_path = output.root().join(FAILURE_FRAME);
    let pixels_path = output.root().join(FAILURE_PIXELS);
    let existing_tree = tree_path.is_file().then(|| read_evidence(&tree_path)).transpose()?;
    let existing_frame = frame_path.is_file().then(|| read_evidence(&frame_path)).transpose()?;
    let existing_pixels = match std::fs::symlink_metadata(&pixels_path) {
        Ok(_) => Some(
            crate::bounded_fs::read_input(&pixels_path, 64 * 1024 * 1024)
                .map_err(|error| output::bounded_read_error(error, "historical pixels exceed 64 MiB"))?,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(io(error)),
    };
    if let Some(bytes) = &existing_tree {
        let existing: TreeEvidence = serde_json::from_slice(bytes).map_err(io)?;
        if &existing != tree {
            return Err(Error::new("E_OUTPUT_IO", 8, "partial failure tree belongs to another run generation"));
        }
        existing.validate_frame(frame)?;
    }
    if let Some(bytes) = &existing_frame {
        let existing: FrameEvidence = serde_json::from_slice(bytes).map_err(io)?;
        tree.validate_frame(&existing)?;
        // Complete identity agreement is required; the historical freshness and
        // readiness flags stamped at publication deliberately differ.
        if existing.schema_version != frame.schema_version || existing.build_id != frame.build_id
            || existing.generation != frame.generation || existing.surface != frame.surface
            || existing.metadata_revision != frame.metadata_revision
            || existing.resource_revision != frame.resource_revision
            || existing.surface_configuration != frame.surface_configuration
            || existing.validation_frame != frame.validation_frame
            || existing.pixel_presentation != frame.pixel_presentation
            || existing.layout_document_coherent != frame.layout_document_coherent
            || existing.detail("configurationId") != frame.detail("configurationId")
            || existing.detail("documentIdentity") != frame.detail("documentIdentity") {
            return Err(Error::new("E_OUTPUT_IO", 8, "partial failure frame belongs to another run generation"));
        }
    }
    if let (Some(have), Some(new)) = (&existing_pixels, pixels)
        && output::digest(have) != output::digest(new) {
        return Err(Error::new("E_OUTPUT_IO", 8, "historical pixel identity mismatch"));
    }
    let mut frame = frame.clone();
    frame.extra.insert("historical".into(), json!(true));
    frame.extra.insert("satisfiesFailureRevision".into(), status.satisfies_failure_revision.map_or(Value::Null, Value::from));
    if let Some(readiness) = status.readiness { frame.extra.insert("failureReadiness".into(), readiness); }
    let pixels_available = existing_pixels.is_some() || pixels.is_some();
    frame.extra.insert("pixelsAvailable".into(), json!(pixels_available));
    if existing_tree.is_none() {
        output.json(FAILURE_TREE, status.tree_role, &serde_json::to_value(tree).map_err(io)?)?;
    }
    if let Some(bytes) = pixels && existing_pixels.is_none() {
        output.bytes(FAILURE_PIXELS, "historical-pixels", bytes)?;
    }
    if existing_frame.is_none() {
        output.json(FAILURE_FRAME, "historical-frame", &serde_json::to_value(&frame).map_err(io)?)?;
    }
    Ok(json!({"available":true,"historical":true,"tree":FAILURE_TREE,"frame":FAILURE_FRAME,
        "pixels":pixels_available.then_some(FAILURE_PIXELS)}))
}

/// Called only after the owned worker has stopped. It never loads the application:
/// the retained pair is selected, validated and completed without any readback.
pub(super) fn recover(output: &mut Output) -> Result<Value, Error> {
    let directory = output.root().join(EVIDENCE_DIR);
    let tree_path = output.root().join(FAILURE_TREE);
    let frame_path = output.root().join(FAILURE_FRAME);
    let existing_tree = tree_path.is_file().then(|| read_typed::<TreeEvidence>(&tree_path)).transpose()?;
    let (frame, tree) = if frame_path.is_file() {
        let frame: FrameEvidence = read_typed(&frame_path)?;
        let tree = match existing_tree {
            Some(tree) => tree,
            None => read_typed(&frame_path.with_file_name(frame_path.file_name().expect("frame file").to_string_lossy().replace(".frame.json", ".tree.json")))?,
        };
        (frame, tree)
    } else if let Some(tree) = existing_tree {
        let named = if tree.surface != "main" {
            Some(tree.detail("checkpointSequence").as_u64().ok_or_else(|| io("named evidence has no checkpoint sequence"))?)
        } else { None };
        let (_, frame) = checkpoint_paths(tree.generation, tree.frame_ordinal, named);
        (read_typed(&output.root().join(frame))?, tree)
    } else {
        let mut frames = match std::fs::read_dir(&directory) {
            Ok(entries) => entries.collect::<Result<Vec<_>, _>>().map_err(io)?
                .into_iter().map(|entry| entry.path())
                .filter(|path| path.file_name().and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("generation-") && name.ends_with(".frame.json"))).collect::<Vec<_>>(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(json!({"available":false})),
            Err(error) => return Err(io(error)),
        };
        frames.sort();
        // The newest checkpoint wins. An empty post-fault document is not a
        // complete checkpoint, so an earlier pair may still be the last
        // publishable evidence; any other invalid or corrupt pair errors.
        let (mut primary, mut named) = (None, None);
        while let Some(path) = frames.pop() {
            let named_path = path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.contains("-checkpoint-"));
            let destination = if named_path { &mut named } else { &mut primary };
            if destination.is_some() { continue; }
            let frame: FrameEvidence = read_typed(&path)?;
            let file = path.file_name().and_then(|name| name.to_str()).expect("frame file name checked");
            let tree: TreeEvidence = read_typed(&path.with_file_name(
                format!("{}.tree.json", file.strip_suffix(".frame.json").expect("frame suffix checked"))))?;
            tree.validate_frame(&frame)?;
            if tree.nodes.is_empty() { continue; }
            tree.validate_topology()?;
            *destination = Some((frame, tree));
            if primary.is_some() && named.is_some() { break; }
        }
        let selected = primary.into_iter().chain(named).max_by_key(|(frame, _)| {
            (frame.generation, frame.detail("checkpointSequence").as_u64().unwrap_or(frame.validation_frame.ordinal))
        });
        let Some(selected) = selected else { return Ok(json!({"available":false})); };
        selected
    };
    tree.validate_frame(&frame).map_err(|_| Error::new("E_OUTPUT_IO", 8, "recovered frame and tree identities disagree"))?;
    let mut pixels = None::<Vec<u8>>;
    if !output.root().join(FAILURE_PIXELS).is_file() && !frame_path.is_file() {
        let directory = if frame.surface == "main" { output.root().join(CAPTURES_DIR) }
            else { output.root().join(NAMED_CAPTURES_DIR).join(&frame.surface) };
        match std::fs::read_dir(directory) {
            Ok(entries) => for entry in entries {
                let path = entry.map_err(io)?.path();
                if !path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.ends_with(".frame.json")) { continue; }
                let candidate: Value = serde_json::from_slice(&read_evidence(&path)?).map_err(io)?;
                if candidate["generation"] != frame.generation || candidate["buildId"] != frame.build_id
                    || candidate["surface"] != frame.surface || candidate["documentIdentity"] != *frame.detail("documentIdentity")
                    || candidate["pixelPresentation"] != serde_json::to_value(&frame.pixel_presentation).map_err(io)? { continue; }
                // The loose identity match named this candidate; its typed parse must now succeed.
                let capture: FrameEvidence = serde_json::from_value(candidate).map_err(io)?;
                let file = path.file_name().and_then(|name| name.to_str()).expect("capture file name checked");
                let png = path.with_file_name(format!("{}.png", file.strip_suffix(".frame.json").expect("capture suffix checked")));
                let bytes = crate::bounded_fs::read_input(&png, 64 * 1024 * 1024)
                    .map_err(|error| output::bounded_read_error(error, "captured pixels exceed 64 MiB"))?;
                if capture.png_digest.as_deref() != Some(output::digest(&bytes).as_str()) {
                    return Err(Error::new("E_OUTPUT_IO", 8, "historical pixel identity mismatch"));
                }
                pixels = Some(bytes);
                break;
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io(error)),
        }
    }
    publish_historical(output, &tree, &frame, pixels.as_deref(),
        HistoricalStatus { satisfies_failure_revision: None, readiness: None, tree_role: "historical-tree" })
}

/// Bounded evidence read shared by recovery and the controller's cause lookup.
pub(super) fn read_evidence(path: &Path) -> Result<Vec<u8>, Error> {
    crate::bounded_fs::read_input(path, 8 * 1024 * 1024)
        .map_err(|error| output::bounded_read_error(error, "recovered evidence exceeds 8 MiB"))
}
fn read_typed<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Error> {
    serde_json::from_slice(&read_evidence(path)?).map_err(io)
}
fn io(error: impl std::fmt::Display) -> Error { Error::new("E_OUTPUT_IO", 8, error.to_string()) }

#[cfg(test)]
mod tests;
