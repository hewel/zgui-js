//! Opaque node identity, synchronous topology, and turn-boundary reclamation.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use zgui::view::{DocumentId, DomHandle, ElementName, NodeId, OverlayLayer};

/// A borrowed reference to a node in one session generation and native document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Handle {
    session: u64,
    generation: u64,
    document: DocumentId,
    token: u64,
}

impl Handle {
    /// The bridge-local token; this is not a native node identifier.
    pub fn token(self) -> u64 {
        self.token
    }
}

/// An owned bridge-tree record for Rust-side inspection, not a JavaScript value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotNode {
    pub token: u64,
    pub parent: Option<u64>,
    pub native: NodeId,
    pub kind: &'static str,
    pub test_id: Option<String>,
}

/// A bounded, stable preorder projection of the attached live bridge tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub nodes: Vec<SnapshotNode>,
    pub truncated: bool,
    pub metadata_revision: u64,
}

/// A rejected bridge operation. Validation happens before native mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BridgeError {
    WrongSession,
    WrongGeneration,
    WrongDocument,
    StaleHandle,
    Disposed,
    TurnNotActive,
    UnsupportedTag(String),
    NodeLimit,
    RootBoundary,
    NotText,
    TextParent,
    EditableParent,
    NotEditable,
    NotField,
    NotImage,
    ImageParent,
    NotVector,
    VectorParent,
    WrongParent,
    InvalidAnchor,
    Cycle,
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongSession => f.write_str("node belongs to another session"),
            Self::WrongGeneration => f.write_str("node belongs to another session generation"),
            Self::WrongDocument => f.write_str("node belongs to another native document"),
            Self::StaleHandle => f.write_str("native node handle is stale"),
            Self::Disposed => f.write_str("node registry has been disposed"),
            Self::TurnNotActive => f.write_str("native mutation requires an active structural turn"),
            Self::UnsupportedTag(tag) => write!(f, "unsupported native element: {tag}"),
            Self::NodeLimit => f.write_str("native node allocation limit reached"),
            Self::RootBoundary => f.write_str("the host-owned root cannot be detached or reparented"),
            Self::NotText => f.write_str("node is not a text node"),
            Self::TextParent => f.write_str("text nodes cannot contain children"),
            Self::EditableParent => f.write_str("editable controls cannot contain reconciled children"),
            Self::NotEditable => f.write_str("value, defaultValue, and readOnly require a field or editor"),
            Self::NotField => f.write_str("secure requires a field"),
            Self::NotImage => f.write_str("src requires an image element"),
            Self::ImageParent => f.write_str("image elements cannot contain reconciled children"),
            Self::NotVector => f.write_str("d and viewBox require a vector element"),
            Self::VectorParent => f.write_str("vector elements cannot contain reconciled children"),
            Self::WrongParent => f.write_str("node is not a child of the supplied parent"),
            Self::InvalidAnchor => f.write_str("insertion anchor is not a child of the supplied parent"),
            Self::Cycle => f.write_str("insertion would create a topology cycle"),
        }
    }
}

impl std::error::Error for BridgeError {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Root,
    Element,
    Field,
    Editor,
    Text,
    Image,
    Vector,
    Portal,
}

struct PortalRecord {
    marker: NodeId,
    layer: OverlayLayer,
}

struct Record {
    native: NodeId,
    kind: Kind,
    parent: Option<u64>,
    first: Option<u64>,
    last: Option<u64>,
    previous: Option<u64>,
    next: Option<u64>,
    pending_detach: bool,
    test_id: Option<String>,
}

impl Record {
    fn new(native: NodeId, kind: Kind) -> Self {
        Self {
            native,
            kind,
            parent: None,
            first: None,
            last: None,
            previous: None,
            next: None,
            pending_detach: false,
            test_id: None,
        }
    }
}

/// Owns JS-created nodes, but only borrows the host root.
///
/// Sibling links make traversal and splicing O(1); cycle checks walk the ancestor
/// chain. A removed tree is usable only until `finish_turn`. Never-linked trees
/// survive successful turns and are reclaimed on disposal. Native frame ending
/// remains the host's responsibility.
pub struct NodeRegistry {
    dom: DomHandle,
    root: Handle,
    records: HashMap<u64, Record>,
    native_tokens: HashMap<NodeId, u64>,
    next_token: u64,
    max_nodes: usize,
    pending: Vec<u64>,
    turn_active: bool,
    disposed: bool,
    metadata_revision: u64,
    portals: BTreeMap<u64, PortalRecord>,
    portal_parking: Option<NodeId>,
}

