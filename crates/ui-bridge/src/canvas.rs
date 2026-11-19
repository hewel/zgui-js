//! Session-owned canvas handles, vector bindings, and QuickJS command decoding.

use std::{collections::HashMap, f64::consts::TAU, rc::Rc};

use rquickjs::{
    Array, Class, Ctx, Exception, FromJs, JsLifetime, Object, Value,
    class::Trace,
    function::{Func, Opt},
    object::Property,
};
use zgui::{
    canvas::{Brush, CanvasToken, SceneHandle, Shape, ShapeBuilder, zgui_color::Color},
    scene::kurbo::{Affine, Arc, BezPath, Circle, PathEl, Point, Rect, RoundedRect, Shape as _},
    view::{DomHandle, NodeId, PropKey, PropValue},
    vocab::prop::drawing,
};

use crate::{Handle, UiBridge, bounded_array_len, host_error, owner};

// A scene holds native resources even when it is not bound to any node.
const MAX_SCENES: usize = 256;
// Bound per-update paint items and the allocation made from the command array.
const MAX_SHAPES: usize = 4096;
// Bound JS path decoding, even when a single command holds the entire path.
const MAX_PATH_SEGMENTS: usize = 16_384;
// Cap retained native geometry outside the JS heap, including generated path elements.
const MAX_RETAINED_SHAPES: usize = 8192;
const MAX_RETAINED_SEGMENTS: usize = 32_768;
// Use sub-unit curve error at normal scale; larger radii use a relative floor.
const PATH_TOLERANCE: f64 = 0.1;
// Leave room for view transforms and derived points in the painter's f32 coordinates.
const MAX_MAGNITUDE: f64 = 1.0e9;

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct CanvasLease {
    #[qjs(skip_trace)]
    token: CanvasToken,
}

struct Scene {
    handle: SceneHandle,
    nodes: HashMap<Handle, NodeId>,
    shape_count: usize,
    segment_count: usize,
}

struct Geometry {
    shapes: Vec<Shape>,
    segments: usize,
}

pub(super) struct CanvasScenes {
    dom: DomHandle,
    scenes: HashMap<CanvasToken, Scene>,
    bindings: HashMap<Handle, CanvasToken>,
    retained_shapes: usize,
    retained_segments: usize,
}

