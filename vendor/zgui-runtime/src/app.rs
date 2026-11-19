//! The application: the platform handler, its windows, and how the loop parks.
//!
//! # Parking, which is the part that is easy to get wrong twice
//!
//! A loop that has nothing to do must consume nothing. A loop that has something to do *later*
//! must be woken then and not before. Those two together are the whole of the parking design, and
//! both ways of getting it wrong look identical from outside: nothing happens.
//!
//! * **The stall.** The loop computes a deadline, parks on it, is woken when it arrives — and
//!   nothing turns "the deadline arrived" into a request to draw. A timer fires no frame; an
//!   animation never advances. [`Runtime::deadline_reached`] is the edge that closes it.
//! * **The spin.** A deadline that has already passed is installed anyway. The platform recomputes
//!   the time remaining on every turn, finds none, reports it reached again, and the loop runs no
//!   frames while burning a core. [`Runtime::idle`] installs a deadline only when it is strictly in
//!   the future, and a deadline that has arrived is answered with a redraw request and a park on
//!   nothing at all.
//!
//! The ratio of resumes to frames is the only thing that separates the second from a correct park,
//! which is why the headless backend asserts on it after every turn.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use zgui_geom::{Css, CssPx, Size};
use zgui_platform::{
    AppHandler, Clock, IdlePolicy, LayerPlacement, MonitorInfo, PlatformCx, PlatformError,
    PopupPlacement, SessionLockState, Surface, SurfaceAttributes, SurfaceEvent, SurfaceId,
    SurfaceRole, WakeReason,
};
use zgui_render::{RenderTarget, Renderer};
use zgui_view::{Anchor, BuildCx};

use crate::app_timers::AppTimers;
use crate::commands::{CloseResponse, WindowCommand, WindowCommands, WindowSpec, WindowToken};
use crate::error::AppError;
use crate::text::{NoText, TextEngine};
use crate::timer::Timers;
use crate::wake::{FrameGate, RuntimeWaker};
use crate::window::{Window, WindowContent};

/// Builds the renderer one surface draws through.
///
/// It is a callback rather than a fixed type because the graphics backend is a decision an
/// application makes, and because the whole loop has to be exercisable with a renderer that draws
/// nowhere — which is not a fallback for a real window but a different thing entirely.
pub type RendererFactory =
    Box<dyn FnMut(&Arc<dyn Surface>, RenderTarget) -> Result<Box<dyn Renderer>, AppError>>;

/// Builds the text engine a window shapes with.
pub type TextFactory = Box<dyn FnMut() -> Box<dyn TextEngine>>;

/// Answers the cascade's font-metric questions.
pub type MetricsFactory = Box<dyn FnMut() -> Arc<dyn zgui_text::FontMetricsSource>>;

/// Turns the glyphs a window shaped into pixels.
///
/// Separate from the text engine because the two are used at different times by different code: a
/// shaper is held mutably while layout measures, and a rasteriser is read from the emit walk while
/// nothing may be mutated. Shared rather than owned, because one set of faces serves every window.
pub type RasterFactory = Box<dyn FnMut() -> Arc<dyn zgui_text::GlyphRaster>>;

