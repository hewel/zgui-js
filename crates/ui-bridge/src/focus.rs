use std::collections::{BTreeMap, HashSet};
use rquickjs::{Ctx, Exception};
use zgui::view::FocusTrap;
use crate::registry::Handle;

pub(crate) struct FocusTraps {
    held: BTreeMap<u64, (u64, FocusTrap)>,
    next: u64,
    limit: usize,
}

impl FocusTraps {
    pub(crate) fn new(limit: usize) -> Self {
        Self { held: BTreeMap::new(), next: 1, limit }
    }

    pub(crate) fn reserve(&mut self, ctx: &Ctx<'_>) -> rquickjs::Result<u64> {
        if self.held.len() >= self.limit {
            return Err(Exception::throw_range(ctx, "native focus trap limit reached"));
        }
        let id = self.next;
        self.next = id.checked_add(1).ok_or_else(|| Exception::throw_range(ctx, "native focus trap identifiers exhausted"))?;
        Ok(id)
    }

    pub(crate) fn add(&mut self, id: u64, owner: Handle, trap: FocusTrap) {
        self.held.insert(id, (owner.token(), trap));
    }

    pub(crate) fn take(&mut self, id: u64) -> Option<FocusTrap> {
        self.held.remove(&id).map(|(_, trap)| trap)
    }

    pub(crate) fn remove_nodes(&mut self, invalid: &[Handle]) -> Vec<FocusTrap> {
        if invalid.is_empty() || self.held.is_empty() { return Vec::new(); }
        let tokens: HashSet<u64> = invalid.iter().map(|handle| handle.token()).collect();
        let ids: Vec<u64> = self.held.iter().rev()
            .filter_map(|(&id, (owner, _))| tokens.contains(owner).then_some(id)).collect();
        ids.into_iter().filter_map(|id| self.take(id)).collect()
    }

    pub(crate) fn take_all(&mut self) -> Vec<FocusTrap> {
        std::mem::take(&mut self.held).into_iter().rev().map(|(_, (_, trap))| trap).collect()
    }

    pub(crate) fn len(&self) -> usize { self.held.len() }
}
