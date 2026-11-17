use std::{cell::{Cell, RefCell}, collections::{BTreeMap, HashSet}, rc::{Rc, Weak}};
use rquickjs::{Ctx, Exception};
use zgui::view::{DomHandle, HostHandle, NodeId};
use crate::registry::Handle;

struct WindowLock {
    dom: DomHandle, root: NodeId, host: HostHandle, holders: Cell<usize>,
}
thread_local! {
    static WINDOWS: RefCell<Vec<Weak<WindowLock>>> = const { RefCell::new(Vec::new()) };
}
impl WindowLock {
    fn shared(dom: DomHandle, root: NodeId, host: HostHandle) -> Rc<Self> {
        WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            windows.retain(|entry| entry.strong_count() != 0);
            for entry in windows.iter().filter_map(Weak::upgrade) {
                if entry.dom.ptr_eq(&dom) && entry.root == root { return entry; }
            }
            let value = Rc::new(Self { dom, root, host, holders: Cell::new(0) });
            windows.push(Rc::downgrade(&value));
            value
        })
    }
    fn acquire(&self) {
        let previous = self.holders.get();
        self.holders.set(previous + 1);
        if previous == 0 { self.host.freeze_scrolling(true); }
    }
    fn release(&self, count: usize) {
        let remaining = self.holders.get() - count;
        self.holders.set(remaining);
        if remaining == 0 { self.host.freeze_scrolling(false); }
    }
}

pub(crate) struct ScrollLocks {
    dom: DomHandle, root: NodeId, host: HostHandle,
    window: Option<Rc<WindowLock>>, held: BTreeMap<u64, u64>, next: u64, limit: usize,
}
impl ScrollLocks {
    pub(crate) fn new(dom: DomHandle, root: NodeId, host: HostHandle, limit: usize) -> Self {
        Self { dom, root, host, window: None, held: BTreeMap::new(), next: 1, limit }
    }
    pub(crate) fn reserve(&mut self, ctx: &Ctx<'_>) -> rquickjs::Result<u64> {
        if self.held.len() >= self.limit {
            return Err(Exception::throw_range(ctx, "native scroll lock limit reached"));
        }
        let id = self.next;
        self.next = id.checked_add(1).ok_or_else(|| Exception::throw_range(ctx, "native scroll lock identifiers exhausted"))?;
        self.window.get_or_insert_with(|| WindowLock::shared(self.dom.clone(), self.dom.root(self.root), self.host.clone()));
        Ok(id)
    }
    pub(crate) fn add(&mut self, id: u64, owner: Handle) {
        self.held.insert(id, owner.token());
        self.window.as_ref().expect("reserved before add").acquire();
    }
    fn release(&self, count: usize) {
        if count != 0 { self.window.as_ref().expect("held implies window").release(count); }
    }
    pub(crate) fn take(&mut self, id: u64) {
        if self.held.remove(&id).is_some() { self.release(1); }
    }
    pub(crate) fn remove_nodes(&mut self, invalid: &[Handle]) {
        if invalid.is_empty() || self.held.is_empty() { return; }
        let tokens: HashSet<u64> = invalid.iter().map(|handle| handle.token()).collect();
        let previous = self.held.len();
        self.held.retain(|_, owner| !tokens.contains(owner));
        self.release(previous - self.held.len());
    }
    pub(crate) fn clear(&mut self) {
        let count = self.held.len();
        self.held.clear();
        self.release(count);
    }
    pub(crate) fn len(&self) -> usize { self.held.len() }
}
impl Drop for ScrollLocks { fn drop(&mut self) { self.clear(); } }