/// Builds the view one window holds.
pub type ViewFactory = Box<dyn FnMut(&mut BuildCx<'_>) -> Box<dyn Anchor>>;

/// Builds the embed host one window fills its `surface` elements through.
///
/// Per window, like the renderer, because a host holds attachments against one window's renderer
/// and one window's replaced nodes. An application that never shows a surface element installs
/// none and pays nothing.
pub type EmbedFactory = Box<dyn FnMut() -> Box<dyn crate::embed::EmbedHost>>;

/// Builds the two halves of one window's custom-element registry, given its document.
///
/// The document is the argument because both halves resolve elements through it: which node
/// names which implementation is a property read, and a source built with no document would have
/// nothing to read it from.
pub type CustomFactory = Box<
    dyn FnMut(
        &Rc<RefCell<zgui_dom::Document>>,
    ) -> (
        Box<dyn zgui_layout::custom::CustomLayoutSource>,
        Box<dyn zgui_paint::content::custom::CustomPaintSource>,
    ),
>;

/// Where a runtime records why it stopped.
///
/// Shared, so that the answer survives the runtime being handed to a platform backend.
///
/// ```
/// use zgui_runtime::Failure;
///
/// let failure = Failure::default();
/// assert!(failure.take().is_none());
/// ```
#[derive(Clone, Debug, Default)]
pub struct Failure {
    /// What stopped the application, once something has.
    inner: Rc<RefCell<Option<AppError>>>,
}

impl Failure {
    /// Records `error`, unless something earlier already stopped the application.
    ///
    /// The first failure is the one worth reporting: everything after it is a consequence of a
    /// program that is already stopping.
    pub fn record(&self, error: AppError) {
        let mut slot = self.inner.borrow_mut();
        if slot.is_none() {
            *slot = Some(error);
        }
    }

    /// Takes what stopped the application, if anything did.
    pub fn take(&self) -> Option<AppError> {
        self.inner.borrow_mut().take()
    }
}

/// An application, before it runs.
///
/// ```
/// use zgui_runtime::App;
///
/// let app = App::new()
///     .with_title("counter")
///     .with_size(480.0, 320.0)
///     .with_stylesheet("root { display: block }");
/// assert_eq!(app.title(), "counter");
/// ```
pub struct App {
    /// What the window should be.
    attributes: SurfaceAttributes,
    /// What the window should hold.
    options: WindowContent,
    /// What draws it.
    renderer: Option<RendererFactory>,
    /// What shapes its text.
    text: Option<TextFactory>,
    /// What answers the cascade's font-metric questions.
    metrics: Option<MetricsFactory>,
    /// What turns its glyphs into pixels.
    raster: Option<RasterFactory>,
    /// What fills its `surface` elements.
    embed: Option<EmbedFactory>,
    /// What answers its custom elements.
    custom: Option<CustomFactory>,
    /// What to run in the scope above every window, before the first one opens.
    shared: Option<Box<dyn FnOnce()>>,
    /// When the application stops.
    exit: ExitPolicy,
    /// What builds the application menu, when the application has one.
    menu: Option<Box<dyn Fn() -> zgui_platform::AppMenu>>,
    /// How many workers the style cascade may use, when the default is not wanted.
    style_threads: Option<usize>,
    /// How many workers the layout engine may use, when the default is not wanted.
    layout_threads: Option<usize>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// An application with one resizable, decorated window and nothing in it.
    pub fn new() -> Self {
        Self {
            attributes: SurfaceAttributes::new("zgui"),
            options: WindowContent::default(),
            renderer: None,
            text: None,
            metrics: None,
            raster: None,
            embed: None,
            custom: None,
            shared: None,
            exit: ExitPolicy::default(),
            menu: None,
            style_threads: None,
            layout_threads: None,
        }
    }

    /// How many workers the layout engine may use.
    ///
    /// The default is the machine's parallelism; the engine clamps to what it benefits from.
    /// `0` and `1` mean layout runs on the frame thread with no pool at all. The
    /// `ZGUI_LAYOUT_THREADS` environment variable overrides whatever is set here.
    pub fn with_layout_threads(mut self, threads: usize) -> Self {
        self.layout_threads = Some(threads);
        self
    }

    /// How many workers the style cascade may use.
    ///
    /// The default is the machine's parallelism; the engine clamps to what it supports. `0` and
    /// `1` mean the cascade runs on the frame thread with no pool at all. The `ZGUI_STYLE_THREADS`
    /// environment variable overrides whatever is set here, so a run can be forced serial without
    /// a rebuild.
    pub fn with_style_threads(mut self, threads: usize) -> Self {
        self.style_threads = Some(threads);
        self
    }

    /// Returns the style worker count set with [`App::with_style_threads`], if one was set.
    pub fn style_threads(&self) -> Option<usize> {
        self.style_threads
    }

    /// Returns the layout worker count set with [`App::with_layout_threads`], if one was set.
    pub fn layout_threads(&self) -> Option<usize> {
        self.layout_threads
    }

    /// Runs `setup` in the scope above every window, before the first one opens.
    ///
    /// This is where state that belongs to the *application* rather than to a window goes. A
    /// context provided inside a window's view is that window's own — a second window resolves
    /// nothing for it — and one provided here is resolved by every window there will ever be.
    ///
    /// Signals need none of this: a signal belongs to no scope, so one made anywhere and moved into
    /// two views is read and written by both. What this is for is [`provide_context`] — the state
    /// a component reaches by type rather than by having been handed it.
    ///
    /// [`provide_context`]: zgui_reactive::provide_context
    pub fn with_context(mut self, setup: impl FnOnce() + 'static) -> Self {
        self.shared = Some(Box::new(setup));
        self
    }

    /// Decides when the application stops.
    ///
    /// The default stops it when the last window closes, which an application that opens one window
    /// cannot tell from any other policy. It matters as soon as there are several: see
    /// [`ExitPolicy`].
    pub fn with_exit_policy(mut self, exit: ExitPolicy) -> Self {
        self.exit = exit;
        self
    }

    /// Gives the application a menu outside its windows.
    ///
    /// `build` runs in the scope above every window, after [`App::with_context`], and runs again
    /// whenever a signal it read changes. Only a desktop with an application menu shows it; see
    /// [`PlatformCapabilities::app_menu`](zgui_platform::PlatformCapabilities::app_menu). What the
    /// user picks reaches [`on_menu`](crate::desktop::on_menu).
    pub fn with_menu(mut self, build: impl Fn() -> zgui_platform::AppMenu + 'static) -> Self {
        self.menu = Some(Box::new(build));
        self
    }

    /// The window's own attributes, for a caller building on this.
    pub fn attributes_mut(&mut self) -> &mut SurfaceAttributes {
        &mut self.attributes
    }

    /// The window's own options, for a caller building on this.
    pub fn options_mut(&mut self) -> &mut WindowContent {
        &mut self.options
    }

    /// The window's title.
    pub fn title(&self) -> &str {
        self.attributes.title.as_str()
    }

    /// Names the window.
    pub fn with_title(mut self, title: impl Into<zgui_vocab::SharedString>) -> Self {
        self.attributes.title = title.into();
        self
    }

    /// The identifier the desktop groups this application's windows under, if one was set.
    pub fn application_id(&self) -> Option<&str> {
        self.attributes
            .application_id
            .as_ref()
            .map(zgui_vocab::SharedString::as_str)
    }

    /// Names the application to the desktop.
    ///
    /// This is what a compositor matches window rules, icons and task-bar grouping against — the
    /// Wayland toplevel's `app_id` and the X11 window's `WM_CLASS`. It is not the title: the title
    /// is what a user reads and changes with the document, while this identifies the *program* and
    /// should match the `.desktop` file shipped with it, so the convention is a reverse-domain name
    /// such as `dev.example.Counter`.
    ///
    /// An application that sets none is one the desktop cannot address: it has no icon of its own,
    /// no window rules can select it, and its windows group under nothing.
    pub fn with_application_id(mut self, id: impl Into<zgui_vocab::SharedString>) -> Self {
        self.attributes.application_id = Some(id.into());
        self
    }

    /// Sets the size the window starts at, in CSS pixels.
    pub fn with_size(mut self, width: f32, height: f32) -> Self {
        self.attributes.size = Some(Size::<CssPx, Css>::new(CssPx(width), CssPx(height)));
        self
    }

    /// Installs `probe`, which is told about every frame of every window this opens.
    ///
    /// The seam a diagnostic tool attaches to; see [`FrameProbe`](crate::FrameProbe) for what it is
    /// handed and what it may do with it.
    pub fn with_probe(mut self, probe: std::rc::Rc<dyn crate::FrameProbe>) -> Self {
        self.options.probe = Some(probe);
        self
    }

    /// Adds a stylesheet of the application's own.
    ///
    /// Each call adds one sheet. The sheets cascade in call order.
    pub fn with_stylesheet(mut self, css: impl Into<String>) -> Self {
        self.options.stylesheets.push(css.into());
        self
    }

    /// Selects image source authority and publication policy before any window mounts.
    pub fn with_image_policy(mut self, policy: crate::ImagePolicy) -> Self {
        self.options.image_policy = policy;
        self
    }

    /// Installs what builds the renderer each window draws through.
    pub fn with_renderer(mut self, factory: RendererFactory) -> Self {
        self.renderer = Some(factory);
        self
    }

    /// Installs what builds the text engine each window shapes with.
    pub fn with_text_engine(mut self, factory: TextFactory) -> Self {
        self.text = Some(factory);
        self
    }

    /// Installs what answers the cascade's font-metric questions.
    pub fn with_metrics(mut self, factory: MetricsFactory) -> Self {
        self.metrics = Some(factory);
        self
    }

    /// Installs what fills each window's `surface` elements.
    pub fn with_embed(mut self, factory: EmbedFactory) -> Self {
        self.embed = Some(factory);
        self
    }

    /// Installs what answers each window's custom elements.
    pub fn with_custom(mut self, factory: CustomFactory) -> Self {
        self.custom = Some(factory);
        self
    }

    /// Installs what turns each window's glyphs into pixels.
    ///
    /// Without one a window lays its text out and draws no glyph at all, which is what an
    /// application that has chosen a shaper and not a rasteriser has asked for.
    pub fn with_glyph_raster(mut self, factory: RasterFactory) -> Self {
        self.raster = Some(factory);
        self
    }

    /// Turns the application into the handler a platform backend drives.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::ForeignExecutor`] when another asynchronous runtime already holds this
    /// process's executor slot, because the reactive layer's tasks have to run on the thread that
    /// owns the window.
    pub fn into_handler<V>(self, view: V) -> Result<Runtime, AppError>
    where
        V: FnMut(&mut BuildCx<'_>) -> Box<dyn Anchor> + 'static,
    {
        zgui_reactive::install()?;
        Ok(Runtime::new(self, Box::new(view)))
    }

    /// Runs the application on `driver` until the last window closes.
    ///
    /// `driver` is a platform backend's own entry point: it takes the handler and drives it. A
    /// windowing backend blocks until the loop finishes; a headless one may return at once.
    ///
    /// # Errors
    ///
    /// Returns whatever stopped it: a platform that could not do what was asked, a machine with no
    /// usable graphics device, or an executor slot already taken.
    pub fn run<V, D>(self, view: V, driver: D) -> Result<(), AppError>
    where
        V: FnMut(&mut BuildCx<'_>) -> Box<dyn Anchor> + 'static,
        D: FnOnce(Box<dyn AppHandler>) -> Result<(), PlatformError>,
    {
        let handler = self.into_handler(view)?;
        // Taken before the handler is handed over, because the driver consumes it. Without this a
        // machine with no usable graphics device runs a loop that opens nothing, draws nothing and
        // exits reporting success.
        let failure = handler.failure();
        let ran = driver(Box::new(handler));
        // The runtime is gone with the driver. Whatever is left in the task pool is dropped now,
        // while the thread's other state is alive.
        zgui_reactive::shutdown();
        ran?;
        failure.take().map_or(Ok(()), Err)
    }
}

/// When an application stops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExitPolicy {
    /// When the last window closes.
    ///
    /// What a document-based application wants, and what an application that never opens a second
    /// window cannot tell apart from any other policy.
    #[default]
    WhenAllWindowsClose,
    /// When the window the application launched with closes.
    ///
    /// For a main window whose palettes and inspectors have no life of their own.
    WhenPrimaryCloses,
    /// Only when the application says so.
    ///
    /// For an application that lives in a tray or a menu bar with no window open at all. Nothing
    /// stops it on its own, so it has to call [`WindowCommands::quit`] itself.
    Explicit,
}

/// One window the application wants, whether or not it is open.
struct LiveWindow {
    /// The name it is known by.
    token: WindowToken,
    /// What it should be.
    spec: Box<WindowSpec>,
    /// The surface it is open on, or `None` while it waits for one.
    surface: Option<SurfaceId>,
    /// The handle the application holds it by.
    handle: crate::windows::WindowHandle,
}

/// The running application, as the platform sees it.
pub struct Runtime {
    /// The application's own options, layered under every window's own.
    app_options: WindowContent,
    /// Every window the application wants, open or not, in creation order.
    live: Vec<LiveWindow>,
    /// The name of the window the application launched with.
    primary: Option<WindowToken>,
    /// The last inventory reported by the platform.
    monitors: Vec<MonitorInfo>,
    /// What the application has asked its own runtime to do.
    commands: WindowCommands,
    /// The desktop's clipboards, as the application asks for them.
    clipboards: crate::clipboard::Clipboards,
    /// The windows as the application holds them.
    windows_api: crate::windows::Windows,
    /// The scope above every window's, where what is shared between them lives.
    scope: zgui_reactive::Mounted,
    /// When the application stops.
    exit: ExitPolicy,
    /// What the application asked to hear about outside its windows.
    hooks: crate::desktop::AppHooks,
    /// The application menu, when the application has one.
    menu: Option<crate::desktop::MenuSlot>,
    /// The shortcuts of the menu's actions, shared with every window.
    menu_keys: crate::desktop::MenuKeys,
    /// The next document identity to mint.
    next_document: u16,
    /// What the desktop groups this application's windows under.
    ///
    /// Application-wide rather than per window: it is what a compositor matches window rules,
    /// icons and task-bar grouping against, and a window opened while the application runs belongs
    /// to the same application as the one it was opened from.
    application_id: Option<zgui_vocab::SharedString>,
    /// What builds a renderer.
    renderer: RendererFactory,
    /// What builds a text engine.
    text: TextFactory,
    /// What answers the cascade's font-metric questions.
    metrics: MetricsFactory,
    /// What turns glyphs into pixels.
    raster: RasterFactory,
    /// What builds an embed host, when the application brought one.
    embed: Option<EmbedFactory>,
    /// What builds a custom-element registry, when the application brought one.
    custom: Option<CustomFactory>,
    /// The windows that are open.
    windows: Vec<Window>,
    /// Whether a frame is in flight and what it owes.
    gate: Arc<FrameGate>,
    /// The scheduled callbacks of every window.
    timers: Rc<RefCell<Timers>>,
    /// Callbacks belonging to the application rather than one of its windows.
    app_timers: AppTimers,
    /// Where a wake from another thread goes.
    waker: Option<Arc<RuntimeWaker>>,
    /// The clock every phase reads.
    clock: Option<Arc<dyn Clock>>,
    /// The deadline each surface was parked on, as of the last park.
    ///
    /// Reaching a deadline says only *that* one was reached, never which. Recomputing the answer
    /// when the loop wakes gets it wrong for the deadline that is recomputed from `now` — an
    /// animation's next tick is always in the future, so a surface animating and nothing more
    /// would be woken every tick and asked to draw on none of them.
    parked: Vec<(
        SurfaceId,
        std::time::Instant,
        crate::window::frame::DeadlineKind,
    )>,
    /// Why the application stopped, if it did.
    failure: Failure,
    /// The cascade's worker pool, shared by every window.
    ///
    /// Its workers start on the first broad restyle of a document large enough to use them.
    /// `None` when the application asked for a serial cascade.
    style_pool: Option<Rc<StylePoolCell>>,
    /// The layout engine's worker pool, shared the same way.
    ///
    /// Its workers start on the first batch or pre-shape that layout distributes.
    layout_pool: Option<std::sync::Arc<zgui_layout::tree::parallel::LayoutPool>>,
}

/// The cascade's worker pool, started on first use.
pub(crate) struct StylePoolCell {
    /// How many workers the pool starts.
    width: usize,
    /// The pool, once a restyle asks for it.
    pool: std::cell::OnceCell<zgui_style::engine::thread_pool::StylePool>,
}

impl StylePoolCell {
    /// A cell for a pool of `width` workers, with no worker started.
    fn new(width: usize) -> Self {
        Self {
            width,
            pool: std::cell::OnceCell::new(),
        }
    }

    /// The pool, started on the first call.
    pub(crate) fn get(&self) -> &zgui_style::engine::thread_pool::StylePool {
        self.pool
            .get_or_init(|| zgui_style::engine::thread_pool::StylePool::new(self.width))
    }
}

/// The cascade pool an application runs with, from the environment or its builder.
///
/// `ZGUI_STYLE_THREADS` overrides the builder so a run can be forced serial, or to an exact
/// width, without a rebuild.
fn style_pool_for(requested: Option<usize>) -> Option<Rc<StylePoolCell>> {
    let width = style_pool_width(
        std::env::var("ZGUI_STYLE_THREADS").ok().as_deref(),
        requested,
        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get),
    );
    (width > 1).then(|| Rc::new(StylePoolCell::new(width)))
}

/// How wide the cascade pool should be, with the environment over the builder over the machine.
///
/// `0` and `1` mean no pool: a one-worker pool costs scheduling and buys nothing over the frame
/// thread. An unparseable environment value is ignored rather than honoured as zero, so a typo
/// degrades to the configured behaviour instead of silently forcing a serial cascade.
fn style_pool_width(env: Option<&str>, requested: Option<usize>, machine: usize) -> usize {
    env.and_then(|value| value.trim().parse::<usize>().ok())
        .or(requested)
        .unwrap_or(machine)
}

/// The extent a new window's renderer starts at, in device pixels.
///
/// A surface that the compositor has not configured yet has no extent. The extent the window
/// asked for, at the surface's current scale, is then the likeliest first extent, and a swap chain
/// configured at it is not configured again by the first frame. With no extent asked for, the
/// surface's own answer stays, and the first frame configures the swap chain again.
fn first_extent(
    surface: Size<zgui_geom::DevicePx, zgui_geom::Device>,
    asked: Option<Size<CssPx, Css>>,
    scale: f64,
) -> Size<i32, zgui_geom::Device> {
    let known = Size::new(surface.width.0 as i32, surface.height.0 as i32);
    match asked {
        Some(asked) if known.width < 1 || known.height < 1 => Size::new(
            (f64::from(asked.width.0) * scale).round() as i32,
            (f64::from(asked.height.0) * scale).round() as i32,
        ),
        _ => known,
    }
}

/// The layout pool an application runs with, resolved the same way as the cascade's.
fn layout_pool_for(
    requested: Option<usize>,
) -> Option<std::sync::Arc<zgui_layout::tree::parallel::LayoutPool>> {
    let width = style_pool_width(
        std::env::var("ZGUI_LAYOUT_THREADS").ok().as_deref(),
        requested,
        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get),
    );
    (width > 1).then(|| zgui_layout::tree::parallel::LayoutPool::new(width))
}