impl NodeRegistry {
    /// Borrows a live host-owned root. Session generations must have unique IDs.
    pub fn new(
        dom: DomHandle,
        root: NodeId,
        session: u64,
        generation: u64,
        max_nodes: usize,
    ) -> Self {
        assert!(dom.is_live(root), "registry root must be live");
        let handle = Handle {
            session,
            generation,
            document: root.document(),
            token: 0,
        };
        let mut records = HashMap::new();
        records.insert(0, Record::new(root, Kind::Root));
        Self {
            dom,
            root: handle,
            records,
            native_tokens: HashMap::from([(root, 0)]),
            next_token: 1,
            max_nodes,
            pending: Vec::new(),
            turn_active: false,
            disposed: false,
            metadata_revision: 0,
            portals: BTreeMap::new(),
            portal_parking: None,
        }
    }

    /// Returns the boundary handle, which never exposes its native parent/siblings.
    pub fn root(&self) -> Handle {
        self.root
    }

    pub(crate) fn node_limit(&self) -> usize {
        self.max_nodes
    }

    /// Copies at most `max_nodes` attached live records, including root token 0.
    /// Detached and never-attached subtrees are excluded. A disposed registry or
    /// dead root produces an empty snapshot; `truncated` means a live node was
    /// omitted by the limit. Native IDs remain valid only while their nodes live.
    /// This does not mutate the tree or enter JavaScript.
    pub fn snapshot(&self, max_nodes: usize) -> Snapshot {
        let mut snapshot = Snapshot { nodes: Vec::new(), truncated: false, metadata_revision: self.metadata_revision };
        if self.disposed {
            return snapshot;
        }
        let mut current = Some(0);
        while let Some(token) = current {
            let record = &self.records[&token];
            if self.dom.is_live(record.native) {
                if snapshot.nodes.len() == max_nodes {
                    snapshot.truncated = true;
                    break;
                }
                snapshot.nodes.push(SnapshotNode {
                    token,
                    parent: record.parent,
                    native: record.native,
                    test_id: record.test_id.clone(),
                    kind: match record.kind {
                        Kind::Root => "root",
                        Kind::Element => "element",
                        Kind::Field => "field",
                        Kind::Editor => "editor",
                        Kind::Image => "image",
                        Kind::Vector => "vector",
                        Kind::Text => "text",
                        Kind::Portal => "portal",
                    },
                });
                if let Some(child) = record.first {
                    current = Some(child);
                    continue;
                }
            }
            // Ascend through completed subtrees without recursion or a work stack.
            let mut completed = token;
            current = loop {
                let record = &self.records[&completed];
                if record.next.is_some() {
                    break record.next;
                }
                match record.parent {
                    Some(parent) => completed = parent,
                    None => break None,
                }
            };
        }
        snapshot
    }

    /// Revision of project-owned metadata, independent of native DOM/layout state.
    pub fn metadata_revision(&self) -> u64 { self.metadata_revision }

    /// Diagnostic metadata follows handle lifetime; it is not a native attribute.
    pub fn set_test_id(&mut self, handle: Handle, value: Option<String>) -> Result<(), BridgeError> {
        self.check_turn()?;
        if self.validate(handle)?.test_id != value {
            self.records.get_mut(&handle.token).expect("validated record").test_id = value;
            self.metadata_revision += 1;
        }
        Ok(())
    }

    pub(crate) fn dom_handle(&self) -> DomHandle { self.dom.clone() }

    pub(crate) fn stylesheet_prefix(&self) -> String {
        format!("zgui-js-{}-{}-", self.root.session, self.root.generation)
    }

    /// Editor-owned descendants retarget to the nearest public control wrapper.
    pub(crate) fn event_handle(&self, mut native: NodeId) -> Option<Handle> {
        if self.disposed || native.document() != self.root.document { return None; }
        loop {
            if let Some(&token) = self.native_tokens.get(&native) {
                let handle = self.handle(token);
                return self.validate(handle).ok().map(|_| handle);
            }
            native = self.dom.parent(native)?;
        }
    }

    /// Starts an outermost synchronous update. Nested turns are a host error.
    pub fn begin_turn(&mut self) {
        assert!(!self.disposed, "cannot begin a disposed registry turn");
        assert!(!self.turn_active, "structural turns cannot be nested");
        self.turn_active = true;
    }

