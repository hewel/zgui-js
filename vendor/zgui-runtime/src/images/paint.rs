//! Observe the actual paint source seam, without replacing allocation or pixel production.
use super::*;
use zgui_paint::{emit::replaced::Source, walk::order::ReplacedSource};

pub(crate) struct PaintImages<'a> {
    pub(crate) source: &'a dyn ReplacedSource,
    pub(crate) images: &'a ImageLoader,
    pub(crate) refusals: &'a RefCell<Vec<NodeKey>>,
}

impl ReplacedSource for PaintImages<'_> {
    fn revision(&self, id: ReplacedId) -> u64 { self.source.revision(id) }

    fn source(&self, id: ReplacedId) -> Option<Source> {
        let source = self.source.source(id);
        if source.is_none() && self.images.decoded_attachment(id.node()) {
            self.refusals.borrow_mut().push(id.node());
        }
        source
    }

    fn clipped_source(
        &self,
        id: ReplacedId,
        placement: &zgui_paint::emit::replaced::ReplacedPlacement,
        clip_path: &zgui_paint::lower::ClipPathShape,
        reference_box: zgui_geom::Rect<zgui_geom::DevicePx, zgui_geom::Device>,
    ) -> Option<Source> {
        let source = self
            .source
            .clipped_source(id, placement, clip_path, reference_box);
        if source.is_none() && self.images.decoded_attachment(id.node()) {
            self.refusals.borrow_mut().push(id.node());
        }
        source
    }
}

impl ImageLoader {
    pub(crate) fn restricted_sources(&self) -> bool { self.completed.restricted() }

    fn decoded_attachment(&self, node: NodeKey) -> bool {
        self.by_node.get(&node).and_then(|key| self.entries.get(key))
            .and_then(|entry| entry.active.and_then(|class| entry.variants.get(&class)))
            .is_some_and(|variant| matches!(variant.state, VariantState::Ready(_)))
    }

    pub(crate) fn begin_paint(&mut self, content: &ContentCache) {
        if !self.completed.restricted() { return; }
        for entry in self.entries.values_mut() {
            for variant in entry.variants.values_mut() {
                if variant.uploaded && !content.image_tile_resident(variant.handle) {
                    variant.uploaded = false;
                    self.revision += 1;
                }
            }
        }
    }

    pub(crate) fn finish_paint(&mut self, refusals: Vec<NodeKey>, upload_error: Option<String>) {
        if !self.completed.restricted() { return; }
        if self.graphics_failure.is_none() && upload_error.is_some() {
            self.graphics_failure = upload_error;
            self.revision += 1;
        }
        for node in refusals {
            let Some(key) = self.by_node.get(&node) else { continue; };
            let Some(entry) = self.entries.get_mut(key) else { continue; };
            let Some(active) = entry.active.and_then(|class| entry.variants.get(&class)) else { continue; };
            // No decoded attachment is an ordinary loading placeholder. Uploaded tile loss is
            // recoverable through take_missing_images/redecode_missing. Only refusal of actual
            // decoded content at the paint seam is an allocation/attachment contract failure.
            if matches!(active.state, VariantState::Ready(_)) && entry.failure.is_none() {
                entry.failure = Some("native paint could not allocate or resolve the decoded image in the atlas".into());
                self.revision += 1;
            }
        }
    }
}