impl Runtime {
    /// Builds the handler for `app`.
    fn new(mut app: App, view: ViewFactory) -> Self {
        let style_pool = style_pool_for(app.style_threads);
        let layout_pool = layout_pool_for(app.layout_threads);
        let commands = WindowCommands::new();
        let clipboards = crate::clipboard::Clipboards::new();
        // The scope above every window: what is provided here is visible in all of them, and what a
        // window provides stays its own. It is created before any window so that each window's own
        // scope can be mounted under it.
        let scope = zgui_reactive::Mounted::new();
        let windows_api = crate::windows::Windows::new(commands.clone());
        let shared = app.shared.take();
        let hooks = crate::desktop::AppHooks::new();
        let build_menu = app.menu.take();
        let mut menu = None;
        scope.with(|| {
            zgui_reactive::provide_local_context(commands.clone());
            // How every window reaches the others, and how any of them opens one.
            zgui_reactive::provide_local_context(windows_api.clone());
            // The desktop's clipboards, reachable from every window.
            zgui_reactive::provide_local_context(clipboards.clone());
            // What the application hears from outside its windows.
            zgui_reactive::provide_local_context(hooks.clone());
            // Work spawned above the windows dies with the application rather than never.
            zgui_reactive::provide_task_set();
            // The application's own state, provided where every window can resolve it.
            if let Some(setup) = shared {
                setup();
            }
            // After the application's own state, which the menu is built from.
            menu = build_menu.map(crate::desktop::MenuSlot::new);
        });
        let application_id = app.attributes.application_id.clone();
        let primary = commands.mint();
        let live = vec![LiveWindow {
            token: primary,
            spec: Box::new(WindowSpec::new(
                app.attributes,
                WindowContent::default(),
                view,
            )),
            surface: None,
            // The window the application launched with is opened rather than asked for, and still
            // needs the handle every other window has.
            handle: crate::windows::WindowHandle::pending(
                primary,
                commands.clone(),
                Rc::clone(windows_api.capabilities_slot()),
            ),
        }];
        Self {
            app_options: app.options,
            live,
            primary: Some(primary),
            monitors: Vec::new(),
            commands,
            clipboards,
            windows_api,
            scope,
            exit: app.exit,
            hooks,
            menu,
            menu_keys: crate::desktop::MenuKeys::default(),
            next_document: 0,
            application_id: application_id.clone(),
            renderer: app.renderer.unwrap_or_else(|| {
                Box::new(|_, _| Err(AppError::GpuUnavailable(zgui_render::GpuUnavailable::new())))
            }),
            text: app
                .text
                .unwrap_or_else(|| Box::new(|| Box::new(NoText::new()))),
            metrics: app
                .metrics
                .unwrap_or_else(|| Box::new(|| Arc::new(zgui_text::FixedMetrics::new()))),
            raster: app
                .raster
                .unwrap_or_else(|| Box::new(|| Arc::new(zgui_text::NoRaster))),
            embed: app.embed,
            custom: app.custom,
            windows: Vec::new(),
            gate: Arc::new(FrameGate::new()),
            timers: Rc::new(RefCell::new(Timers::new())),
            app_timers: AppTimers::default(),
            waker: None,
            clock: None,
            style_pool,
            layout_pool,
            parked: Vec::new(),
            failure: Failure::default(),
        }
    }