    pub fn create_element(&mut self, tag: &str) -> Result<Handle, BridgeError> {
        self.check_allocation()?;
        let kind = match tag {
            "field" => Kind::Field,
            "editor" => Kind::Editor,
            "image" => Kind::Image,
            "vector" => Kind::Vector,
            "box" | "row" | "column" | "stack" | "text" | "label" | "scroll" | "control" => Kind::Element,
            _ => return Err(BridgeError::UnsupportedTag(tag.to_owned())),
        };
        let native = self.dom.create_element(ElementName::new(tag));
        Ok(self.register(native, kind))
    }

    pub fn create_portal(&mut self, layer: OverlayLayer) -> Result<Handle, BridgeError> {
        self.check_allocation()?;
        let parking = match self.portal_parking {
            Some(node) => node,
            None => {
                let node = self.dom.create_element(ElementName::new("box"));
                self.portal_parking = Some(node);
                node
            }
        };
        let native = self.dom.create_element(ElementName::new("box"));
        self.dom.set_style_text(native, Some("display: contents"));
        self.dom.insert(parking, native, None);
        let marker = self.dom.create_marker();
        let handle = self.register(native, Kind::Portal);
        self.portals.insert(handle.token, PortalRecord { marker, layer });
        Ok(handle)
    }

    pub fn create_text(&mut self, text: &str) -> Result<Handle, BridgeError> {
        self.check_allocation()?;
        let native = self.dom.create_text(text);
        Ok(self.register(native, Kind::Text))
    }

    pub fn replace_text(&mut self, node: Handle, text: &str) -> Result<(), BridgeError> {
        self.check_turn()?;
        let record = self.validate(node)?;
        if record.kind != Kind::Text {
            return Err(BridgeError::NotText);
        }
        if !self.dom.text_is(record.native, text) {
            self.dom.set_text(record.native, text);
        }
        Ok(())
    }

    pub fn is_text(&self, node: Handle) -> Result<bool, BridgeError> {
        Ok(self.validate(node)?.kind == Kind::Text)
    }

    pub(crate) fn require_editable(&self, node: Handle) -> Result<(), BridgeError> {
        if matches!(self.validate(node)?.kind, Kind::Field | Kind::Editor) {
            Ok(())
        } else {
            Err(BridgeError::NotEditable)
        }
    }

    pub(crate) fn require_field(&self, node: Handle) -> Result<(), BridgeError> {
        if self.validate(node)?.kind == Kind::Field { Ok(()) }
        else { Err(BridgeError::NotField) }
    }

    pub(crate) fn require_image(&self, node: Handle) -> Result<(), BridgeError> {
        if self.validate(node)?.kind == Kind::Image { Ok(()) }
        else { Err(BridgeError::NotImage) }
    }

    pub(crate) fn require_vector(&self, node: Handle) -> Result<(), BridgeError> {
        if self.validate(node)?.kind == Kind::Vector { Ok(()) }
        else { Err(BridgeError::NotVector) }
    }

    pub fn insert(
        &mut self,
        parent: Handle,
        node: Handle,
        anchor: Option<Handle>,
    ) -> Result<(), BridgeError> {
        self.check_turn()?;
        let parent_native = self.validate(parent)?.native;
        self.validate(node)?;
        let node_native = self.placement_native(node.token);
        if node == self.root {
            return Err(BridgeError::RootBoundary);
        }
        if self.records[&parent.token].kind == Kind::Text {
            return Err(BridgeError::TextParent);
        }
        if matches!(self.records[&parent.token].kind, Kind::Field | Kind::Editor) {
            return Err(BridgeError::EditableParent);
        }
        if self.records[&parent.token].kind == Kind::Image {
            return Err(BridgeError::ImageParent);
        }
        if self.records[&parent.token].kind == Kind::Vector {
            return Err(BridgeError::VectorParent);
        }
        let anchor_native = if let Some(anchor) = anchor {
            let record = self.validate(anchor)?;
            if record.parent != Some(parent.token) {
                return Err(BridgeError::InvalidAnchor);
            }
            Some(self.placement_native(anchor.token))
        } else {
            None
        };
        let mut ancestor = Some(parent.token);
        while let Some(token) = ancestor {
            if token == node.token {
                return Err(BridgeError::Cycle);
            }
            ancestor = self.records[&token].parent;
        }
        let record = &self.records[&node.token];
        if record.parent == Some(parent.token)
            && (anchor == Some(node) || record.next == anchor.map(|handle| handle.token))
        {
            return Ok(());
        }
        self.dom.insert(parent_native, node_native, anchor_native);
        self.unlink(node.token);
        let previous = anchor
            .and_then(|handle| self.records[&handle.token].previous)
            .or_else(|| if anchor.is_none() { self.records[&parent.token].last } else { None });
        if let Some(previous) = previous {
            self.records.get_mut(&previous).unwrap().next = Some(node.token);
        } else {
            self.records.get_mut(&parent.token).unwrap().first = Some(node.token);
        }
        if let Some(anchor) = anchor {
            self.records.get_mut(&anchor.token).unwrap().previous = Some(node.token);
        } else {
            self.records.get_mut(&parent.token).unwrap().last = Some(node.token);
        }
        let record = self.records.get_mut(&node.token).unwrap();
        record.parent = Some(parent.token);
        record.previous = previous;
        record.next = anchor.map(|handle| handle.token);
        record.pending_detach = false;
        debug_assert_eq!(self.dom.parent(node_native), Some(parent_native));
        self.sync_portals(Some(node.token));
        Ok(())
    }

