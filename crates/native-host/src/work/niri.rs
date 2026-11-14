use super::*;
use host_services::{ServiceError, niri::{NiriSnapshot, NiriSubscription}};
use rquickjs::{Array, IntoJs};
use std::{ops::Bound::{Excluded, Unbounded}, path::PathBuf};

const MAX_SUBSCRIPTIONS: usize = 4;

pub(super) struct NiriListener {
    transport: NiriSubscription,
    snapshot: Persistent<Function<'static>>,
    error: Persistent<Function<'static>>,
}

pub(crate) struct NiriDelivery {
    generation: u64,
    id: u64,
    result: Result<NiriSnapshot, ServiceError>,
}

impl Work {
    pub(super) fn watch_niri<'js>(self: &Rc<Self>, ctx: &Ctx<'js>, snapshot: Function<'js>, error: Function<'js>) -> rquickjs::Result<Object<'js>> {
        self.require_live_service(ctx, "watchNiri")?;
        let path = std::env::var_os("NIRI_SOCKET").map(PathBuf::from);
        self.subscribe_niri(ctx, path, snapshot, error)
    }

    fn subscribe_niri<'js>(self: &Rc<Self>, ctx: &Ctx<'js>, path: Option<PathBuf>, snapshot: Function<'js>, error: Function<'js>) -> rquickjs::Result<Object<'js>> {
        self.require_live_service(ctx, "watchNiri")?;
        self.require_creation(ctx)?;
        if self.subscriptions.borrow().len() >= MAX_SUBSCRIPTIONS {
            return Err(Exception::throw_range(ctx, "niri subscription limit reached"));
        }
        let path = path.filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| Exception::throw_type(ctx, "NIRI_SOCKET is not configured"))?;
        let id = self.id(ctx)?;
        let object = Object::new(ctx.clone())?;
        let weak = Rc::downgrade(self);
        object.prop("cancel", data_property(Func::from(move |ctx: Ctx<'js>| {
            owner(&weak, &ctx)?.subscriptions.borrow_mut().remove(&id);
            Ok::<(), rquickjs::Error>(())
        })))?;
        let transport = NiriSubscription::start(path, self.wake.clone())
            .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
        self.subscriptions.borrow_mut().insert(id, NiriListener {
            transport, snapshot: Persistent::save(ctx, snapshot), error: Persistent::save(ctx, error),
        });
        Ok(object)
    }

    pub(crate) fn next_subscription(&self) -> Option<NiriDelivery> {
        if self.mode.get() != Mode::Mounted { return None; }
        let subscriptions = self.subscriptions.borrow();
        let cursor = self.subscription_cursor.get();
        let delivery = subscriptions.range((Excluded(cursor), Unbounded)).chain(subscriptions.range(..=cursor))
            .find_map(|(&id, listener)| listener.transport.take().map(|result| NiriDelivery { generation: self.generation, id, result }));
        if let Some(delivery) = &delivery { self.subscription_cursor.set(delivery.id); }
        delivery
    }

    pub(crate) fn has_subscriptions(&self) -> bool {
        self.mode.get() == Mode::Mounted && self.subscriptions.borrow().values().any(|listener| listener.transport.has_pending())
    }

    pub(crate) fn deliver_subscription<'js>(&self, ctx: &Ctx<'js>, delivery: NiriDelivery) -> rquickjs::Result<()> {
        if delivery.generation != self.generation || self.mode.get() != Mode::Mounted { return Ok(()); }
        match delivery.result {
            Ok(snapshot) => {
                let callback = self.subscriptions.borrow().get(&delivery.id).map(|listener| listener.snapshot.clone());
                let Some(callback) = callback else { return Ok(()); };
                // No registry or transport borrow survives conversion or entry into JS.
                callback.restore(ctx)?.call::<_, ()>((snapshot_object(ctx, snapshot)?,))
            }
            Err(error) => {
                if !self.subscriptions.borrow().contains_key(&delivery.id) { return Ok(()); }
                // Error construction in the engine may invoke an inherited message
                // setter. Recheck cancellation afterward and release roots on errors too.
                let argument = service_error(ctx, error.code, &error.message);
                let listener = self.subscriptions.borrow_mut().remove(&delivery.id);
                let Some(listener) = listener else { return argument.map(|_| ()); };
                let NiriListener { transport, snapshot, error: callback } = listener;
                drop((transport, snapshot));
                let argument = argument?;
                if self.mode.get() != Mode::Mounted { return Ok(()); }
                callback.restore(ctx)?.call::<_, ()>((argument,))
            }
        }
    }
}

fn snapshot_object<'js>(ctx: &Ctx<'js>, snapshot: NiriSnapshot) -> rquickjs::Result<Object<'js>> {
    let windows = Array::new(ctx.clone())?;
    for (index, window) in (0u32..).zip(snapshot.windows) {
        let row = Object::new(ctx.clone())?;
        row.prop("id", data_property(window.id.to_string()))?;
        row.prop("title", data_property(nullable(ctx, window.title.as_deref())?))?;
        row.prop("appId", data_property(nullable(ctx, window.app_id.as_deref())?))?;
        row.prop("pid", data_property(nullable(ctx, window.pid)?))?;
        row.prop("workspaceId", data_property(nullable(ctx, window.workspace_id.map(|id| id.to_string()))?))?;
        row.prop("focused", data_property(window.is_focused))?;
        row.prop("floating", data_property(window.is_floating))?;
        row.prop("urgent", data_property(window.is_urgent))?;
        windows.as_object().prop(index, data_property(row))?;
    }
    let workspaces = Array::new(ctx.clone())?;
    for (index, workspace) in (0u32..).zip(snapshot.workspaces) {
        let row = Object::new(ctx.clone())?;
        row.prop("id", data_property(workspace.id.to_string()))?;
        row.prop("index", data_property(workspace.idx))?;
        row.prop("name", data_property(nullable(ctx, workspace.name.as_deref())?))?;
        row.prop("output", data_property(nullable(ctx, workspace.output.as_deref())?))?;
        row.prop("urgent", data_property(workspace.is_urgent))?;
        row.prop("active", data_property(workspace.is_active))?;
        row.prop("focused", data_property(workspace.is_focused))?;
        row.prop("activeWindowId", data_property(nullable(ctx, workspace.active_window_id.map(|id| id.to_string()))?))?;
        workspaces.as_object().prop(index, data_property(row))?;
    }
    let object = Object::new(ctx.clone())?;
    object.prop("windows", data_property(windows))?;
    object.prop("workspaces", data_property(workspaces))?;
    Ok(object)
}

fn nullable<'js>(ctx: &Ctx<'js>, value: Option<impl IntoJs<'js>>) -> rquickjs::Result<Value<'js>> {
    match value {
        Some(value) => value.into_js(ctx),
        None => Ok(Value::new_null(ctx.clone())),
    }
}

#[cfg(all(test, unix))]
mod tests;
