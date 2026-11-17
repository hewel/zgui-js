use super::*;

const MAX_RECORDS: usize = 4096;

impl ImageLoader {
    pub(crate) fn configure(&mut self, policy: crate::ImagePolicy) {
        assert!(self.entries.is_empty(), "image policy must be selected before mounting");
        self.completed = Tasks::new(policy);
    }

    pub(crate) fn replace_policy(
        &mut self,
        policy: crate::ImagePolicy,
        document: &Rc<RefCell<Document>>,
        content: &mut ContentCache,
    ) {
        self.completed.cancel();
        self.pending.borrow_mut().clear();
        let previous = std::mem::take(&mut self.by_node);
        {
            let document = document.borrow();
            document.edit(&zgui_dom::EverythingMatters, |edit| {
                for &node in previous.keys() {
                    let Some(index) = document.store().index_of(node) else { continue; };
                    edit.set_state(index, zgui_vocab::UiState::BROKEN, false);
                    edit.set_custom_state(index, zgui_interned::Ident::new("zgui-image-broken"), false);
                }
            }).expect("image state updates require a live document");
        }
        let mut document = document.borrow_mut();
        for node in previous.into_keys() {
            self.retire_request(node);
            let id = ReplacedId::new(node);
            content.remove_image(id);
            self.intrinsics.remove(id);
            if let Some(index) = document.store().index_of(node) {
                document.replaced_content_changed(index);
            }
        }
        for entry in self.entries.drain().map(|(_, entry)| entry) {
            for variant in entry.variants.into_values() {
                content.remove_shared_tile(variant.handle);
            }
        }
        // Source strings may be identical across packages; the completion domain and every
        // lease are not. Handle/request counters intentionally survive to prevent reuse.
        self.completed = Tasks::new(policy);
        self.failures.clear();
        self.graphics_failure = None;
        self.audited_revision = None;
        self.revision += 1;
        self.recount();
    }

    pub(crate) fn revision(&self) -> u64 { self.revision }

    pub(crate) fn poll_barrier(&mut self) -> bool {
        let was_ready = self.completed.has_ready();
        let ready = self.completed.poll();
        if !was_ready && self.completed.has_ready() { self.revision += 1; }
        ready
    }

    pub(super) fn retire_request(&mut self, node: NodeKey) {
        if let Some(mut request) = self.requests.remove(&node) {
            request.state = ImageState::Canceled;
            request.visible = false;
            request.attached_variant = None;
            request.uploaded = false;
            if self.retired.len() == MAX_RECORDS {
                self.retired.pop_front();
                self.dropped += 1;
            }
            self.retired.push_back(request);
            self.revision += 1;
        }
    }

    pub(crate) fn resources(&self, content: &ContentCache) -> ImageResources {
        let mut resources: Vec<_> = self.requests.iter().take(MAX_RECORDS).map(|(node, request)| {
            let mut request = request.clone();
            if let Some(entry) = self.by_node.get(node).and_then(|key| self.entries.get(key)) {
                request.requested_variant = entry.target_class(self.limits.max_dimension);
                request.attached_variant = (!entry.attach_owed).then_some(entry.active).flatten();
                request.uploaded = request.attached_variant.and_then(|class| entry.variants.get(&class))
                    .is_some_and(|variant| variant.uploaded && content.image_tile_resident(variant.handle));
                request.failure = entry.failure.clone();
                request.state = if matches!(entry.probe, Probe::Failed) || request.failure.is_some() {
                    ImageState::Failed
                } else if request.requested_variant.is_some_and(|target| {
                    request.attached_variant.is_some_and(|active| active >= target)
                        && request.uploaded
                        && matches!(entry.probe, Probe::Done)
                        && entry.variants.get(&entry.active.unwrap()).is_some_and(|variant| {
                            matches!(variant.state, VariantState::Ready(_) | VariantState::Uploaded { .. })
                        })
                }) {
                    ImageState::Ready
                } else if request.requested_variant.is_some() || matches!(entry.probe, Probe::Pending) {
                    ImageState::Loading
                } else { ImageState::Requested };
            }
            request
        }).collect();
        resources.sort_by_key(|request| request.generation);
        let truncated = self.requests.len() > MAX_RECORDS;
        let remaining = MAX_RECORDS.saturating_sub(resources.len());
        let dropped = self.dropped + self.retired.len().saturating_sub(remaining) as u64;
        resources.extend(self.retired.iter().cloned().take(remaining));
        resources.sort_by_key(|request| request.generation);
        let (declared_assets, dropped_declarations) = self.completed.declarations(MAX_RECORDS);
        ImageResources { resources, dropped, truncated, failures: self.failures.clone(), declared_assets, dropped_declarations, graphics_failure: self.graphics_failure.clone() }
    }

    pub(crate) fn audit_css(&mut self, css: &str) -> bool {
        if !self.completed.restricted() { return true; }
        let mut input = cssparser::ParserInput::new(css);
        if forbidden_css(&mut cssparser::Parser::new(&mut input)) {
            self.policy_failure("CSS resource URLs, @import, and image-set are unsupported by the native painter; use an image element with a declared package src");
            return false;
        }
        true
    }

    pub(super) fn policy_failure(&mut self, message: &str) {
        if self.failures.len() < 64 && !self.failures.iter().any(|failure| failure == message) {
            self.failures.push(message.to_owned());
            self.revision += 1;
        }
    }

    pub(crate) fn audit_inline(&mut self, document: &Rc<RefCell<Document>>, revision: u64) {
        if !self.completed.restricted() || self.audited_revision == Some(revision) { return; }
        self.audited_revision = Some(revision);
        let document = document.borrow();
        let store = document.store();
        let Some(root) = document.root_index() else { return; };
        let mut stack = vec![root];
        let guard = store.lock().read();
        while let Some(index) = stack.pop() {
            let key = store.key_of(index);
            if let Some(block) = store.columns().inline_style.get(key).and_then(Option::as_ref) {
                let mut css = String::new();
                if block.read_with(&guard).to_css(&mut css).is_ok() { self.audit_css(&css); }
            }
            let mut child = store.core(index).first_child();
            while let Some(index) = child {
                stack.push(index);
                child = store.core(index).next_sibling();
            }
        }
    }
}

fn forbidden_css(parser: &mut cssparser::Parser<'_, '_>) -> bool {
    use cssparser::Token;
    while let Ok(token) = parser.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::UnquotedUrl(_) | Token::BadUrl(_) => return true,
            Token::AtKeyword(name) if name.eq_ignore_ascii_case("import") => return true,
            Token::Function(name) if name.eq_ignore_ascii_case("url") || name.eq_ignore_ascii_case("image-set") || name.eq_ignore_ascii_case("-webkit-image-set") => return true,
            Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let nested: Result<bool, cssparser::ParseError<'_, ()>> = parser.parse_nested_block(|parser| Ok(forbidden_css(parser)));
                if nested.unwrap_or(true) { return true; }
            }
            _ => {}
        }
    }
    false
}