    /// The windows that are open.
    pub fn windows(&self) -> &[Window] {
        &self.windows
    }

    /// The windows that are open, mutably, for a caller driving one by hand.
    pub fn windows_mut(&mut self) -> &mut [Window] {
        &mut self.windows
    }

    /// Starts without a primary window and stays alive until explicitly told to quit.
    ///
    /// # Panics
    ///
    /// Must be called before the runtime is launched.
    pub fn without_primary_surface(&mut self) {
        assert!(self.clock.is_none(), "the runtime has already been launched");
        if let Some(index) = self.primary.take().and_then(|token| self.live_index(token)) {
            self.discard_window(index);
        }
        self.exit = ExitPolicy::Explicit;
    }

    /// The monitor inventory from startup or the most recent platform notification.
    pub fn monitors(&self) -> &[MonitorInfo] {
        &self.monitors
    }

    /// Requests a compositor lock that does not depend on any window's lifetime.
    ///
    /// # Errors
    ///
    /// Returns the platform's error if it cannot start or retain a lock.
    pub fn request_session_lock(&self, cx: &dyn PlatformCx) -> Result<(), PlatformError> {
        cx.request_session_lock()
    }

    /// Reads the platform's latest compositor lock decision.
    #[must_use]
    pub fn session_lock_state(&self, cx: &dyn PlatformCx) -> SessionLockState {
        cx.session_lock_state()
    }

    /// Requests explicit unlock of an acquired lock; state reports compositor acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns the platform's error if the lock is not acquired or the request cannot be sent.
    pub fn unlock_session(&self, cx: &dyn PlatformCx) -> Result<(), PlatformError> {
        cx.unlock_session()
    }

    /// Schedules application work independently of frames and surface lifetime.
    pub fn app_timers(&self) -> AppTimers {
        self.app_timers.clone()
    }

    /// Delivers a surface event, calling `before_close` before this window is closed.
    ///
    /// A vetoed close request does not call the hook. Owners may use it to retire
    /// auxiliary windows while the closing window and its document are still available.
    pub fn surface_event_with_close(
        &mut self,
        cx: &dyn PlatformCx,
        surface: SurfaceId,
        event: SurfaceEvent,
        before_close: impl FnOnce(&mut Self),
    ) {
        let mut fired = Vec::new();
        match event {
            SurfaceEvent::RedrawRequested => {
                let clock = cx.clock();
                let now = clock.now();
                if let Some(window) = self.window_mut(surface) {
                    // Not every redraw is one this window asked for. A backend turns a configure
                    // into one on its own account, and a window that owes a reconfiguration the
                    // output could not yet show declines it — which is where the layout, the
                    // repaint and the swapchain rebuild for a size that is already superseded are
                    // saved. What is owed stays owed, and the deadline the park installs comes
                    // back for it.
                    if !window.wants_a_frame(now) {
                        window.declined_a_frame();
                    } else if window.holds_a_frame(now) {
                        // Not a refusal. The frame is worth running and is about to be: it is
                        // being started as late inside this interval as it can be and still
                        // finish, so that the picture it composes is as current as it can be when
                        // the display shows it. Everything that arrives in the meantime is queued
                        // as usual and drawn by that same frame.
                        window.held_a_frame();
                    } else {
                        window.frame(clock.as_ref());
                        fired = window.take_menu_actions();
                        // The copies first and the pastes second, so a copy and a paste from the
                        // same batch paste what was just copied rather than what preceded it.
                        Self::put_on_clipboard(cx, window);
                        Self::answer_pastes(cx, window);
                        self.sweep_other_windows(surface);
                    }
                }
            }
            SurfaceEvent::PresentReady => {
                if let Some(window) = self.window_mut(surface) {
                    window.present_held(cx.clock().as_ref());
                }
            }
            SurfaceEvent::CloseRequested => {
                // The user asking, which is the one close an application may refuse. Whatever
                // refuses owes the user another way out.
                let vetoed = self.live_for_surface(surface).is_some_and(|index| {
                    let callbacks = Rc::clone(&self.live[index].spec.close);
                    let asked = self.scope.with(|| {
                        // As a listener's body is: a callback that reads a signal is reading it,
                        // not subscribing whatever scope happens to be current.
                        let _zone = zgui_reactive::enter_non_reactive_zone();
                        callbacks.borrow_mut().ask()
                    });
                    asked == CloseResponse::Veto
                });
                if !vetoed {
                    before_close(self);
                    self.close_window(cx, surface);
                    self.apply_exit_policy(cx);
                }
            }
            // The platform took it. There is nothing to refuse: the window is already gone.
            SurfaceEvent::Destroyed => {
                before_close(self);
                self.close_window(cx, surface);
                self.apply_exit_policy(cx);
            }
            other => {
                if let Some(window) = self.window_mut(surface) {
                    // Every remaining event asks for a frame, for one of two reasons. Input is
                    // dispatched by the frame it asks for rather than where it arrives, because a
                    // handler's writes have to settle somewhere. Everything else — a resize, a
                    // scale change, an occlusion — changes what the next frame is drawn against.
                    // An event describing a state the window is already in — the same extent
                    // again, an occlusion that has not changed — asks for nothing.
                    if window.queue(other) {
                        window.request_frame();
                    }
                }
            }
        }
        // A chord nothing in the document claimed runs its menu action, after the frame.
        for id in fired {
            self.app_event(cx, zgui_platform::AppEvent::Menu(id));
        }
        // A frame just ran, and an effect inside it may have rebuilt the menu.
        self.apply_menu(cx);
        // A frame just ran, and a listener, an effect or a timer inside it may have asked for a
        // window. This is the first point since then that holds a platform context.
        if !self.commands.is_empty() {
            self.drain_window_commands(cx);
        }
        // The editor's own copies were put on the clipboard while the frame ran, so what the
        // application asked for lands after them: a component that copies and pastes in one batch
        // pastes what it just copied.
        if !self.clipboards.is_empty() {
            self.clipboards.drain(cx);
        }
    }

