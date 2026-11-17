use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rquickjs::{Ctx, Exception, Function, Object, Persistent};
use zgui::view::{DomHandle, HostHandle, NodeId, Observed, ObservedValue};

use crate::registry::Handle;

pub(crate) type Dispatcher = dyn Fn(u64, ObservedValue, f32);

struct Observation {
    owner: u64,
    callback: Persistent<Function<'static>>,
    #[allow(dead_code)]
    lease: zgui::view::ObservationHandle,
    last: Option<Measurement>,
    viewport: bool,
}

pub(crate) struct Geometry {
    dom: DomHandle,
    root: NodeId,
    host: HostHandle,
    observations: HashMap<u64, Observation>,
    next: u64,
    limit: usize,
}

pub(crate) enum Quantity {
    Node(Observed),
    Viewport,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Measurement {
    Rect { x: f64, y: f64, width: f64, height: f64 },
    Size { width: f64, height: f64 },
    Scroll { x: f64, y: f64, content_width: f64, content_height: f64, port_width: f64, port_height: f64 },
    Viewport { x: f64, y: f64, width: f64, height: f64, scale: f64 },
}

impl Geometry {
    pub(crate) fn new(dom: DomHandle, root: NodeId, host: HostHandle, limit: usize) -> Self {
        Self { dom, root, host, observations: HashMap::new(), next: 1, limit }
    }

    pub(crate) fn reserve(&mut self, ctx: &Ctx<'_>) -> rquickjs::Result<u64> {
        if self.observations.len() >= self.limit {
            return Err(Exception::throw_range(ctx, "native geometry observation limit reached"));
        }
        let id = self.next;
        self.next = id.checked_add(1).ok_or_else(|| Exception::throw_range(ctx, "native geometry observation identifiers exhausted"))?;
        Ok(id)
    }

    pub(crate) fn kind(ctx: &Ctx<'_>, name: &str) -> rquickjs::Result<Quantity> {
        match name {
            "borderBox" => Ok(Quantity::Node(Observed::BorderBox)),
            "borderSize" => Ok(Quantity::Node(Observed::BorderSize)),
            "contentSize" => Ok(Quantity::Node(Observed::ContentSize)),
            "scrollPosition" => Ok(Quantity::Node(Observed::ScrollPosition)),
            "viewport" => Ok(Quantity::Viewport),
            _ => Err(Exception::throw_type(ctx, "unsupported native geometry quantity")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        id: u64,
        owner: Handle,
        native: NodeId,
        what: Quantity,
        callback: Function<'js>,
        dispatcher: Rc<Dispatcher>,
    ) {
        let (native, what, viewport) = match what {
            Quantity::Node(what) => (native, what, false),
            Quantity::Viewport => (self.dom.root(self.root), Observed::BorderBox, true),
        };
        let host = self.host.clone();
        let lease = self.dom.observe(native, what, Rc::new(move |value| dispatcher(id, value, host.scale())));
        self.observations.insert(id, Observation {
            owner: owner.token(),
            callback: Persistent::save(ctx, callback),
            lease,
            last: None,
            viewport,
        });
    }

    pub(crate) fn prepare<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        id: u64,
        value: ObservedValue,
        scale: f32,
    ) -> rquickjs::Result<Option<(Persistent<Function<'static>>, Measurement)>> {
        let Some(record) = self.observations.get_mut(&id) else { return Ok(None); };
        let measurement = Measurement::of(ctx, value, scale)?;
        let measurement = if record.viewport {
            match measurement {
                Measurement::Rect { x, y, width, height } => Measurement::Viewport {
                    x, y, width, height, scale: f64::from(scale),
                },
                _ => return Err(Exception::throw_type(ctx, "unsupported native viewport sample")),
            }
        } else { measurement };
        if record.last == Some(measurement) { return Ok(None); }
        record.last = Some(measurement);
        Ok(Some((record.callback.clone(), measurement)))
    }

    pub(crate) fn cancel(&mut self, id: u64) {
        self.observations.remove(&id);
    }

    pub(crate) fn remove_nodes(&mut self, invalid: &[Handle]) {
        if invalid.is_empty() || self.observations.is_empty() { return; }
        let tokens: HashSet<u64> = invalid.iter().map(|handle| handle.token()).collect();
        self.observations.retain(|_, record| !tokens.contains(&record.owner));
    }

    pub(crate) fn clear(&mut self) {
        self.observations.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.observations.len()
    }
}

impl Drop for Geometry {
    fn drop(&mut self) { self.clear(); }
}

impl Measurement {
    fn of(ctx: &Ctx<'_>, value: ObservedValue, scale: f32) -> rquickjs::Result<Self> {
        let scale = f64::from(scale);
        if !scale.is_finite() || scale <= 0.0 {
            return Err(Exception::throw_range(ctx, "native window scale must be finite and positive"));
        }
        let css = |component: f32| -> rquickjs::Result<f64> {
            let value = f64::from(component) / scale;
            if !value.is_finite() {
                return Err(Exception::throw_range(ctx, "native geometry must be finite"));
            }
            Ok(value)
        };
        match value {
            ObservedValue::BorderBox(rect) => Ok(Self::Rect {
                x: css(rect.origin.x.0)?,
                y: css(rect.origin.y.0)?,
                width: css(rect.size.width.0)?,
                height: css(rect.size.height.0)?,
            }),
            ObservedValue::BorderSize(size) | ObservedValue::ContentSize(size) => Ok(Self::Size {
                width: css(size.width.0)?,
                height: css(size.height.0)?,
            }),
            ObservedValue::ScrollPosition(position) => Ok(Self::Scroll {
                x: css(position.offset.x.0)?,
                y: css(position.offset.y.0)?,
                content_width: css(position.content_size.width.0)?,
                content_height: css(position.content_size.height.0)?,
                port_width: css(position.scrollport.width.0)?,
                port_height: css(position.scrollport.height.0)?,
            }),
            _ => Err(Exception::throw_type(ctx, "unsupported native geometry sample")),
        }
    }

    pub(crate) fn into_js<'js>(self, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        let object = Object::new(ctx.clone())?;
        match self {
            Self::Rect { x, y, width, height } | Self::Viewport { x, y, width, height, .. } => {
                object.set("x", x)?;
                object.set("y", y)?;
                object.set("width", width)?;
                object.set("height", height)?;
            }
            Self::Size { width, height } => {
                object.set("width", width)?;
                object.set("height", height)?;
            }
            Self::Scroll { x, y, content_width, content_height, port_width, port_height } => {
                let offset = Object::new(ctx.clone())?;
                offset.set("x", x)?;
                offset.set("y", y)?;
                object.set("offset", offset)?;
                let content = Object::new(ctx.clone())?;
                content.set("width", content_width)?;
                content.set("height", content_height)?;
                object.set("contentSize", content)?;
                let port = Object::new(ctx.clone())?;
                port.set("width", port_width)?;
                port.set("height", port_height)?;
                object.set("scrollport", port)?;
            }
        }
        if let Self::Viewport { scale, .. } = self { object.set("scale", scale)?; }
        Ok(object)
    }
}
