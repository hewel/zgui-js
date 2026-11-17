use std::{cell::RefCell, collections::{BTreeMap, HashSet}, rc::{Rc, Weak}};

use rquickjs::{Ctx, Exception};
use zgui::view::{DomHandle, EventCx, ListenerId, NodeId, OverlayLayer};
use zgui::vocab::{EventKind, ListenerOptions};

use crate::{events::{Dispatcher, DismissReason, dismiss_reason}, registry::Handle};

#[derive(Clone)]
pub(crate) struct Options {
    pub band: OverlayLayer,
    pub outside: bool,
    pub escape: bool,
    pub exclude: Option<(Handle, NodeId)>,
    pub leaving: bool,
}

#[derive(Clone)]
struct Entry { node: NodeId, options: Options, callback: u64, dispatcher: Rc<Dispatcher> }

struct WindowLayers {
    dom: DomHandle, root: NodeId, next: u64, generation: u64,
    entries: BTreeMap<u64, Entry>, listeners: Vec<ListenerId>,
}

thread_local! {
    static WINDOWS: RefCell<Vec<Weak<RefCell<WindowLayers>>>> = const { RefCell::new(Vec::new()) };
}

fn contains(dom: &DomHandle, root: NodeId, mut node: NodeId) -> bool {
    while dom.is_live(node) {
        if node == root { return true; }
        let Some(parent) = dom.parent(node) else { return false; };
        node = parent;
    }
    false
}

impl WindowLayers {
    fn shared(dom: DomHandle, root: NodeId) -> Rc<RefCell<Self>> {
        WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            windows.retain(|entry| entry.strong_count() != 0);
            for entry in windows.iter().filter_map(Weak::upgrade) {
                let matches = { let held = entry.borrow(); held.dom.ptr_eq(&dom) && held.root == root };
                if matches { return entry; }
            }
            let value = Rc::new(RefCell::new(Self { dom, root, next: 1, generation: 0,
                entries: BTreeMap::new(), listeners: Vec::new() }));
            windows.push(Rc::downgrade(&value));
            value
        })
    }

    fn add(shared: &Rc<RefCell<Self>>, id: u64, entry: Entry) {
        let mut window = shared.borrow_mut();
        window.entries.insert(id, entry);
        if !window.listeners.is_empty() { return; }
        window.generation = id;
        for kind in [EventKind::PointerDown, EventKind::KeyDown] {
            let weak = Rc::downgrade(shared);
            let listener = window.dom.add_listener(window.root, kind, ListenerOptions::CAPTURE,
                Rc::new(move |event| {
                    let Some(window) = weak.upgrade() else { return; };
                    let selected = window.borrow().select(id, event);
                    if let Some((reason, callback, dispatcher)) = selected {
                        if reason == DismissReason::Escape {
                            event.stop_immediate_propagation();
                            event.prevent_default();
                        }
                        dispatcher(callback, event);
                    }
                }));
            window.listeners.push(listener);
        }
    }

    fn select(&self, generation: u64, event: &EventCx<'_>) -> Option<(DismissReason, u64, Rc<Dispatcher>)> {
        if self.generation != generation || self.listeners.is_empty() { return None; }
        let reason = dismiss_reason(event.kind, event.payload())?;
        let (_, entry) = self.entries.iter()
            .filter(|(_, entry)| contains(&self.dom, self.root, entry.node)
                && (reason != DismissReason::Escape || !entry.options.leaving))
            .max_by_key(|(id, entry)| (entry.options.band, **id))?;
        let accepted = match reason {
            DismissReason::Escape => entry.options.escape,
            DismissReason::OutsidePress => entry.options.outside
                && !contains(&self.dom, entry.node, event.target)
                && !entry.options.exclude.as_ref().is_some_and(|(_, node)| contains(&self.dom, *node, event.target)),
        };
        accepted.then(|| (reason, entry.callback, entry.dispatcher.clone()))
    }

    fn remove(shared: &Rc<RefCell<Self>>, id: u64) {
        let (dom, root, listeners) = {
            let mut window = shared.borrow_mut();
            window.entries.remove(&id);
            let listeners = if window.entries.is_empty() { std::mem::take(&mut window.listeners) } else { Vec::new() };
            (window.dom.clone(), window.root, listeners)
        };
        for listener in listeners { dom.remove_listener(root, listener); }
    }
}

