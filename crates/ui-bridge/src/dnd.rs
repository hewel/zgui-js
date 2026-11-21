//! Node-scoped drag-and-drop listener registry (`onDragEnter`, `onDragOver`,
//! `onDragLeave`, `onDrop`) owned by [`UiBridge`].
use std::collections::HashMap;

use rquickjs::{Ctx, Exception, Function, Persistent, Value};
use zgui::view::NodeId;

use crate::registry::Handle;

/// Drag-and-drop event listeners attachable to native DOM nodes via JSX props.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DndEventKind {
    DragEnter,
    DragOver,
    DragLeave,
    Drop,
}

impl DndEventKind {
    /// Parses a JSX event prop name into a [`DndEventKind`].
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "onDragEnter" => Some(Self::DragEnter),
            "onDragOver" => Some(Self::DragOver),
            "onDragLeave" => Some(Self::DragLeave),
            "onDrop" => Some(Self::Drop),
            _ => None,
        }
    }
}

struct ListenerEntry {
    owner_token: u64,
    callback: Persistent<Function<'static>>,
}

/// Persistent DnD callbacks keyed by `(NodeId, DndEventKind)` and indexed by owner token.
/// Drop before dropping the owning QuickJS runtime.
#[derive(Default)]
pub(crate) struct DndListeners {
    by_node: HashMap<(NodeId, DndEventKind), ListenerEntry>,
    by_owner: HashMap<(u64, DndEventKind), NodeId>,
}

impl DndListeners {
    pub(crate) fn set<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        owner: Handle,
        native: NodeId,
        kind: DndEventKind,
        value: Value<'js>,
    ) -> rquickjs::Result<()> {
        let callback = if value.is_null() || value.is_undefined() {
            None
        } else {
            Some(value.into_function().ok_or_else(|| {
                Exception::throw_type(
                    ctx,
                    "native DnD event properties accept only Function, null, or undefined",
                )
            })?)
        };
        let owner_key = (owner.token(), kind);
        if let Some(prev_native) = self.by_owner.remove(&owner_key) {
            self.by_node.remove(&(prev_native, kind));
        }
        if let Some(cb) = callback {
            self.by_owner.insert(owner_key, native);
            self.by_node.insert(
                (native, kind),
                ListenerEntry {
                    owner_token: owner.token(),
                    callback: Persistent::save(ctx, cb),
                },
            );
        }
        Ok(())
    }

    pub(crate) fn callback(&self, native: NodeId, kind: DndEventKind) -> Option<Persistent<Function<'static>>> {
        self.by_node.get(&(native, kind)).map(|entry| entry.callback.clone())
    }

    pub(crate) fn remove_owner(&mut self, token: u64) {
        self.by_owner.retain(|(owner_token, _), _| *owner_token != token);
        self.by_node.retain(|_, entry| entry.owner_token != token);
    }

    pub(crate) fn clear(&mut self) {
        self.by_owner.clear();
        self.by_node.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.by_node.len()
    }
}

impl Drop for DndListeners {
    fn drop(&mut self) {
        self.clear();
    }
}