impl CanvasScenes {
    pub(super) fn new(dom: DomHandle) -> Self {
        Self {
            dom,
            scenes: HashMap::new(),
            bindings: HashMap::new(),
            retained_shapes: 0,
            retained_segments: 0,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.scenes.len()
    }

    fn require_scene(&self, ctx: &Ctx<'_>, token: CanvasToken) -> rquickjs::Result<()> {
        if self.scenes.contains_key(&token) {
            Ok(())
        } else {
            Err(unknown_scene(ctx))
        }
    }

    fn bind(
        &mut self,
        ctx: &Ctx<'_>,
        node: Handle,
        native: NodeId,
        token: Option<CanvasToken>,
    ) -> rquickjs::Result<()> {
        if let Some(token) = token {
            self.require_scene(ctx, token)?;
        }
        if self.bindings.get(&node).copied() == token {
            return Ok(());
        }
        self.unlink(node);
        if let Some(token) = token {
            let scene = self.scenes.get_mut(&token).unwrap_or_else(|| {
                panic!("validated canvas is missing: token={token:?}, node={node:?}")
            });
            scene.nodes.insert(node, native);
            self.bindings.insert(node, token);
            write_revision(&self.dom, native, token, scene.handle.revision());
            write_view(&self.dom, native, scene.handle.view());
        } else {
            clear_binding(&self.dom, native);
        }
        Ok(())
    }

    fn update(
        &mut self,
        ctx: &Ctx<'_>,
        token: CanvasToken,
        geometry: Option<Geometry>,
        transform: Option<Affine>,
    ) -> rquickjs::Result<()> {
        let scene = self
            .scenes
            .get_mut(&token)
            .ok_or_else(|| unknown_scene(ctx))?;
        if let Some(Geometry { shapes, segments }) = geometry {
            let retained_shapes = self.retained_shapes - scene.shape_count + shapes.len();
            let retained_segments = self.retained_segments - scene.segment_count + segments;
            if retained_shapes > MAX_RETAINED_SHAPES || retained_segments > MAX_RETAINED_SEGMENTS {
                return Err(Exception::throw_range(
                    ctx,
                    "retained canvas geometry limit reached",
                ));
            }
            self.retained_shapes = retained_shapes;
            self.retained_segments = retained_segments;
            scene.shape_count = shapes.len();
            scene.segment_count = segments;
            scene.handle.edit(|canvas| canvas.replace(shapes));
            if !scene.nodes.is_empty() {
                let revision = scene.handle.revision();
                for &node in scene.nodes.values() {
                    write_revision(&self.dom, node, token, revision);
                }
            }
        }
        if let Some(transform) = transform
            && scene.handle.set_transform(transform)
            && !scene.nodes.is_empty()
        {
            let view = scene.handle.view();
            for &node in scene.nodes.values() {
                write_view(&self.dom, node, view);
            }
        }
        Ok(())
    }

    fn dispose(&mut self, token: CanvasToken) {
        if let Some(scene) = self.scenes.remove(&token) {
            self.retained_shapes -= scene.shape_count;
            self.retained_segments -= scene.segment_count;
            for (handle, node) in scene.nodes {
                self.bindings.remove(&handle);
                clear_binding(&self.dom, node);
            }
        }
    }

    fn unlink(&mut self, node: Handle) {
        if let Some(token) = self.bindings.remove(&node) {
            self.scenes
                .get_mut(&token)
                .unwrap_or_else(|| {
                    panic!("bound vector has no live canvas: token={token:?}, node={node:?}")
                })
                .nodes
                .remove(&node);
        }
    }

    pub(super) fn remove_nodes(&mut self, nodes: &[Handle]) {
        for &node in nodes {
            self.unlink(node);
        }
    }

    pub(super) fn clear(&mut self) {
        self.bindings.clear();
        self.scenes.clear();
        self.retained_shapes = 0;
        self.retained_segments = 0;
    }
}

fn write_revision(dom: &DomHandle, node: NodeId, token: CanvasToken, revision: u64) {
    dom.set_property(
        node,
        PropKey::new(drawing::CANVAS),
        PropValue::Integer(drawing::canvas_value(token.0, revision)),
    );
}

fn write_view(dom: &DomHandle, node: NodeId, view: u64) {
    // The DOM decodes this signed slot as u64; preserve every revision bit.
    dom.set_property(
        node,
        PropKey::new(drawing::CANVAS_VIEW),
        PropValue::Integer(i64::from_ne_bytes(view.to_ne_bytes())),
    );
}

fn clear_binding(dom: &DomHandle, node: NodeId) {
    dom.set_property(node, PropKey::new(drawing::CANVAS), PropValue::Unset);
    dom.set_property(node, PropKey::new(drawing::CANVAS_VIEW), PropValue::Unset);
}

fn unknown_scene(ctx: &Ctx<'_>) -> rquickjs::Error {
    Exception::throw_reference(
        ctx,
        "canvas scene does not belong to this session or is disposed",
    )
}

pub(super) fn bind<'js>(
    bridge: &UiBridge,
    ctx: &Ctx<'js>,
    node: Handle,
    value: Value<'js>,
) -> rquickjs::Result<()> {
    let native = {
        let registry = bridge.registry.borrow();
        registry
            .require_vector(node)
            .map_err(|error| host_error(ctx, error))?;
        registry
            .native(node)
            .map_err(|error| host_error(ctx, error))?
    };
    let token = if value.is_null() || value.is_undefined() {
        None
    } else {
        let lease = Class::<CanvasLease>::from_js(ctx, value)
            .map_err(|_| Exception::throw_type(ctx, "canvas requires a native canvas scene"))?;
        Some(lease.borrow().token)
    };
    bridge
        .canvas_scenes
        .borrow_mut()
        .bind(ctx, node, native, token)
}