    /// Opens a secondary surface immediately, within an active platform callback.
    ///
    /// The surface and its window document are available before this returns.
    ///
    /// # Errors
    ///
    /// Returns an error when surfaces are unavailable or creation fails.
    pub fn open_surface(
        &mut self,
        cx: &dyn PlatformCx,
        spec: WindowSpec,
    ) -> Result<WindowToken, AppError> {
        let token = self.commands.mint();
        self.open_named_surface(cx, token, Box::new(spec))?;
        Ok(token)
    }

    /// Closes this window and its pop-up descendants, even if its own surface is suspended.
    pub fn close_surface(&mut self, cx: &dyn PlatformCx, token: WindowToken) {
        if self.close_named_surface(cx, token) {
            self.apply_exit_policy(cx);
        }
    }

    /// The current native identity of the window named by `token`, if it is open.
    pub fn surface_id(&self, token: WindowToken) -> Option<SurfaceId> {
        self.live[self.live_index(token)?].surface
    }

    /// Updates a layer's live placement and retains it for the next resume.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown window, a non-layer role, an output change, or a request
    /// refused by the backend.
    pub fn reconfigure_layer(
        &mut self,
        token: WindowToken,
        placement: LayerPlacement,
        size: Size<CssPx, Css>,
    ) -> Result<(), PlatformError> {
        let index = self.live_index(token).ok_or(PlatformError::NoSuchSurface)?;
        let live = &self.live[index];
        let SurfaceRole::Layer(current) = &live.spec.attributes.role else {
            return Err(zgui_platform::Unsupported.into());
        };
        if current.monitor != placement.monitor {
            return Err(PlatformError::Backend(
                "moving a layer surface to another output requires reopening it".to_owned(),
            ));
        }
        if let Some(surface) = live.surface {
            let window = self.window_mut(surface).ok_or(PlatformError::NoSuchSurface)?;
            let previous = window.surface().size();
            window.surface().reconfigure_layer(&placement, size)?;
            // Offscreen backends may apply the size synchronously without a configure event.
            let configured = window.surface().size();
            if configured != previous && window.queue(SurfaceEvent::Resized(configured)) {
                window.request_frame();
            }
        }
        let attrs = &mut self.live[index].spec.attributes;
        attrs.role = SurfaceRole::Layer(placement);
        attrs.size = Some(size);
        Ok(())
    }

    /// Updates a live pop-up's placement and requested size without changing its parent or grab.
    ///
    /// # Errors
    ///
    /// Returns an error for an absent or non-pop-up surface, a parent or grab change, or a
    /// request refused by the backend.
    pub fn reconfigure_popup(
        &mut self,
        token: WindowToken,
        placement: PopupPlacement,
        size: Size<CssPx, Css>,
    ) -> Result<(), PlatformError> {
        let index = self.live_index(token).ok_or(PlatformError::NoSuchSurface)?;
        let live = &self.live[index];
        let SurfaceRole::Popup(current) = &live.spec.attributes.role else {
            return Err(zgui_platform::Unsupported.into());
        };
        if current.parent != placement.parent || current.grab != placement.grab {
            return Err(PlatformError::Backend(
                "changing a pop-up's parent or grab requires reopening it".to_owned(),
            ));
        }
        let surface = live.surface.ok_or(PlatformError::NoSuchSurface)?;
        let window = self.window_mut(surface).ok_or(PlatformError::NoSuchSurface)?;
        let previous = window.surface().size();
        window.surface().reconfigure_popup(&placement, size)?;
        // Offscreen backends may apply the size synchronously without a configure event.
        let configured = window.surface().size();
        if configured != previous && window.queue(SurfaceEvent::Resized(configured)) {
            window.request_frame();
        }
        let attrs = &mut self.live[index].spec.attributes;
        attrs.role = SurfaceRole::Popup(placement);
        attrs.size = Some(size);
        Ok(())
    }

    /// The primary surface's creation attributes while it has no native surface.
    ///
    /// Available before the first `surfaces_available` and after `surfaces_lost`.
    /// Returns `None` while the primary surface is open or after its window was
    /// closed. This lets a downstream host configure roles the facade does not
    /// expose, without replacing the renderer, text engine or window lifecycle.
    pub fn primary_surface_attributes_mut(&mut self) -> Option<&mut SurfaceAttributes> {
        let token = self.primary?;
        let primary = self.live.iter_mut().find(|window| window.token == token)?;
        if primary.surface.is_some() {
            return None;
        }
        Some(&mut primary.spec.attributes)
    }

    /// The queue this runtime carries out window work from.
    ///
    /// What the public window API is built on, and what a test opens a second window through.
    pub fn window_commands(&self) -> WindowCommands {
        self.commands.clone()
    }

    /// The scope above every window, where what is shared between them lives.
    pub fn scope(&self) -> &zgui_reactive::Mounted {
        &self.scope
    }

    /// Mints the next document identity.
    ///
    /// Never reused. A node handle carries the document it was minted in, and state above the
    /// windows can outlive a window while still holding one: with reuse, such a handle would
    /// resolve inside an unrelated later document and pass every ownership assertion while doing
    /// it. Four thousand window-opens in one process is the honest limit that buys that, and
    /// reaching it fails loudly rather than aliasing.
    fn allocate_document(&mut self) -> Result<zgui_dom::DocumentId, AppError> {
        let id =
            zgui_dom::DocumentId::new(self.next_document).ok_or(AppError::DocumentsExhausted)?;
        self.next_document += 1;
        Ok(id)
    }

    /// The window `token` names, if it is one the application still wants.
    fn live_index(&self, token: WindowToken) -> Option<usize> {
        self.live.iter().position(|live| live.token == token)
    }

    /// The window drawing into `surface`, if the application still wants one.
    fn live_for_surface(&self, surface: SurfaceId) -> Option<usize> {
        self.live
            .iter()
            .position(|live| live.surface == Some(surface))
    }

    /// Where this runtime records why it stopped.
    ///
    /// A handle rather than an answer, because by the time a platform backend's driver returns the
    /// runtime it was given has been consumed by it, and whoever asked for the application to run
    /// still has to be told why it did not.
    pub fn failure(&self) -> Failure {
        self.failure.clone()
    }

    /// The window drawing into `surface`, if one is.
    fn window_mut(&mut self, surface: SurfaceId) -> Option<&mut Window> {
        self.windows
            .iter_mut()
            .find(|window| window.surface().id() == surface)
    }