    pub fn remove(&mut self, parent: Handle, node: Handle) -> Result<(), BridgeError> {
        self.check_turn()?;
        self.validate(parent)?;
        let record = self.validate(node)?;
        if node == self.root {
            return Err(BridgeError::RootBoundary);
        }
        if record.parent != Some(parent.token) {
            return Err(BridgeError::WrongParent);
        }
        let native = self.placement_native(node.token);
        self.dom.detach(native);
        self.unlink(node.token);
        let record = self.records.get_mut(&node.token).unwrap();
        record.pending_detach = true;
        self.pending.push(node.token);
        debug_assert_eq!(self.dom.parent(native), None);
        self.sync_portals(None);
        Ok(())
    }

    pub fn parent(&self, node: Handle) -> Result<Option<Handle>, BridgeError> {
        self.related(self.validate(node)?.parent)
    }

    pub fn first_child(&self, node: Handle) -> Result<Option<Handle>, BridgeError> {
        self.related(self.validate(node)?.first)
    }

    pub fn next_sibling(&self, node: Handle) -> Result<Option<Handle>, BridgeError> {
        self.related(self.validate(node)?.next)
    }

    /// Validates identity and native liveness before exposing a native ID to Rust.
    pub fn native(&self, node: Handle) -> Result<NodeId, BridgeError> {
        Ok(self.validate(node)?.native)
    }

    /// Invalidates still-detached trees, excluding descendants moved elsewhere.
    pub fn finish_turn(&mut self) -> Vec<Handle> {
        assert!(self.turn_active, "no structural turn is active");
        self.turn_active = false;
        let mut invalidated = Vec::new();
        for index in 0..self.pending.len() {
            let token = self.pending[index];
            if self.records.get(&token).is_some_and(|record| record.pending_detach && record.parent.is_none()) {
                self.invalidate_tree(token, &mut invalidated);
            }
        }
        self.pending.clear();
        self.retire_portal_parking();
        invalidated
    }

    /// Mutations are not rolled back: a failed turn retires the whole session.
    pub fn abort_turn(&mut self) -> Vec<Handle> {
        self.dispose()
    }

    /// Reclaims every owned tree, including never-linked allocation roots.
    /// The native host root is never detached. Repeated disposal is harmless.
    pub fn dispose(&mut self) -> Vec<Handle> {
        if self.disposed {
            return Vec::new();
        }
        let root_native = self.records[&0].native;
        let mut cleanup = None;
        for (&token, record) in &self.records {
            let native = self.placement_native(token);
            if token == 0 || !self.dom.is_live(native) {
                continue;
            }
            if record.parent == Some(0) {
                self.dom.detach(native);
            } else if record.parent.is_none() && self.dom.is_live(root_native) {
                // detach(never_linked) does not schedule collection. The box must
                // itself be attached before removal, even for a fully orphaned tree.
                let cleanup = *cleanup.get_or_insert_with(|| {
                    let node = self.dom.create_element(ElementName::new("box"));
                    self.dom.insert(root_native, node, None);
                    node
                });
                self.dom.insert(cleanup, native, None);
            }
        }
        for &token in self.portals.keys() {
            self.dom.detach(self.records[&token].native);
        }
        if let Some(parking) = self.portal_parking.take()
            && self.dom.is_live(parking) && self.dom.is_live(root_native)
        {
            self.dom.insert(root_native, parking, None);
            self.dom.detach(parking);
        }
        if let Some(cleanup) = cleanup {
            self.dom.detach(cleanup);
        }
        let handles = self.records.keys().map(|&token| self.handle(token)).collect();
        self.records.clear();
        self.native_tokens.clear();
        self.portals.clear();
        self.pending.clear();
        self.turn_active = false;
        self.disposed = true;
        handles
    }