pub(super) fn install<'js>(bridge: &Rc<UiBridge>, api: &Object<'js>) -> rquickjs::Result<()> {
    let weak = Rc::downgrade(bridge);
    api.set(
        "createCanvasScene",
        Func::from(move |ctx: Ctx<'js>| {
            let bridge = owner(&weak, &ctx)?;
            bridge.require_creation(&ctx)?;
            if bridge.canvas_scenes.borrow().len() >= MAX_SCENES {
                return Err(Exception::throw_range(
                    &ctx,
                    "native canvas scene limit reached",
                ));
            }
            let handle = SceneHandle::new();
            let token = handle.token();
            if bridge.canvas_scenes.borrow().scenes.contains_key(&token) {
                return Err(Exception::throw_range(
                    &ctx,
                    "native canvas scene token is already live",
                ));
            }
            let lease = Class::instance(ctx.clone(), CanvasLease { token })?.into_inner();
            lease.prop("token", Property::from(token.0).enumerable())?;
            let update_owner = weak.clone();
            lease.prop(
                "update",
                Property::from(Func::from(
                    move |ctx: Ctx<'js>, commands: Value<'js>, transform: Opt<Value<'js>>| {
                        let bridge = owner(&update_owner, &ctx)?;
                        bridge.require_turn(&ctx)?;
                        bridge.canvas_scenes.borrow().require_scene(&ctx, token)?;
                        // JS getters may reenter; decode without borrowing bridge state and recheck ownership.
                        let geometry = parse_geometry(&ctx, commands)?;
                        let transform = parse_transform(&ctx, transform.0)?;
                        bridge
                            .canvas_scenes
                            .borrow_mut()
                            .update(&ctx, token, geometry, transform)
                    },
                ))
                .enumerable(),
            )?;
            let dispose_owner = weak.clone();
            lease.prop(
                "dispose",
                Property::from(Func::from(move |ctx: Ctx<'js>| {
                    if let Some(bridge) = dispose_owner.upgrade()
                        && bridge.canvas_scenes.borrow().scenes.contains_key(&token)
                    {
                        bridge.require_turn(&ctx)?;
                        bridge.canvas_scenes.borrow_mut().dispose(token);
                    }
                    Ok::<(), rquickjs::Error>(())
                }))
                .enumerable(),
            )?;
            bridge.canvas_scenes.borrow_mut().scenes.insert(
                token,
                Scene {
                    handle,
                    nodes: HashMap::new(),
                    shape_count: 0,
                    segment_count: 0,
                },
            );
            Ok::<_, rquickjs::Error>(lease)
        }),
    )
}

fn parse_geometry<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Option<Geometry>> {
    if value.is_null() {
        return Ok(None);
    }
    let array = value
        .into_array()
        .ok_or_else(|| Exception::throw_type(ctx, "canvas commands must be an array or null"))?;
    let len = bounded_array_len(ctx, &array, MAX_SHAPES, "too many canvas commands")?;
    let mut shapes = Vec::with_capacity(len);
    let mut segments = 0;
    let mut segments_left = MAX_PATH_SEGMENTS;
    for index in 0..len {
        let command = record(ctx, array.get(index)?, "canvas command must be an object")?;
        let shape = parse_shape(&command, &mut segments_left)?;
        segments += shape.path.elements().len();
        shapes.push(shape);
    }
    Ok(Some(Geometry { shapes, segments }))
}