    /// Opens the window `self.live[index]` describes, on `cx`.
    ///
    /// The one path a window is opened by, whether it is the window the application launched with,
    /// one it asked for later, or one being opened again after the platform took its surface away.
    fn open_live_window(&mut self, cx: &dyn PlatformCx, index: usize) -> Result<(), AppError> {
        let document = self.allocate_document()?;
        // A window opened while the application runs belongs to the same application as the one it
        // was opened from. Without this the desktop sees an unnamed window: no icon, no window
        // rule matches it, and it groups under nothing.
        if self.live[index].spec.attributes.application_id.is_none() {
            self.live[index]
                .spec
                .attributes
                .application_id
                .clone_from(&self.application_id);
        }
        let surface = cx.create_surface(&self.live[index].spec.attributes)?;
        // A predecessor identifies only this creation, not any subsequent resume.
        self.live[index].spec.attributes.replacing_lock = None;
        let scale = surface.scale_factor();
        let size = first_extent(surface.size(), self.live[index].spec.attributes.size, scale);
        let translucent = self.live[index].spec.attributes.transparent;
        let target = RenderTarget::new(size, zgui_geom::Scale::new(scale as f32));
        // A window asked to be transparent is composed translucent from its first frame: the
        // renderer picks a premultiplied presentation for it, and the paint gives its text ordinary
        // coverage.
        let target = if translucent {
            target.translucent()
        } else {
            target
        };
        let renderer = match (self.renderer)(&surface, target) {
            Ok(renderer) => renderer,
            Err(error) => {
                cx.destroy_surface(surface.id());
                return Err(error);
            }
        };
        // Before the view is built, not after: a view that asks for anything while it is being
        // built asks through a waker that has to already know which surface it belongs to.
        let waker = Arc::clone(
            self.waker
                .as_ref()
                .expect("the waker is installed before any window is opened"),
        );
        waker.owns(surface.id());

        let clock = self
            .clock
            .clone()
            .expect("the clock is installed before any window is opened");
        let options = self
            .app_options
            .layered_with(&self.live[index].spec.options);
        let text = (self.text)();
        let raster = (self.raster)();
        let metrics = (self.metrics)();
        let timers = Rc::clone(&self.timers);
        let handle = self.live[index].handle.clone();
        let close = Rc::clone(&self.live[index].spec.close);
        let view = &mut self.live[index].spec.view;
        // Under the application's scope, so that this window's own scope is a child of it: what the
        // application provides above the windows is then visible from inside every one of them.
        let mut window = self.scope.with(|| {
            Window::open(
                Arc::clone(&surface),
                document,
                renderer,
                text,
                raster,
                metrics,
                clock,
                timers,
                waker,
                &options,
                handle,
                close,
                |cx| view(cx),
            )
        });
        // Before the first frame, so the document is styled against the desktop's preference from
        // the first pixel rather than being laid out light and re-cascaded dark. A platform that
        // cannot be asked reports nothing, and nothing is not light: the window then keeps the
        // scheme it was opened with instead of being told it is in one.
        if let Some(scheme) = cx.color_scheme() {
            window.set_platform_color_scheme(scheme);
        }
        // What a detent of a wheel means here, asked of the backend rather than assumed: how far
        // one travels and which way it points are both properties of the desktop, and a constant
        // above this line is a wheel that is wrong on every machine but the one it was written on.
        window.set_scroll_settings(cx.scroll_settings());
        window.set_translucent(translucent);
        if matches!(self.live[index].spec.attributes.role, SurfaceRole::Lock(_)) {
            window.enable_lock_backdrop();
        }
        if let Some(pool) = &self.style_pool {
            window.set_style_pool(Rc::clone(pool));
        }
        if let Some(pool) = &self.layout_pool {
            window.set_layout_pool(std::sync::Arc::clone(pool));
        }
        if let Some(embed) = self.embed.as_mut() {
            window.install_embed_host(embed());
        }
        if let Some(custom) = self.custom.as_mut() {
            let (layout, paint) = custom(window.document());
            window.install_custom_sources(layout, paint);
        }
        window.set_menu_keys(self.menu_keys.clone());
        self.windows.push(window);
        self.live[index].surface = Some(surface.id());
        self.commands
            .note_opened(self.live[index].token, surface.id());
        // The handle can act on the window from here on, and what it reports is seeded from what
        // the surface actually turned out to be rather than from what was asked for.
        self.live[index].handle.attach(&surface);
        self.windows_api
            .note_opened(self.live[index].handle.clone());
        // A window that has just been built has everything to draw, and nothing else will ask.
        if let Some(window) = self.windows.last() {
            window.request_frame();
        }
        Ok(())
    }

    fn open_named_surface(
        &mut self,
        cx: &dyn PlatformCx,
        token: WindowToken,
        mut spec: Box<WindowSpec>,
    ) -> Result<(), AppError> {
        let handle = spec.handle.take().unwrap_or_else(|| {
            crate::windows::WindowHandle::pending(
                token,
                self.commands.clone(),
                Rc::clone(self.windows_api.capabilities_slot()),
            )
        });
        self.live.push(LiveWindow { token, spec, surface: None, handle });
        let index = self.live.len() - 1;
        let result = if self.waker.is_some() {
            self.open_live_window(cx, index)
        } else {
            Err(PlatformError::SurfaceCreation("surfaces are not available".to_owned()).into())
        };
        if result.is_err() {
            self.discard_window(index);
        }
        result
    }

    fn close_named_surface(&mut self, cx: &dyn PlatformCx, token: WindowToken) -> bool {
        let Some(index) = self.live_index(token) else { return false };
        if let Some(surface) = self.live[index].surface {
            self.close_window(cx, surface);
        } else {
            self.discard_window(index);
        }
        true
    }

    fn discard_window(&mut self, index: usize) {
        let live = self.live.remove(index);
        self.commands.note_closed(live.token);
        live.handle.note_closed();
        self.windows_api.note_closed(live.token);
    }

    /// Carries out everything the application has asked its runtime to do.
    ///
    /// Drained where a platform context is in hand and no window's frame is running: the end of a
    /// surface event, the end of a wake, and just after the surfaces first appear. The loop runs
    /// until the queue is empty rather than taking one command, so that a window whose view opens
    /// another window opens both.
    fn drain_window_commands(&mut self, cx: &dyn PlatformCx) {
        let mut closed_any = false;
        while let Some(command) = self.commands.pop() {
            match command {
                WindowCommand::Open { token, spec } => {
                    if let Err(error) = self.open_named_surface(cx, token, spec) {
                        tracing::error!(target: "zgui::app", %error, "the window could not be opened");
                    }
                }
                WindowCommand::Close(token) => closed_any |= self.close_named_surface(cx, token),
                WindowCommand::Quit => cx.request_exit(),
            }
        }
        if closed_any {
            self.apply_exit_policy(cx);
        }
    }

    /// Gives the desktop the version of the application menu it has not seen.
    fn apply_menu(&self, cx: &dyn PlatformCx) {
        let Some(slot) = &self.menu else { return };
        if !cx.capabilities().app_menu {
            return;
        }
        if let Some(menu) = slot.take() {
            cx.set_app_menu(&menu);
            self.menu_keys.follow(&menu);
        }
    }

    /// Carries out something the desktop asked of the application as a whole.
    fn app_event(&mut self, cx: &dyn PlatformCx, event: zgui_platform::AppEvent) {
        use zgui_platform::AppEvent;

        match event {
            AppEvent::Reopen => {
                let hooks = self.hooks.clone();
                self.scope.with(|| {
                    let _zone = zgui_reactive::enter_non_reactive_zone();
                    hooks.reopen();
                });
            }
            AppEvent::Menu(id) => {
                let hooks = self.hooks.clone();
                self.scope.with(|| {
                    let _zone = zgui_reactive::enter_non_reactive_zone();
                    hooks.menu(&id);
                });
            }
            // The focused window receives the press the menu took from it.
            AppEvent::Role(role) => {
                let Some(event) = crate::desktop::replay(role, cx.clock().timestamp()) else {
                    return;
                };
                if let Some(window) = self
                    .windows
                    .iter_mut()
                    .find(|window| window.is_surface_focused())
                    && window.queue(event)
                {
                    window.request_frame();
                }
            }
            _ => {}
        }
    }

    /// Stops the application when its policy says the windows that are left are not enough.
    fn apply_exit_policy(&self, cx: &dyn PlatformCx) {
        let done = match self.exit {
            ExitPolicy::WhenAllWindowsClose => self.live.is_empty(),
            ExitPolicy::WhenPrimaryCloses => {
                self.primary.is_none_or(|token| self.live_index(token).is_none())
            }
            ExitPolicy::Explicit => false,
        };
        if done {
            cx.request_exit();
        }
    }

