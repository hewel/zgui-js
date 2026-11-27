//! Session-scoped coordinator for gesture-bound external `xdg_activation_v1` token minting.
//!
//! Enforces single-issuance per physical press, synchronous Wayland wire commit before
//! returning a JavaScript `Promise<string | null>`, a surface-independent 500 ms one-shot
//! timeout on `AppTimers`, fail-closed revocation under retained `SessionLockState` or staged
//! candidate mounts, and leak-free retirement without cancelling on component `onCleanup`.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    time::Duration,
};

use rquickjs::{
    Ctx, Exception, Function, Object, Persistent, Promise, Value,
    function::{Func, Opt},
};
use zgui::{
    platform::{ActivationToken, ActivationTokenSerial, PendingActivationToken},
    runtime::AppTimers,
    view::TimerId,
};

use super::Session;

pub(crate) const ACTIVATION_TOKEN_TIMEOUT: Duration = Duration::from_millis(500);
pub(crate) const MAX_IN_FLIGHT_ACTIVATION_TOKENS: usize = 32;
const MAX_APP_ID_BYTES: usize = 256;

struct InFlightToken {
    resolve: Persistent<Function<'static>>,
    pending: PendingActivationToken,
    timer: TimerId,
}

#[derive(Default)]
pub(crate) struct ActivationTokens {
    in_flight: RefCell<HashMap<ActivationTokenSerial, InFlightToken>>,
    ready: RefCell<VecDeque<(Persistent<Function<'static>>, Option<String>)>>,
    timers: RefCell<Option<AppTimers>>,
}

impl ActivationTokens {
    pub(crate) fn new(timers: Option<AppTimers>) -> Self {
        Self {
            in_flight: RefCell::new(HashMap::new()),
            ready: RefCell::new(VecDeque::new()),
            timers: RefCell::new(timers),
        }
    }

    pub(crate) fn set_timers(&self, timers: AppTimers) {
        *self.timers.borrow_mut() = Some(timers);
    }

    #[must_use]
    pub(crate) fn has_ready(&self) -> bool {
        !self.ready.borrow().is_empty()
    }

    #[must_use]
    pub(crate) fn owns(&self, serial: ActivationTokenSerial) -> bool {
        self.in_flight.borrow().contains_key(&serial)
    }

    /// Cancels all in-flight Wayland token objects and timers and resolves their Promises to `null`
    /// on the next checkpoint (for instance when `SessionLockState` becomes retained).
    pub(crate) fn revoke_all_to_null(&self) -> bool {
        let drained: Vec<InFlightToken> = self
            .in_flight
            .borrow_mut()
            .drain()
            .map(|(_, entry)| entry)
            .collect();
        if drained.is_empty() {
            return false;
        }
        let timers = self.timers.borrow().clone();
        let mut ready = self.ready.borrow_mut();
        for entry in drained {
            if let Some(timers) = &timers {
                timers.cancel(entry.timer);
            }
            entry.pending.cancel();
            ready.push_back((entry.resolve, None));
        }
        true
    }

    /// Releases all persistent JS roots, cancels timers, and destroys any pending protocol objects
    /// before dropping the owning QuickJS runtime.
    pub(crate) fn clear(&self) {
        let drained: Vec<InFlightToken> = self
            .in_flight
            .borrow_mut()
            .drain()
            .map(|(_, entry)| entry)
            .collect();
        let timers = self.timers.borrow().clone();
        for entry in drained {
            if let Some(timers) = &timers {
                timers.cancel(entry.timer);
            }
            entry.pending.cancel();
        }
        self.ready.borrow_mut().clear();
    }
}

fn parse_app_id<'js>(ctx: &Ctx<'js>, options: Opt<Value<'js>>) -> rquickjs::Result<Option<String>> {
    let Some(value) = options.0 else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    if value.is_null() || value.is_array() || value.is_function() {
        return Err(Exception::throw_type(
            ctx,
            "requestActivationToken options must be a plain object",
        ));
    }
    let Some(object) = value.as_object() else {
        return Err(Exception::throw_type(
            ctx,
            "requestActivationToken options must be a plain object",
        ));
    };
    let raw: Value<'js> = object.get("appId")?;
    if raw.is_undefined() {
        return Ok(None);
    }
    let Some(js_str) = raw.as_string() else {
        return Err(Exception::throw_type(
            ctx,
            "requestActivationToken appId must be a string",
        ));
    };
    let text = js_str.to_string()?;
    if text.chars().any(char::is_control) {
        return Err(Exception::throw_type(
            ctx,
            "requestActivationToken appId must not contain control characters",
        ));
    }
    if text.len() > MAX_APP_ID_BYTES {
        return Err(Exception::throw_type(
            ctx,
            "requestActivationToken appId exceeds 256 UTF-8 bytes",
        ));
    }
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

impl Session {
    pub(crate) fn set_app_timers(&self, timers: AppTimers) {
        self.activation.set_timers(timers);
    }