fn parse_shape(command: &Object<'_>, segments_left: &mut usize) -> rquickjs::Result<Shape> {
    let ctx = command.ctx();
    let fill = command.get::<_, Value>("fill")?;
    let stroke = command.get::<_, Value>("stroke")?;
    if fill.is_undefined() && stroke.is_undefined() {
        return Err(Exception::throw_type(
            ctx,
            "canvas command requires a fill or stroke",
        ));
    }
    let kind: String = command.get("kind")?;
    let path = match kind.as_str() {
        "rect" => rect(command)?.to_path(PATH_TOLERANCE),
        "roundedRect" => {
            let rect = rect(command)?;
            let radius = positive(command, "radius")?.min(rect.width().min(rect.height()) * 0.5);
            RoundedRect::from_rect(rect, radius).to_path(curve_tolerance(radius))
        }
        "circle" => {
            let radius = positive(command, "radius")?;
            Circle::new(point(command, "x", "y")?, radius).to_path(curve_tolerance(radius))
        }
        "arc" => {
            let radius = positive(command, "radius")?;
            let start = number(ctx, command.get("startAngle")?, "startAngle")?.rem_euclid(TAU);
            let sweep = number(ctx, command.get("sweepAngle")?, "sweepAngle")?;
            if sweep == 0.0 || sweep.abs() > TAU {
                return Err(Exception::throw_range(
                    ctx,
                    "canvas arc sweep must be nonzero and at most one turn",
                ));
            }
            let mut path = Arc::new(
                point(command, "x", "y")?,
                (radius, radius),
                start,
                sweep,
                0.0,
            )
            .to_path(curve_tolerance(radius));
            if sweep.abs() == TAU {
                path.close_path();
            }
            path
        }
        "path" => parse_path(ctx, command.get("segments")?, segments_left)?,
        _ => {
            return Err(Exception::throw_type(
                ctx,
                "unsupported canvas command kind",
            ));
        }
    };
    let mut builder = ShapeBuilder::new(path);
    if !fill.is_undefined() {
        builder = builder.fill(brush(ctx, fill)?);
    }
    if !stroke.is_undefined() {
        let stroke = record(ctx, stroke, "canvas stroke must be an object")?;
        builder = builder.stroke(
            brush(ctx, stroke.get("color")?)?,
            positive(&stroke, "width")?,
        );
    }
    Ok(builder.build())
}

fn parse_path<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    segments_left: &mut usize,
) -> rquickjs::Result<BezPath> {
    let array = value
        .into_array()
        .ok_or_else(|| Exception::throw_type(ctx, "canvas path segments must be an array"))?;
    let len = bounded_array_len(ctx, &array, *segments_left, "too many canvas path segments")?;
    if len == 0 {
        return Err(Exception::throw_type(
            ctx,
            "canvas path requires a moveTo segment",
        ));
    }
    *segments_left -= len;
    let mut elements = Vec::with_capacity(len);
    let mut open = false;
    for index in 0..len {
        let segment = record(
            ctx,
            array.get(index)?,
            "canvas path segment must be an object",
        )?;
        let kind: String = segment.get("kind")?;
        let end = || point(&segment, "x", "y");
        let element = match kind.as_str() {
            "moveTo" => {
                open = true;
                PathEl::MoveTo(end()?)
            }
            _ if !open => {
                return Err(Exception::throw_type(
                    ctx,
                    "canvas subpath must begin with moveTo",
                ));
            }
            "lineTo" => PathEl::LineTo(end()?),
            "quadTo" => PathEl::QuadTo(point(&segment, "cx", "cy")?, end()?),
            "cubicTo" => PathEl::CurveTo(
                point(&segment, "c1x", "c1y")?,
                point(&segment, "c2x", "c2y")?,
                end()?,
            ),
            "close" => {
                open = false;
                PathEl::ClosePath
            }
            _ => {
                return Err(Exception::throw_type(
                    ctx,
                    "unsupported canvas path segment kind",
                ));
            }
        };
        elements.push(element);
    }
    Ok(BezPath::from_vec(elements))
}