    /// Writes whatever a cut or a copy asked for onto the platform's clipboard.
    ///
    /// Here rather than inside the frame because this is the only place that holds both: the window
    /// knows what was cut, and the platform context is what owns the clipboard. A failure is logged
    /// and not retried — the text has already left the field either way, and a queue that grew every
    /// time a write was refused would grow without bound.
    fn put_on_clipboard(cx: &dyn PlatformCx, window: &mut Window) {
        for text in window.take_clipboard() {
            let data = zgui_platform::ClipboardData::Text(text.into());
            if let Err(error) = cx.clipboard().write(
                zgui_platform::ClipboardKind::Standard,
                data,
                zgui_platform::ClipboardWriteOptions::default(),
            ) {
                tracing::warn!(target: "zgui::app", %error, "the clipboard refused a write");
            }
        }
    }

    /// Answers the paste requests a frame recorded, with the clipboard's text.
    ///
    /// The read is the blocking one, which the contract offers for exactly this caller: one
    /// already on the loop, on a desktop that ordinarily has the answer at once. A clipboard that
    /// is empty, refuses, or holds something other than text answers no paste at all — the field
    /// stays as it was, which is what pasting nothing does everywhere else on the desktop.
    fn answer_pastes(cx: &dyn PlatformCx, window: &mut Window) {
        let wanted = window.take_paste_requests();
        if wanted.is_empty() {
            return;
        }
        let text = match cx.clipboard().read_blocking(
            zgui_platform::ClipboardKind::Standard,
            zgui_platform::ClipboardFormat::Text,
        ) {
            Ok(data) => match data.as_text() {
                Some(text) => text.to_owned(),
                None => return,
            },
            Err(error) => {
                tracing::debug!(target: "zgui::app", %error, "the clipboard had nothing to paste");
                return;
            }
        };
        for node in wanted {
            window.paste(node, text.clone());
        }
        // The answer is applied by a frame, and nothing else knows one is owed.
        window.request_frame();
    }

    /// Closes the window drawing into `surface` and its pop-up descendants, for good.
    ///
    /// The window stops being one the application wants, which is what tells this apart from a
    /// platform taking every surface away: a window closed here does not come back on resume.
    fn close_window(&mut self, cx: &dyn PlatformCx, surface: SurfaceId) {
        // A pop-up is opened after its parent. Take a snapshot so removals cannot invalidate
        // indices, then go in reverse creation order to close each child before its parent.
        let mut descendants = Vec::new();
        for live in &self.live {
            if let SurfaceRole::Popup(placement) = &live.spec.attributes.role
                && let Some(id) = live.surface
                && (placement.parent == surface || descendants.contains(&placement.parent))
            {
                descendants.push(id);
            }
        }
        for id in descendants.into_iter().rev().chain(std::iter::once(surface)) {
            self.close_one_window(cx, id);
        }
    }

    fn close_one_window(&mut self, cx: &dyn PlatformCx, surface: SurfaceId) {
        // Looked up before the teardown, which is what clears the surface this names it by.
        let index = self.live_for_surface(surface);
        self.forget_surface(surface);
        // The window itself, after everything drawing into it has been dropped. Letting go of the
        // application's own half is not enough: what the backend holds is what keeps the window on
        // the screen, and a window nothing draws into is one that no longer responds or closes.
        cx.destroy_surface(surface);
        if let Some(index) = index {
            self.discard_window(index);
        }
    }

    /// Retires transient windows in reverse creation order, without closing their owners.
    fn retire_popups(&mut self, cx: &dyn PlatformCx) {
        for index in (0..self.live.len()).rev() {
            let live = &self.live[index];
            if !matches!(live.spec.attributes.role, SurfaceRole::Popup(_)) {
                continue;
            }
            if let Some(surface) = live.surface {
                self.close_one_window(cx, surface);
            } else {
                self.discard_window(index);
            }
        }
    }

    /// Tears down the window drawing into `surface`, leaving what the application wants alone.
    ///
    /// The half a suspend uses: the surface is gone, the window built on it cannot outlive it, and
    /// the specification that says the application wants it is untouched.
    fn forget_surface(&mut self, surface: SurfaceId) {
        if let Some(position) = self
            .windows
            .iter()
            .position(|window| window.surface().id() == surface)
        {
            self.windows[position].close();
            self.windows.remove(position);
        }
        if let Some(live) = self
            .live
            .iter_mut()
            .find(|live| live.surface == Some(surface))
        {
            live.surface = None;
            live.handle.detach();
            self.windows_api.note_closed(live.token);
        }
        if let Some(waker) = &self.waker {
            waker.disowns(surface);
        }
    }

    /// Asks every other window whether this frame wrote its document, and asks it to draw if so.
    ///
    /// The reactive flush is thread-wide: a frame in one window runs every effect that is ready,
    /// including effects that write another window's document. Nothing else would ever ask that
    /// window for the frame that shows what was written — its own wake was serviced by this frame's
    /// flush — so the check is made here, against the document's own revision, which moves for a
    /// write however it arrived. A window swept for nothing restyles nothing and draws nothing.
    fn sweep_other_windows(&self, drawn: SurfaceId) {
        for window in &self.windows {
            if window.surface().id() != drawn && window.owes_frame_for_document() {
                window.request_frame();
            }
        }
    }

    fn fire_app_timers(&mut self, cx: &dyn PlatformCx, now: std::time::Instant) {
        let due = self.app_timers.due(now);
        if due.is_empty() {
            return;
        }
        self.scope.with(|| {
            for callback in due {
                callback();
            }
        });
        self.drain_window_commands(cx);
        self.clipboards.drain(cx);
        self.apply_menu(cx);
    }
}

impl Drop for Runtime {
    /// Closes every window that is still open, then lets the tasks the windows left run to their
    /// end while the application's scope is alive.
    fn drop(&mut self) {
        self.app_timers.detach();
        for window in &mut self.windows {
            window.close();
        }
        self.windows.clear();
        zgui_reactive::flush();
    }
}

impl AppHandler for Runtime {
    fn surfaces_available(&mut self, cx: &dyn PlatformCx) {
        // Installed before anything else: a clock read from two places is two clocks, and the
        // wake edge has to exist before the first effect is created or work that becomes ready
        // between frames is queued with nothing to ask for the frame that would poll it.
        self.clock = Some(cx.clock());
        self.app_timers.attach(cx);
        self.monitors_changed(cx);
        let waker = Arc::new(RuntimeWaker::new(cx.waker(), Arc::clone(&self.gate)));
        zgui_reactive::set_frame_waker(Arc::clone(&waker) as Arc<dyn zgui_reactive::FrameWaker>);
        self.waker = Some(waker);
        self.commands.set_platform(cx.waker());
        self.clipboards.set_platform(cx.waker());
        *self.windows_api.capabilities_slot().borrow_mut() = cx.capabilities().clone();

        // Every window the application wants and has no surface for. At launch that is the window
        // it launched with; after a suspend it is all of them, which is why a specification is kept
        // rather than consumed — and why resuming needs no path of its own.
        for index in 0..self.live.len() {
            if self.live[index].surface.is_some() {
                continue;
            }
            if let Err(error) = self.open_live_window(cx, index) {
                tracing::error!(target: "zgui::app", %error, "the window could not be opened");
                // The window the application launched with failing to open is the application
                // failing: there is nothing left to run.
                if Some(self.live[index].token) == self.primary {
                    self.failure.record(error);
                    cx.request_exit();
                    return;
                }
            }
        }
        self.drain_window_commands(cx);
        self.apply_menu(cx);
    }

    fn surfaces_lost(&mut self, cx: &dyn PlatformCx) {
        // Pop-ups belong to these native parent surfaces, not to their replacement IDs after
        // resume. Retire them child-first; neither their anchors nor any grabs survive a suspend.
        self.retire_popups(cx);
        // Keep the other specifications so the next `surfaces_available` can reopen them.
        // Do not apply the exit policy merely because the application went into the background.
        let surfaces: Vec<_> = self
            .live
            .iter()
            .filter_map(|live| {
                Some((
                    live.surface?,
                    matches!(live.spec.attributes.role, SurfaceRole::Lock(_)),
                ))
            })
            .collect();
        for (id, lock) in surfaces {
            self.forget_surface(id);
            if lock {
                // A lock role can survive the window; tell the platform to restore its fallback.
                cx.destroy_surface(id);
            }
        }
        self.waker = None;
    }

