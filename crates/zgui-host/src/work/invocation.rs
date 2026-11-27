use super::{Work, Mode, MAX_CALLBACKS, data_property, endpoint_value, owner};
use crate::{assets::{AssetError, AssetSource, fixture::FixtureRuntime}, instance::Inbox, invocation::{Activation, ActivationInfo}};
use rquickjs::{Array, Ctx, Exception, Function, Object, Persistent, function::Func};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc, sync::Arc};

#[derive(Clone, Default)]
pub(crate) struct Launch {
    pub initial: Option<Rc<ActivationInfo>>,
    pub token: Option<Rc<RefCell<Option<String>>>>,
    pub inbox: Option<Arc<Inbox>>,
}

pub(super) struct Invocations {
    launch: Launch,
    fixture: Option<Rc<FixtureRuntime>>,
    handlers: RefCell<BTreeMap<u64, Persistent<Function<'static>>>>,
}

impl Invocations {
    pub(super) fn new(assets: &AssetSource, launch: Launch) -> Self {
        Self {
            launch,
            fixture: match assets {
                AssetSource::Fixture(fixture) => Some(Rc::clone(fixture)),
                AssetSource::Live(_) => None,
            },
            handlers: RefCell::default(),
        }
    }

    pub(super) fn len(&self) -> usize { self.handlers.borrow().len() }
    pub(super) fn clear(&self) { self.handlers.borrow_mut().clear(); }
    pub(super) fn pending(&self) -> usize {
        if let Some(fixture) = &self.fixture { fixture.pending_activations() }
        else { self.launch.inbox.as_ref().map_or(0, |inbox| inbox.len()) }
    }
}

impl Work {
    pub(super) fn install_invocations<'js>(self: &Rc<Self>, services: &Object<'js>) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        services.prop("getInitialActivation", data_property(Func::from(move |ctx: Ctx<'js>| {
            let work = owner(&weak, &ctx)?;
            work.require_creation(&ctx)?;
            if let Some(fixture) = &work.invocations.fixture {
                work.trace_invocation(&ctx, fixture, "getInitialActivation", work.id(&ctx)?)?;
            }
            let info = work.invocations.fixture.as_ref().and_then(|fixture| fixture.initial_activation())
                .or(work.invocations.launch.initial.as_deref());
            info_value(&ctx, info)
        })))?;
        let weak = Rc::downgrade(self);
        services.prop("onActivation", data_property(Func::from(move |ctx: Ctx<'js>, handler: Function<'js>| {
            let work = owner(&weak, &ctx)?;
            work.require_creation(&ctx)?;
            if work.invocations.len() >= MAX_CALLBACKS {
                return Err(Exception::throw_range(&ctx, "activation listener limit reached"));
            }
            let id = work.id(&ctx)?;
            let weak = Rc::downgrade(&work);
            let stop = Function::new(ctx.clone(), move || {
                if let Some(work) = weak.upgrade() {
                    work.invocations.handlers.borrow_mut().remove(&id);
                }
            })?;
            if let Some(fixture) = &work.invocations.fixture {
                work.trace_invocation(&ctx, fixture, "onActivation", id)?;
            }
            work.invocations.handlers.borrow_mut().insert(id, Persistent::save(&ctx, handler));
            Ok(stop)
        })))
    }

    fn trace_invocation(&self, ctx: &Ctx<'_>, fixture: &FixtureRuntime, method: &'static str, id: u64) -> rquickjs::Result<()> {
        fixture.activation_call(self.generation, id, method)
            .map_err(|error| self.asset_error(ctx, AssetError::Contract(error)))
    }

    pub(crate) fn has_pending_invocations(&self) -> bool {
        self.mode.get() == Mode::Mounted && self.invocations.pending() != 0
    }

    pub(crate) fn next_invocation(&self) -> Option<Activation> {
        if self.mode.get() != Mode::Mounted { return None; }
        if let Some(fixture) = &self.invocations.fixture {
            fixture.take_activation().map(|info| Activation { info, token: None })
        } else {
            self.invocations.launch.inbox.as_ref()?.pop()
        }
    }

    pub(crate) fn deliver_invocation<'js>(&self, ctx: &Ctx<'js>, info: &ActivationInfo) -> rquickjs::Result<()> {
        let handlers = &self.invocations.handlers;
        let Some((&last, _)) = handlers.borrow().last_key_value() else { return Ok(()); };
        let mut next = 0;
        // A callback may unsubscribe another callback or register a new one. Neither
        // a removed listener nor one added during delivery sees this invocation.
        loop {
            let Some((id, handler)) = handlers.borrow().range(next..=last).next()
                .map(|(&id, handler)| (id, handler.clone())) else { return Ok(()); };
            if let Some(fixture) = &self.invocations.fixture {
                fixture.activation_delivered(self.generation, id);
            }
            handler.restore(ctx)?.call::<_, ()>((info_value(ctx, Some(info))?,))?;
            if id == last { return Ok(()); }
            next = id + 1;
        }
    }
}

fn info_value<'js>(ctx: &Ctx<'js>, info: Option<&ActivationInfo>) -> rquickjs::Result<Object<'js>> {
    let value = Object::new(ctx.clone())?;
    let args = Array::new(ctx.clone())?;
    if let Some(info) = info {
        for (index, arg) in (0_u32..).zip(&info.args) {
            args.as_object().prop(index, data_property(arg.as_str()))?;
        }
    }
    value.prop("args", data_property(args))?;
    value.prop("cwd", data_property(endpoint_value(ctx, info.and_then(|info| info.cwd.as_deref()))?))?;
    Ok(value)
}