fn parse_transform<'js>(
    ctx: &Ctx<'js>,
    value: Option<Value<'js>>,
) -> rquickjs::Result<Option<Affine>> {
    let Some(value) = value.filter(|value| !value.is_undefined()) else {
        return Ok(None);
    };
    let array: Array<'js> = value
        .into_array()
        .ok_or_else(|| Exception::throw_type(ctx, "canvas transform must be a six-number array"))?;
    let mut coefficients = [0.0; 6];
    let error = "canvas transform must have six coefficients";
    if bounded_array_len(ctx, &array, coefficients.len(), error)? != coefficients.len() {
        return Err(Exception::throw_range(ctx, error));
    }
    for (index, coefficient) in coefficients.iter_mut().enumerate() {
        *coefficient = number(ctx, array.get(index)?, "transform coefficient")?;
    }
    Ok(Some(Affine::new(coefficients)))
}

fn record<'js>(ctx: &Ctx<'js>, value: Value<'js>, error: &str) -> rquickjs::Result<Object<'js>> {
    if value.is_array() || value.is_function() {
        return Err(Exception::throw_type(ctx, error));
    }
    value
        .into_object()
        .ok_or_else(|| Exception::throw_type(ctx, error))
}

fn number(ctx: &Ctx<'_>, value: Value<'_>, field: &str) -> rquickjs::Result<f64> {
    value
        .as_number()
        .filter(|value| value.abs() <= MAX_MAGNITUDE)
        .ok_or_else(|| {
            Exception::throw_type(
                ctx,
                &format!(
                    "canvas {field} must be a finite number with magnitude at most {MAX_MAGNITUDE}"
                ),
            )
        })
}

fn positive(record: &Object<'_>, field: &str) -> rquickjs::Result<f64> {
    let value = number(record.ctx(), record.get(field)?, field)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(Exception::throw_range(
            record.ctx(),
            &format!("canvas {field} must be positive"),
        ))
    }
}

fn point(record: &Object<'_>, x: &str, y: &str) -> rquickjs::Result<Point> {
    let ctx = record.ctx();
    Ok(Point::new(
        number(ctx, record.get(x)?, x)?,
        number(ctx, record.get(y)?, y)?,
    ))
}

fn rect(record: &Object<'_>) -> rquickjs::Result<Rect> {
    Ok(Rect::from_origin_size(
        point(record, "x", "y")?,
        (positive(record, "width")?, positive(record, "height")?),
    ))
}

fn curve_tolerance(radius: f64) -> f64 {
    PATH_TOLERANCE.max(radius * 1e-6)
}

fn brush<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Brush> {
    let invalid = || {
        Exception::throw_type(
            ctx,
            "canvas colors must use #rgb, #rgba, #rrggbb, or #rrggbbaa",
        )
    };
    let text = match value.into_string().ok_or_else(invalid)?.to_string() {
        Ok(text) => text,
        Err(rquickjs::Error::Utf8(_)) => return Err(invalid()),
        Err(error) => return Err(error),
    };
    let hex = text.as_str().strip_prefix('#').ok_or_else(invalid)?;
    if !matches!(hex.len(), 3 | 4 | 6 | 8) || hex.starts_with('+') {
        return Err(invalid());
    }
    let bits = u32::from_str_radix(hex, 16).map_err(|_| invalid())?;
    let short = |shift: u32| ((bits >> shift) as u8 & 0xf) * 17;
    let byte = |shift: u32| (bits >> shift) as u8;
    let [r, g, b, a] = match hex.len() {
        3 => [short(8), short(4), short(0), 255],
        4 => [short(12), short(8), short(4), short(0)],
        6 => [byte(16), byte(8), byte(0), 255],
        _ => [byte(24), byte(16), byte(8), byte(0)],
    };
    Ok(Brush::Solid(Color::srgb_u8(r, g, b, a)))
}