    fn monitors_changed(&mut self, cx: &dyn PlatformCx) {
        self.monitors = cx.monitors();
    }

    fn surface_event(&mut self, cx: &dyn PlatformCx, surface: SurfaceId, event: SurfaceEvent) {
        self.surface_event_with_close(cx, surface, event, |_| {});
    }

    fn wake(&mut self, cx: &dyn PlatformCx, reason: WakeReason) {
        match reason {
            // Something asked for a window, or for the application to stop. Nothing else here can
            // carry that out: a surface is made from a platform context, and this is one.
            WakeReason::AppWork => {}
            WakeReason::ReactiveWork { surfaces } => {
                for id in surfaces.iter() {
                    if let Some(window) = self.window_mut(*id) {
                        window.request_frame();
                    }
                }
            }
            // The desktop-wide route, which a backend that has no per-surface notification uses.
            // The preference is *read* here and pushed into every window, because a frame alone
            // does nothing: the device a frame is styled against is built from the window's own
            // viewport, and a window never told the scheme moved rebuilds an identical device and
            // re-matches nothing.
            WakeReason::ColorSchemeChanged => {
                let scheme = cx.color_scheme();
                for window in &mut self.windows {
                    if let Some(scheme) = scheme {
                        window.set_platform_color_scheme(scheme);
                    }
                    window.request_frame();
                }
            }
            WakeReason::DeviceLost => {
                for window in &self.windows {
                    window.request_frame();
                }
            }
            // An assistive technology has just connected and is holding nothing. There is no dirty
            // check that can notice that, because nothing has changed — so the tree is built in
            // full, here, whether or not anything else in this frame would have asked for one.
            WakeReason::A11yTreeRequested(surface) => {
                if let Some(window) = self.window_mut(surface) {
                    window.publish_full_a11y_tree();
                }
            }
            // An action arrives naming a node and not a window, so the window is the one whose
            // document holds that node. Handing it to every window in turn is how a two-window
            // process routes one, and each answers whether the node was its own.
            WakeReason::A11yAction(request) => {
                let timestamp = cx.clock().timestamp();
                for window in &mut self.windows {
                    if window.apply_a11y_action(&request, timestamp) {
                        break;
                    }
                }
            }
            // A read the application started has been answered. The callback runs before the
            // drain below, so a read that asks for another read is carried out on this same turn.
            WakeReason::ClipboardRead { serial, result } => {
                self.clipboards.resolve(serial, result);
            }
            WakeReason::App(event) => self.app_event(cx, event),
            // A resize is where the state is read, and the last resize came before the change
            // was done.
            WakeReason::SurfaceStateChanged(surface) => {
                if let Some(window) = self.window_mut(surface) {
                    window.refresh_window_state();
                    window.request_frame();
                }
            }
            _ => {}
        }
        self.apply_menu(cx);
        // Every wake is a turn of the loop with a platform context in hand, which is what carrying
        // out a queued window needs. A wake that queued nothing drains nothing.
        if !self.commands.is_empty() {
            self.drain_window_commands(cx);
        }
        if !self.clipboards.is_empty() {
            self.clipboards.drain(cx);
        }
    }

    fn idle(&mut self, cx: &dyn PlatformCx) -> IdlePolicy {
        let now = cx.clock().now();
        self.fire_app_timers(cx, now);
        let mut policy = self.app_timers.deadline().map_or(IdlePolicy::Block, |deadline| {
            IdlePolicy::until(deadline, now)
        });
        self.parked.clear();
        for window in &mut self.windows {
            let Some(deadline) = window.scheduled_deadline(now) else {
                continue;
            };
            if deadline.at > now {
                self.parked
                    .push((window.surface().id(), deadline.at, deadline.kind));
                policy = policy.merge(IdlePolicy::BlockUntil(deadline.at));
            } else {
                // A deadline that has already passed is never installed. The platform recomputes
                // the time remaining on every turn, finds none, and reports it reached again for
                // ever — a loop that runs no frames while burning a core. The answer is the frame
                // it was asking for, now, and a park on nothing.
                match deadline.kind {
                    crate::window::frame::DeadlineKind::Render => window.request_frame(),
                    crate::window::frame::DeadlineKind::Maintenance => {
                        window.maintain(cx.clock().timestamp())
                    }
                }
            }
        }
        policy
    }

    fn deadline_reached(&mut self, cx: &dyn PlatformCx) {
        // The edge without which a parked deadline produces no frame at all. Reaching a deadline
        // draws nothing by itself: the loop wakes, this runs, and the surfaces whose deadline has
        // passed — and no others — are asked for the frame that services it.
        //
        // Which those are is read from what was parked on, never recomputed. What the park
        // installed is what the loop actually waited on, and a surface's own answer can have moved
        // in between — a frame that ran for something else in the same turn retires the moment that
        // was waited for, and re-deriving it here would find the *next* one and conclude that no
        // surface had reached anything.
        let now = cx.clock().now();
        self.fire_app_timers(cx, now);
        // Kept for the surfaces whose own deadline is later than the one the loop parked on: the
        // park is the global minimum, so a wake is not every surface's wake.
        let mut reached = Vec::new();
        self.parked.retain(|(surface, deadline, kind)| {
            if *deadline > now {
                return true;
            }
            reached.push((*surface, *kind));
            false
        });
        for (surface, kind) in reached {
            if let Some(window) = self.window_mut(surface) {
                match kind {
                    crate::window::frame::DeadlineKind::Render => window.request_frame(),
                    crate::window::frame::DeadlineKind::Maintenance => {
                        window.maintain(cx.clock().timestamp())
                    }
                }
            }
        }
    }

    fn shutting_down(&mut self, cx: &dyn PlatformCx) {
        // Before the windows close, so what the callbacks read is still there.
        let hooks = self.hooks.clone();
        self.scope.with(|| {
            let _zone = zgui_reactive::enter_non_reactive_zone();
            hooks.exit();
        });
        self.app_timers.detach();
        self.retire_popups(cx);
        for window in &mut self.windows {
            window.close();
        }
        self.windows.clear();
    }
}

#[cfg(test)]
mod tests {
    use zgui_geom::{CssPx, DevicePx, Size};

    use super::{first_extent, style_pool_width};

    #[test]
    fn an_unconfigured_surface_starts_at_the_extent_the_window_asked_for() {
        let unknown = Size::new(DevicePx(0.0), DevicePx(0.0));
        let asked = Some(Size::new(CssPx(1080.0), CssPx(720.0)));
        assert_eq!(first_extent(unknown, asked, 1.0), Size::new(1080, 720));
        assert_eq!(first_extent(unknown, asked, 1.5), Size::new(1620, 1080));
        // Nothing asked for: the surface's answer stays.
        assert_eq!(first_extent(unknown, None, 1.0), Size::new(0, 0));
        // A surface that knows its extent keeps it.
        let known = Size::new(DevicePx(1916.0), DevicePx(1051.0));
        assert_eq!(first_extent(known, asked, 1.0), Size::new(1916, 1051));
    }

    #[test]
    fn the_environment_overrides_the_builder_and_the_builder_the_machine() {
        assert_eq!(style_pool_width(Some("4"), Some(2), 8), 4);
        assert_eq!(style_pool_width(None, Some(2), 8), 2);
        assert_eq!(style_pool_width(None, None, 8), 8);
    }

    #[test]
    fn zero_and_one_mean_a_serial_cascade_and_a_typo_means_nothing() {
        // The widths that build no pool pass through for the caller to test.
        assert_eq!(style_pool_width(Some("0"), Some(4), 8), 0);
        assert_eq!(style_pool_width(Some("1"), None, 8), 1);
        assert_eq!(style_pool_width(Some("many"), Some(3), 8), 3);
        assert_eq!(style_pool_width(Some(" 2 "), None, 8), 2);
    }
}