    /// Number of owned live records, excluding the borrowed host root.
    pub fn live_count(&self) -> usize {
        self.records.len().saturating_sub(1)
    }

    fn handle(&self, token: u64) -> Handle {
        Handle { token, ..self.root }
    }

    fn validate(&self, handle: Handle) -> Result<&Record, BridgeError> {
        if handle.session != self.root.session {
            return Err(BridgeError::WrongSession);
        }
        if handle.generation != self.root.generation {
            return Err(BridgeError::WrongGeneration);
        }
        if handle.document != self.root.document {
            return Err(BridgeError::WrongDocument);
        }
        if self.disposed {
            return Err(BridgeError::Disposed);
        }
        let record = self.records.get(&handle.token).ok_or(BridgeError::StaleHandle)?;
        if record.native.document() != handle.document || !self.dom.is_live(record.native) {
            return Err(BridgeError::StaleHandle);
        }
        Ok(record)
    }

    fn related(&self, token: Option<u64>) -> Result<Option<Handle>, BridgeError> {
        token.map(|token| {
            let handle = self.handle(token);
            self.validate(handle)?;
            Ok(handle)
        }).transpose()
    }

    fn check_turn(&self) -> Result<(), BridgeError> {
        if self.disposed {
            Err(BridgeError::Disposed)
        } else if !self.turn_active {
            Err(BridgeError::TurnNotActive)
        } else {
            self.validate(self.root).map(|_| ())
        }
    }

    fn check_allocation(&self) -> Result<(), BridgeError> {
        self.check_turn()?;
        if self.live_count() >= self.max_nodes || self.next_token == u64::MAX {
            return Err(BridgeError::NodeLimit);
        }
        Ok(())
    }

    fn register(&mut self, native: NodeId, kind: Kind) -> Handle {
        let token = self.next_token;
        self.next_token += 1;
        self.records.insert(token, Record::new(native, kind));
        self.native_tokens.insert(native, token);
        self.handle(token)
    }

    fn unlink(&mut self, token: u64) {
        let record = &self.records[&token];
        let (parent, previous, next) = (record.parent, record.previous, record.next);
        if let Some(parent) = parent {
            if let Some(previous) = previous {
                self.records.get_mut(&previous).unwrap().next = next;
            } else {
                self.records.get_mut(&parent).unwrap().first = next;
            }
            if let Some(next) = next {
                self.records.get_mut(&next).unwrap().previous = previous;
            } else {
                self.records.get_mut(&parent).unwrap().last = previous;
            }
        }
        let record = self.records.get_mut(&token).unwrap();
        record.parent = None;
        record.previous = None;
        record.next = None;
    }

    fn placement_native(&self, token: u64) -> NodeId {
        self.portals.get(&token).map_or(self.records[&token].native, |portal| portal.marker)
    }

    fn attached(&self, token: u64) -> bool {
        let mut current = Some(token);
        while let Some(token) = current {
            if token == 0 { return true; }
            current = self.records[&token].parent;
        }
        false
    }

    fn sync_portals(&self, moved: Option<u64>) {
        if self.portals.is_empty() { return; }
        let parking = self.portal_parking.expect("portals have a parking root");
        let root = self.records[&0].native;
        for (&token, portal) in &self.portals {
            let native = self.records[&token].native;
            if !self.dom.is_live(native) { continue; }
            let attached = self.attached(token);
            let parent = if attached { self.dom.overlay_root(root, portal.layer) } else { parking };
            if self.dom.parent(native) != Some(parent) || (attached && moved == Some(token)) {
                self.dom.insert(parent, native, None);
            }
        }
    }

    fn retire_portal_parking(&mut self) {
        if !self.portals.is_empty() { return; }
        if let Some(parking) = self.portal_parking.take() {
            let root = self.records[&0].native;
            if self.dom.is_live(parking) && self.dom.is_live(root) {
                self.dom.insert(root, parking, None);
                self.dom.detach(parking);
            }
        }
    }

    fn invalidate_tree(&mut self, token: u64, invalidated: &mut Vec<Handle>) {
        let start = invalidated.len();
        invalidated.push(self.handle(token));
        let mut index = start;
        // The output doubles as a work queue, avoiding recursion on deep trees.
        while index < invalidated.len() {
            let record = self.records.remove(&invalidated[index].token).unwrap();
            if self.portals.remove(&invalidated[index].token).is_some() {
                self.dom.detach(record.native);
            }
            self.native_tokens.remove(&record.native);
            let mut child = record.first;
            while let Some(token) = child {
                invalidated.push(self.handle(token));
                child = self.records[&token].next;
            }
            index += 1;
        }
    }
}