    pub(crate) fn install_activation_api<'js>(
        self: &Rc<Self>,
        shell_api: &Object<'js>,
    ) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        shell_api.set(
            "requestActivationToken",
            Func::from(move |ctx: Ctx<'js>, options: Opt<Value<'js>>| {
                let session = weak.upgrade().ok_or_else(|| {
                    Exception::throw_reference(&ctx, "native session is disposed")
                })?;
                session.request_activation_token_js(&ctx, options)
            }),
        )
    }

    fn request_activation_token_js<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        options: Opt<Value<'js>>,
    ) -> rquickjs::Result<Promise<'js>> {
        let app_id = parse_app_id(ctx, options)?;
        let grant = self.bridges.take_active_gesture_grant();
        let (promise, resolve, _reject) = ctx.promise()?;

        let allowed = self.mount_started.get()
            && !self.shutdown_started.get()
            && !self.staged.get()
            && !self.shell.cleanup_in_progress()
            && !self.shell.lock_is_retained()
            && !self.shell.has_lock_command()
            && self.dnd.active_count() == 0
            && self.activation.in_flight.borrow().len() < MAX_IN_FLIGHT_ACTIVATION_TOKENS;
        if !allowed {
            resolve.call::<_, ()>((Value::new_null(ctx.clone()),))?;
            return Ok(promise);
        }

        let Some((document, grant)) = grant else {
            resolve.call::<_, ()>((Value::new_null(ctx.clone()),))?;
            return Ok(promise);
        };
        let Some(region) = self.mount_region(document) else {
            resolve.call::<_, ()>((Value::new_null(ctx.clone()),))?;
            return Ok(promise);
        };
        if region.is_lock_surface()
            || region.surface != grant.surface()
            || region.bridge.is_disposed()
        {
            resolve.call::<_, ()>((Value::new_null(ctx.clone()),))?;
            return Ok(promise);
        }
        let Some(surface) = self.platform_surface(region.surface) else {
            resolve.call::<_, ()>((Value::new_null(ctx.clone()),))?;
            return Ok(promise);
        };
        let Some(timers) = self.activation.timers.borrow().clone() else {
            resolve.call::<_, ()>((Value::new_null(ctx.clone()),))?;
            return Ok(promise);
        };
        let Some(pending) = surface.request_activation_token(grant, app_id.as_deref()) else {
            resolve.call::<_, ()>((Value::new_null(ctx.clone()),))?;
            return Ok(promise);
        };

        let serial = pending.serial();
        let weak = Rc::downgrade(self);
        let Some(timer) = timers.schedule(
            ACTIVATION_TOKEN_TIMEOUT,
            Rc::new(move || {
                if let Some(session) = weak.upgrade() {
                    session.timeout_activation_token(serial);
                }
            }),
        ) else {
            pending.cancel();
            resolve.call::<_, ()>((Value::new_null(ctx.clone()),))?;
            return Ok(promise);
        };

        self.activation.in_flight.borrow_mut().insert(
            serial,
            InFlightToken {
                resolve: Persistent::save(ctx, resolve),
                pending,
                timer,
            },
        );
        Ok(promise)
    }

    #[must_use]
    pub(crate) fn owns_activation_token(&self, serial: ActivationTokenSerial) -> bool {
        !self.shutdown_started.get() && self.activation.owns(serial)
    }

    pub(crate) fn finish_activation_token(
        &self,
        serial: ActivationTokenSerial,
        token: ActivationToken,
    ) {
        let Some(entry) = self.activation.in_flight.borrow_mut().remove(&serial) else {
            return;
        };
        if let Some(timers) = self.activation.timers.borrow().as_ref() {
            timers.cancel(entry.timer);
        }
        if self.shutdown_started.get() {
            entry.pending.cancel();
            return;
        }
        let value = if self.staged.get()
            || self.shell.lock_is_retained()
            || self.shell.has_lock_command()
        {
            None
        } else {
            Some(token.into_string())
        };
        self.activation
            .ready
            .borrow_mut()
            .push_back((entry.resolve, value));
        self.wake_owner();
    }

    pub(crate) fn timeout_activation_token(&self, serial: ActivationTokenSerial) {
        let Some(entry) = self.activation.in_flight.borrow_mut().remove(&serial) else {
            return;
        };
        entry.pending.cancel();
        if self.shutdown_started.get() {
            return;
        }
        self.activation
            .ready
            .borrow_mut()
            .push_back((entry.resolve, None));
        self.wake_owner();
    }

    pub(crate) fn settle_next_activation_token(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        let Some((resolve, token)) = self.activation.ready.borrow_mut().pop_front() else {
            return Ok(());
        };
        let allowed = !self.staged.get()
            && !self.shell.lock_is_retained()
            && !self.shell.has_lock_command();
        match token.filter(|_| allowed) {
            Some(text) => {
                self.work.remember_external_activation(&text);
                resolve.restore(ctx)?.call::<_, ()>((text,))
            }
            None => resolve
                .restore(ctx)?
                .call::<_, ()>((Value::new_null(ctx.clone()),)),
        }
    }
}