impl Drop for WindowLayers {
    fn drop(&mut self) {
        for listener in self.listeners.drain(..) { self.dom.remove_listener(self.root, listener); }
    }
}

pub(crate) struct Dismissals {
    dom: DomHandle, root: NodeId, window: Option<Rc<RefCell<WindowLayers>>>,
    held: BTreeMap<u64, (u64, u64)>, limit: usize,
}

impl Dismissals {
    pub(crate) fn new(dom: DomHandle, root: NodeId, limit: usize) -> Self {
        Self { dom, root, window: None, held: BTreeMap::new(), limit }
    }

    pub(crate) fn reserve(&mut self, ctx: &Ctx<'_>) -> rquickjs::Result<u64> {
        if self.held.len() >= self.limit {
            return Err(Exception::throw_range(ctx, "native dismiss layer limit reached"));
        }
        let window = self.window.get_or_insert_with(|| WindowLayers::shared(self.dom.clone(), self.dom.root(self.root)));
        let mut window = window.borrow_mut();
        let id = window.next;
        window.next = id.checked_add(1).ok_or_else(|| Exception::throw_range(ctx, "native dismiss layer identifiers exhausted"))?;
        Ok(id)
    }

    pub(crate) fn add(&mut self, id: u64, owner: Handle, node: NodeId, options: Options, callback: u64, dispatcher: Rc<Dispatcher>) {
        self.held.insert(id, (owner.token(), callback));
        let window = self.window.as_ref().expect("reserved before add").clone();
        WindowLayers::add(&window, id, Entry { node, options, callback, dispatcher });
    }

    pub(crate) fn update(&mut self, ctx: &Ctx<'_>, id: u64, options: Options) -> rquickjs::Result<()> {
        if !self.held.contains_key(&id) {
            return Err(Exception::throw_reference(ctx, "native dismiss layer is disposed"));
        }
        let window = self.window.as_ref().expect("held implies window");
        let mut window = window.borrow_mut();
        if let Some(entry) = window.entries.get_mut(&id) {
            entry.options = options;
        }
        Ok(())
    }

    pub(crate) fn take(&mut self, id: u64) -> Option<u64> {
        let (_, callback) = self.held.remove(&id)?;
        if let Some(window) = &self.window {
            WindowLayers::remove(window, id);
        }
        Some(callback)
    }

    pub(crate) fn remove_nodes(&mut self, invalid: &[Handle]) -> Vec<u64> {
        if invalid.is_empty() || self.held.is_empty() {
            return Vec::new();
        }
        let tokens: HashSet<u64> = invalid.iter().map(|handle| handle.token()).collect();
        let mut callbacks = Vec::new();
        let ids: Vec<u64> = self.held.iter()
            .filter(|(_, (owner, _))| tokens.contains(owner))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(callback) = self.take(id) { callbacks.push(callback); }
        }
        if let Some(window) = &self.window {
            let mut window = window.borrow_mut();
            for id in self.held.keys() {
                let Some(entry) = window.entries.get_mut(id) else { continue; };
                if entry.options.exclude.as_ref().is_some_and(|(handle, _)| tokens.contains(&handle.token())) {
                    entry.options.exclude = None;
                }
            }
        }
        callbacks
    }

    pub(crate) fn clear(&mut self) -> Vec<u64> {
        let ids: Vec<u64> = self.held.keys().rev().copied().collect();
        let mut callbacks = Vec::new();
        for id in ids {
            if let Some(callback) = self.take(id) { callbacks.push(callback); }
        }
        self.window.take();
        callbacks
    }
}

impl Drop for Dismissals {
    fn drop(&mut self) { self.clear(); }
}
