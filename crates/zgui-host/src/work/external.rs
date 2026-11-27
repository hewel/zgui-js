//! Session-owned external endpoints and their bounded Bridge-turn deliveries.
use super::{ImageLease, Mode, Work, data_property, endpoint_value, owner, service_error};
use crate::{
    assets::{
        AssetError, AssetSource,
        fixture::{ExternalDelivery, FixtureRuntime},
    },
    package::Package,
};
use host_services::{
    ServiceError, command,
    control::{Inbound, MAX_REPLY_BYTES},
    unix_socket::{self, Framing},
};
use rquickjs::{
    Array, Ctx, Exception, Function, Object, Persistent, Value,
    function::{Func, Opt},
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    ops::Bound::{Excluded, Unbounded},
    os::unix::net::SocketAddr,
    rc::Rc,
    sync::Arc,
};

const MAX_SOCKETS: usize = 16;
const MAX_PROCESSES: usize = 32;
const MAX_CONTROL_QUEUE: usize = 16;
const MAX_COMMAND_ARGS: u32 = 64;
const MAX_ARG_BYTES: usize = 64 * 1024;
const MAX_PROGRAM_BYTES: usize = 4096;
const MAX_SOCKET_IMAGES: usize = 64;
const MAX_SOCKET_IMAGE_BYTES: usize = 64 * 1024 * 1024;

type Callback = Persistent<Function<'static>>;

enum Endpoint<T> {
    Live(T),
    Fixture,
}

enum CommandCallbacks {
    Buffered {
        resolve: Callback,
        reject: Callback,
    },
    Streaming {
        stdout: Option<Callback>,
        stderr: Option<Callback>,
        exit: Option<Callback>,
        error: Option<Callback>,
    },
}

enum Registration {
    Socket {
        endpoint: Endpoint<unix_socket::Socket>,
        framing: Framing,
        message: Callback,
        image: Option<Callback>,
        frame_stream: Option<Callback>,
        close: Option<Callback>,
    },
    Command {
        endpoint: Endpoint<command::Process>,
        callbacks: CommandCallbacks,
    },
    Control(Callback),
}

/// One delivery selected on the owner thread for the next external service turn.
pub(crate) enum Event {
    Socket(u64, unix_socket::Event),
    Frame(u64, crate::frame_stream::SyntheticFrame),
    Command(u64, command::Event),
    LiveControl(u64, Inbound),
    FixtureControl {
        listener: u64,
        request: u64,
        args: Vec<String>,
    },
}

impl Event {
    fn registration(&self) -> u64 {
        match self {
            Self::Socket(id, _)
            | Self::Frame(id, _)
            | Self::Command(id, _)
            | Self::LiveControl(id, _) => *id,
            Self::FixtureControl { listener, .. } => *listener,
        }
    }
}

/// Registrations that still own an endpoint or a control handler.
#[derive(Default)]
pub(crate) struct Counts {
    /// Unix socket connections.
    pub(crate) sockets: usize,
    /// Running command groups.
    pub(crate) processes: usize,
    /// Registered inbound control handlers.
    pub(crate) listeners: usize,
}

#[derive(Clone, Copy, Default)]
struct SocketImageBudget {
    handles: usize,
    bytes: usize,
}

pub(super) struct External {
    package: Option<Rc<Package>>,
    capabilities: Arc<crate::package::Capabilities>,
    fixture: Option<Rc<FixtureRuntime>>,
    registrations: RefCell<BTreeMap<u64, Registration>>,
    cursor: Cell<u64>,
    ready: RefCell<Option<Event>>,
    control: RefCell<VecDeque<(u64, Inbound)>>,
    images: Cell<SocketImageBudget>,
}

impl External {
    pub(super) fn new(
        package: Option<Rc<Package>>,
        capabilities: Arc<crate::package::Capabilities>,
        source: &AssetSource,
    ) -> Self {
        Self {
            package,
            capabilities,
            fixture: match source {
                AssetSource::Fixture(fixture) => Some(fixture.clone()),
                AssetSource::Live(_) => None,
            },
            registrations: RefCell::new(BTreeMap::new()),
            cursor: Cell::new(0),
            ready: RefCell::new(None),
            control: RefCell::new(VecDeque::new()),
            images: Cell::default(),
        }
    }

    fn can_receive_image(&self, bytes: usize) -> bool {
        let budget = self.images.get();
        budget.handles < MAX_SOCKET_IMAGES && bytes <= MAX_SOCKET_IMAGE_BYTES - budget.bytes
    }

    pub(super) fn charge_image(&self, bytes: usize) {
        let mut budget = self.images.get();
        budget.handles += 1;
        budget.bytes += bytes;
        self.images.set(budget);
    }

    pub(super) fn release_image(&self, bytes: usize) {
        let mut budget = self.images.get();
        budget.handles -= 1;
        budget.bytes -= bytes;
        self.images.set(budget);
    }

    fn listener(&self) -> Option<u64> {
        self.registrations
            .borrow()
            .iter()
            .find_map(|(&id, registration)| {
                matches!(registration, Registration::Control(_)).then_some(id)
            })
    }

    fn take(&self, generation: u64) -> Option<Event> {
        if let Some(fixture) = &self.fixture {
            return fixture
                .take_external(generation)
                .map(|delivery| match delivery {
                    ExternalDelivery::Socket(id, event) => Event::Socket(id, event),
                    ExternalDelivery::Frame(id, frame) => Event::Frame(id, frame),
                    ExternalDelivery::Command(id, event) => Event::Command(id, event),
                    ExternalDelivery::Control {
                        listener,
                        request,
                        args,
                    } => Event::FixtureControl {
                        listener,
                        request,
                        args,
                    },
                });
        }
        let slots = self.registrations.borrow();
        let cursor = self.cursor.get();
        for (&id, registration) in slots
            .range((Excluded(cursor), Unbounded))
            .chain(slots.range(..=cursor))
        {
            let event = match registration {
                Registration::Socket {
                    endpoint: Endpoint::Live(socket),
                    ..
                } => socket.try_recv().map(|event| Event::Socket(id, event)),
                Registration::Command {
                    endpoint: Endpoint::Live(process),
                    ..
                } => process.try_recv().map(|event| Event::Command(id, event)),
                Registration::Control(_) => self
                    .control
                    .borrow_mut()
                    .pop_front()
                    .map(|(id, inbound)| Event::LiveControl(id, inbound)),
                _ => None,
            };
            if event.is_some() {
                self.cursor.set(id);
                return event;
            }
        }
        None
    }

    pub(super) fn pending_count(&self, generation: u64) -> usize {
        usize::from(self.ready.borrow().is_some())
            + self.control.borrow().len()
            + self
                .fixture
                .as_ref()
                .map_or(0, |fixture| fixture.pending_external_deliveries(generation))
    }

    pub(super) fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for registration in self.registrations.borrow().values() {
            match registration {
                Registration::Socket {
                    endpoint: Endpoint::Live(socket),
                    ..
                } if !socket.is_active() => {}
                Registration::Command {
                    endpoint: Endpoint::Live(process),
                    ..
                } if !process.is_active() => {}
                Registration::Socket { .. } => counts.sockets += 1,
                Registration::Command { .. } => counts.processes += 1,
                Registration::Control(_) => counts.listeners += 1,
            }
        }
        counts
    }

    fn cancel(&self, id: u64) -> Option<Registration> {
        let registration = self.registrations.borrow_mut().remove(&id)?;
        if let Some(fixture) = &self.fixture {
            fixture.external_cancel(id);
        }
        if matches!(registration, Registration::Control(_)) {
            for (_, inbound) in self.control.borrow_mut().drain(..) {
                inbound.reply(Err("control handler unsubscribed".into()));
            }
        }
        let mut ready = self.ready.borrow_mut();
        if ready
            .as_ref()
            .is_some_and(|event| event.registration() == id)
            && let Some(Event::LiveControl(_, inbound)) = ready.take()
        {
            inbound.reply(Err("control handler unsubscribed".into()));
        }
        Some(registration)
    }

    pub(super) fn clear(&self, generation: u64) {
        let registrations = self.registrations.take();
        if let Some(fixture) = &self.fixture {
            fixture.retire_external_generation(generation);
        }
        if let Some(Event::LiveControl(_, inbound)) = self.ready.take() {
            inbound.reply(Err("session retired".into()));
        }
        for (_, inbound) in self.control.borrow_mut().drain(..) {
            inbound.reply(Err("session retired".into()));
        }
        for registration in registrations.into_values() {
            if let Registration::Socket {
                endpoint: Endpoint::Live(socket),
                ..
            } = registration
            {
                socket.retire();
            }
        }
    }
}

fn thrown(ctx: &Ctx<'_>, error: &ServiceError) -> rquickjs::Error {
    match service_error(ctx, error.code, &error.message) {
        Ok(error) => ctx.throw(error.into_value()),
        Err(error) => error,
    }
}

fn control_response(
    result: rquickjs::Result<Result<String, String>>,
    reply: impl FnOnce(Result<String, String>) -> rquickjs::Result<()>,
) -> rquickjs::Result<()> {
    match result {
        Ok(response) => reply(response),
        Err(error) => {
            reply(Err("control handler failed".into()))?;
            Err(error)
        }
    }
}

impl Work {
    pub(super) fn install_external<'js>(
        self: &Rc<Self>,
        services: &Object<'js>,
    ) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        services.prop(
            "getAuthorizedEndpoint",
            data_property(Func::from(move |ctx: Ctx<'js>, name: Value<'js>| {
                owner(&weak, &ctx)?.authorized_endpoint(&ctx, name)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "connectUnixSocket",
            data_property(Func::from(
                move |ctx: Ctx<'js>, path: Value<'js>, options: Value<'js>| {
                    owner(&weak, &ctx)?.connect_socket(&ctx, path, options)
                },
            )),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "spawnCommand",
            data_property(Func::from(
                move |ctx: Ctx<'js>,
                      program: Value<'js>,
                      args: Value<'js>,
                      options: Opt<Value<'js>>| {
                    owner(&weak, &ctx)?.spawn_command(&ctx, program, args, options.0)
                },
            )),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "onControlMessage",
            data_property(Func::from(move |ctx: Ctx<'js>, handler: Value<'js>| {
                owner(&weak, &ctx)?.register_control(&ctx, handler)
            })),
        )
    }

    fn authorized_endpoint<'js>(
        &self,
        ctx: &Ctx<'js>,
        name: Value<'js>,
    ) -> rquickjs::Result<Value<'js>> {
        let name = name
            .into_string()
            .ok_or_else(|| Exception::throw_type(ctx, "endpoint name must be a string"))?;
        let mut length = 0;
        // SAFETY: The primitive string belongs to this live context; its length is intrinsic.
        let status = unsafe {
            rquickjs::qjs::JS_GetLength(ctx.as_raw().as_ptr(), name.as_raw(), &mut length)
        };
        if status < 0 {
            return Err(rquickjs::Error::Exception);
        }
        if usize::try_from(length)
            .is_ok_and(|length| length <= crate::runtime_paths::MAX_SOCKET_NAME_BYTES)
        {
            let name = name.to_cstring()?;
            if let Some(endpoint) = checked_utf8(&name)
                .ok()
                .and_then(|name| self.resolved.named_sockets().get(name))
            {
                return endpoint_value(ctx, endpoint.as_deref());
            }
        }
        let message = "endpoint name is not declared in capabilities.namedSockets";
        if let Some(fixture) = &self.external.fixture {
            let failure = fixture.deny_capability(message);
            return Err(self.asset_error(ctx, AssetError::Contract(failure)));
        }
        Err(thrown(
            ctx,
            &ServiceError {
                code: "E_CAPABILITY",
                message: message.into(),
            },
        ))
    }

    fn invalid_external(
        &self,
        ctx: &Ctx<'_>,
        code: &'static str,
        message: impl Into<String>,
    ) -> rquickjs::Error {
        let message = message.into();
        if let Some(fixture) = &self.external.fixture {
            return self.asset_error(
                ctx,
                AssetError::Contract(fixture.external_violation(message)),
            );
        }
        thrown(ctx, &ServiceError { code, message })
    }

    fn require_external(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        if self.can_create_work() {
            Ok(())
        } else {
            Err(self.invalid_external(
                ctx,
                "InvalidState",
                "external services require an active session",
            ))
        }
    }

    fn bounded_text<'js>(
        &self,
        ctx: &Ctx<'js>,
        value: Value<'js>,
        limit: usize,
        name: &str,
    ) -> rquickjs::Result<rquickjs::CString<'js>> {
        let string = value
            .into_string()
            .ok_or_else(|| {
                self.invalid_external(ctx, "InvalidArgument", format!("{name} must be a string"))
            })?
            .to_cstring()?;
        if string.len() > limit {
            return Err(self.invalid_external(
                ctx,
                "TooLarge",
                format!("{name} exceeds {limit} UTF-8 bytes"),
            ));
        }
        Ok(string)
    }

    fn read_text<'a>(
        &self,
        ctx: &Ctx<'_>,
        string: &'a rquickjs::CString<'_>,
    ) -> rquickjs::Result<&'a str> {
        checked_utf8(string).map_err(|_| {
            self.invalid_external(
                ctx,
                "InvalidUtf8",
                "text contains an unmatched UTF-16 surrogate",
            )
        })
    }

    fn zero_arg_handle<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        id: u64,
    ) -> rquickjs::Result<Function<'js>> {
        let weak = Rc::downgrade(self);
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            owner(&weak, &ctx)?.cancel_external(&ctx, id)
        })
    }

    fn connect_socket<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        path: Value<'js>,
        options: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_external(ctx)?;
        let path = self.bounded_text(ctx, path, MAX_PROGRAM_BYTES, "socket path")?;
        let path = self.read_text(ctx, &path)?;
        if !self.resolved.allows_socket(path) {
            return Err(self.invalid_external(
                ctx,
                "PermissionDenied",
                format!("socket {path:?} is not authorized by capabilities.unixSockets or capabilities.namedSockets"),
            ));
        }
        SocketAddr::from_pathname(path)
            .map_err(|error| self.invalid_external(ctx, "InvalidArgument", error.to_string()))?;
        let options = options.into_object().ok_or_else(|| {
            self.invalid_external(ctx, "InvalidArgument", "socket options must be an object")
        })?;
        let framing = self.bounded_text(ctx, options.get("framing")?, 6, "framing")?;
        let framing = match self.read_text(ctx, &framing)? {
            "line" => Framing::Line,
            "length" => Framing::Length,
            _ => {
                return Err(self.invalid_external(
                    ctx,
                    "InvalidArgument",
                    "framing must be line or length",
                ));
            }
        };
        let message = self.read_callback(ctx, options.get("onMessage")?, "onMessage")?;
        let image = self.optional_callback(ctx, &options, "onBinaryImage")?;
        let accept_frame_streams = options
            .get::<_, Option<bool>>("acceptFrameStreams")?
            .unwrap_or(false);
        let frame_stream = self.optional_callback(ctx, &options, "onFrameStream")?;
        if (accept_frame_streams && framing != Framing::Length)
            || (frame_stream.is_some() && !accept_frame_streams)
        {
            return Err(self.invalid_external(
                ctx,
                "InvalidArgument",
                "frame streams require length framing and explicit opt-in",
            ));
        }
        let close = self.optional_callback(ctx, &options, "onClose")?;
        if self
            .external
            .registrations
            .borrow()
            .values()
            .filter(|entry| matches!(entry, Registration::Socket { .. }))
            .count()
            >= MAX_SOCKETS
        {
            return Err(self.invalid_external(
                ctx,
                "QueueFull",
                "Unix socket registration limit reached",
            ));
        }
        let id = self.id(ctx)?;
        let object = Object::new(ctx.clone())?;
        let weak = Rc::downgrade(self);
        object.prop(
            "send",
            data_property(Func::from(move |ctx: Ctx<'js>, message: Value<'js>| {
                owner(&weak, &ctx)?.send_socket(&ctx, id, message)
            })),
        )?;
        object.prop("close", data_property(self.zero_arg_handle(ctx, id)?))?;
        let endpoint = if let Some(fixture) = &self.external.fixture {
            fixture
                .open_socket(
                    self.generation,
                    id,
                    path,
                    framing,
                    accept_frame_streams,
                    self.external.package.as_deref(),
                )
                .map_err(|error| self.asset_error(ctx, AssetError::Contract(error)))?;
            Endpoint::Fixture
        } else {
            let socket = if accept_frame_streams {
                let changed = self.frame_streams.changed.clone();
                let wake = self.wake.clone();
                unix_socket::Socket::connect_frames(
                    path.into(),
                    self.frame_streams.budget.clone(),
                    self.frame_streams.modifiers(),
                    Arc::new(move || {
                        changed.store(true, std::sync::atomic::Ordering::Release);
                        wake();
                    }),
                )
            } else {
                unix_socket::Socket::connect(path.into(), framing, self.wake.clone())
            };
            Endpoint::Live(socket.map_err(|error| thrown(ctx, &error.into()))?)
        };
        self.external.registrations.borrow_mut().insert(
            id,
            Registration::Socket {
                endpoint,
                framing,
                message,
                image,
                frame_stream,
                close,
            },
        );
        Ok(object)
    }

    fn send_socket<'js>(&self, ctx: &Ctx<'js>, id: u64, value: Value<'js>) -> rquickjs::Result<()> {
        let message =
            self.bounded_text(ctx, value, unix_socket::MAX_MESSAGE_BYTES, "socket message")?;
        let message = self.read_text(ctx, &message)?;
        let registrations = self.external.registrations.borrow();
        let Some(Registration::Socket {
            endpoint, framing, ..
        }) = registrations.get(&id)
        else {
            return Err(self.invalid_external(ctx, "Closed", "Unix socket is closed"));
        };
        match endpoint {
            Endpoint::Live(socket) => socket.send(message).map_err(|error| thrown(ctx, &error)),
            Endpoint::Fixture => {
                framing
                    .validate_send(message)
                    .map_err(|error| self.invalid_external(ctx, error.code, error.message))?;
                if let Some(fixture) = &self.external.fixture {
                    fixture
                        .socket_send(id, message)
                        .map_err(|error| self.asset_error(ctx, AssetError::Contract(error)))?;
                }
                Ok(())
            }
        }
    }

    fn read_callback<'js>(
        &self,
        ctx: &Ctx<'js>,
        value: Value<'js>,
        name: &str,
    ) -> rquickjs::Result<Callback> {
        let callback = value.into_function().ok_or_else(|| {
            self.invalid_external(ctx, "InvalidArgument", format!("{name} must be a function"))
        })?;
        Ok(Persistent::save(ctx, callback))
    }

    fn optional_callback<'js>(
        &self,
        ctx: &Ctx<'js>,
        options: &Object<'js>,
        name: &str,
    ) -> rquickjs::Result<Option<Callback>> {
        let callback: Value = options.get(name)?;
        if callback.is_undefined() {
            return Ok(None);
        }
        self.read_callback(ctx, callback, name).map(Some)
    }

    fn argv<'js>(&self, ctx: &Ctx<'js>, args: Value<'js>) -> rquickjs::Result<Vec<String>> {
        let args = args.into_array().ok_or_else(|| {
            self.invalid_external(ctx, "InvalidArgument", "command arguments must be an array")
        })?;
        let length: u32 = args.as_object().get("length")?;
        if length > MAX_COMMAND_ARGS {
            return Err(self.invalid_external(
                ctx,
                "TooLarge",
                "command arguments exceed 64 entries",
            ));
        }
        let length = usize::try_from(length).map_err(|_| {
            self.invalid_external(
                ctx,
                "TooLarge",
                "command argument length is not representable",
            )
        })?;
        let mut argv = Vec::with_capacity(length);
        let mut remaining = MAX_ARG_BYTES;
        for index in 0..length {
            let arg = self.bounded_text(ctx, args.get(index)?, remaining, "command argument")?;
            let arg = self.read_text(ctx, &arg)?;
            if arg.contains('\0') {
                return Err(self.invalid_external(
                    ctx,
                    "InvalidArgument",
                    "command arguments cannot contain NUL",
                ));
            }
            remaining -= arg.len();
            argv.push(arg.to_owned());
        }
        Ok(argv)
    }

    fn spawn_command<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        program: Value<'js>,
        args: Value<'js>,
        options: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_external(ctx)?;
        let program = self.bounded_text(ctx, program, MAX_PROGRAM_BYTES, "command program")?;
        let program = self.read_text(ctx, &program)?;
        if !self
            .external
            .capabilities
            .commands
            .iter()
            .any(|allowed| allowed == program)
        {
            return Err(self.invalid_external(
                ctx,
                "PermissionDenied",
                format!("program {program:?} is not listed in capabilities.commands"),
            ));
        }
        command::validate_program(program)
            .map_err(|error| self.invalid_external(ctx, "InvalidArgument", error.to_string()))?;
        let args = self.argv(ctx, args)?;
        let settings = options
            .filter(|options| !options.is_undefined())
            .map(|options| {
                options.into_object().ok_or_else(|| {
                    self.invalid_external(
                        ctx,
                        "InvalidArgument",
                        "command options must be an object",
                    )
                })
            })
            .transpose()?;
        let streaming = if let Some(settings) = &settings {
            let stream: Value = settings.get("stream")?;
            if stream.is_undefined() {
                false
            } else {
                stream.as_bool().ok_or_else(|| {
                    self.invalid_external(ctx, "InvalidArgument", "stream must be a boolean")
                })?
            }
        } else {
            false
        };
        let id = self.id(ctx)?;
        let object = Object::new(ctx.clone())?;
        let cancel = self.zero_arg_handle(ctx, id)?;
        object.prop("cancel", data_property(cancel.clone()))?;
        let (mode, callbacks) = match settings.filter(|_| streaming) {
            Some(options) => {
                object.prop("kill", data_property(cancel))?;
                (
                    command::Mode::Stream,
                    CommandCallbacks::Streaming {
                        stdout: self.optional_callback(ctx, &options, "onStdout")?,
                        stderr: self.optional_callback(ctx, &options, "onStderr")?,
                        exit: self.optional_callback(ctx, &options, "onExit")?,
                        error: self.optional_callback(ctx, &options, "onError")?,
                    },
                )
            }
            None => {
                let (promise, resolve, reject) = ctx.promise()?;
                object.prop("promise", data_property(promise))?;
                (
                    command::Mode::Buffered,
                    CommandCallbacks::Buffered {
                        resolve: Persistent::save(ctx, resolve),
                        reject: Persistent::save(ctx, reject),
                    },
                )
            }
        };
        // Option access can execute user code; admit only after it has returned.
        if self
            .external
            .registrations
            .borrow()
            .values()
            .filter(|entry| matches!(entry, Registration::Command { .. }))
            .count()
            >= MAX_PROCESSES
        {
            return Err(self.invalid_external(
                ctx,
                "QueueFull",
                "command registration limit reached",
            ));
        }
        let endpoint = if let Some(fixture) = &self.external.fixture {
            fixture
                .spawn_command(self.generation, id, program, &args, mode)
                .map_err(|error| self.asset_error(ctx, AssetError::Contract(error)))?;
            Endpoint::Fixture
        } else {
            Endpoint::Live(
                command::Process::spawn(program.to_owned(), args, mode, self.wake.clone())
                    .map_err(|error| thrown(ctx, &error.into()))?,
            )
        };
        self.external.registrations.borrow_mut().insert(
            id,
            Registration::Command {
                endpoint,
                callbacks,
            },
        );
        Ok(object)
    }

    fn register_control<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handler: Value<'js>,
    ) -> rquickjs::Result<Function<'js>> {
        self.require_external(ctx)?;
        if !self.external.capabilities.control_channel {
            return Err(self.invalid_external(
                ctx,
                "PermissionDenied",
                "onControlMessage requires capabilities.controlChannel",
            ));
        }
        let handler = self.read_callback(ctx, handler, "control handler")?;
        if self.external.listener().is_some() {
            return Err(self.invalid_external(
                ctx,
                "QueueFull",
                "a session can register only one control handler",
            ));
        }
        let id = self.id(ctx)?;
        let unsubscribe = self.zero_arg_handle(ctx, id)?;
        if let Some(fixture) = &self.external.fixture {
            fixture
                .register_control(self.generation, id)
                .map_err(|error| self.asset_error(ctx, AssetError::Contract(error)))?;
        }
        self.external
            .registrations
            .borrow_mut()
            .insert(id, Registration::Control(handler));
        Ok(unsubscribe)
    }

    fn cancel_external(&self, ctx: &Ctx<'_>, id: u64) -> rquickjs::Result<()> {
        // Frame sockets keep their wire open for outstanding GPU credits, without joining here.
        let registration = self.external.cancel(id);
        self.frame_streams.close(id);
        if matches!(registration, Some(Registration::Socket { .. })) {
            self.frame_streams
                .changed
                .store(true, std::sync::atomic::Ordering::Release);
            (self.wake)();
        }
        match registration {
            Some(Registration::Socket {
                endpoint: Endpoint::Live(socket),
                ..
            }) => socket.retire(),
            Some(Registration::Command {
                endpoint,
                callbacks: CommandCallbacks::Buffered { reject, .. },
            }) => {
                drop(endpoint);
                reject.restore(ctx)?.call::<_, ()>((service_error(
                    ctx,
                    "Cancelled",
                    "command cancelled",
                )?,))?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Whether live ingress can deliver to this mounted generation.
    pub(crate) fn has_control_handler(&self) -> bool {
        self.mode.get() == Mode::Mounted
            && self.external.fixture.is_none()
            && self.external.listener().is_some()
    }

    /// Queue an inbound CLI request for a Bridge turn instead of executing inline.
    pub(crate) fn offer_control(&self, inbound: Inbound) {
        let listener = self
            .external
            .listener()
            .filter(|_| self.mode.get() == Mode::Mounted && self.external.fixture.is_none());
        let Some(id) = listener else {
            inbound.reply(Err("no active control handler".into()));
            return;
        };
        let mut queue = self.external.control.borrow_mut();
        if queue.len() >= MAX_CONTROL_QUEUE {
            inbound.reply(Err("control queue is full".into()));
        } else {
            queue.push_back((id, inbound));
            (self.wake)();
        }
    }

    /// Dequeue at most one external event for the next Bridge turn.
    pub(crate) fn next_external(&self) -> Option<Event> {
        if self.mode.get() != Mode::Mounted {
            return None;
        }
        self.external
            .ready
            .take()
            .or_else(|| self.external.take(self.generation))
    }

    /// Detect backlog without dropping an event already taken from a worker inbox.
    pub(super) fn has_external(&self) -> bool {
        if self.mode.get() != Mode::Mounted {
            return false;
        }
        let mut ready = self.external.ready.borrow_mut();
        if ready.is_some() {
            return true;
        }
        if let Some(fixture) = &self.external.fixture {
            return fixture.has_external(self.generation);
        }
        *ready = self.external.take(self.generation);
        ready.is_some()
    }

    /// Deliver one external event with no owner-state borrow held across user code.
    pub(crate) fn settle_external(
        self: &Rc<Self>,
        ctx: &Ctx<'_>,
        event: Event,
    ) -> rquickjs::Result<()> {
        match event {
            Event::Frame(id, frame) => self.deliver_frame(ctx, id, frame),
            Event::Socket(id, event) => self.deliver_socket(ctx, id, event),
            Event::Command(id, event) => self.deliver_command(ctx, id, event),
            Event::LiveControl(id, inbound) => {
                if inbound.is_expired() {
                    inbound.reply(Err("control request expired before delivery".into()));
                    return Ok(());
                }
                control_response(self.control_reply(ctx, id, &inbound.args), |reply| {
                    inbound.reply(reply);
                    Ok(())
                })
            }
            Event::FixtureControl {
                listener,
                request,
                args,
            } => control_response(self.control_reply(ctx, listener, &args), |reply| {
                if let Some(fixture) = &self.external.fixture {
                    fixture
                        .external_reply(request, reply)
                        .map_err(|error| self.asset_error(ctx, AssetError::Contract(error)))?;
                }
                Ok(())
            }),
        }
    }

    fn deliver_socket(
        self: &Rc<Self>,
        ctx: &Ctx<'_>,
        id: u64,
        event: unix_socket::Event,
    ) -> rquickjs::Result<()> {
        match event {
            unix_socket::Event::Message(text) => {
                let callback = match self.external.registrations.borrow().get(&id) {
                    Some(Registration::Socket { message, .. }) => message.clone(),
                    _ => return Ok(()),
                };
                callback.restore(ctx)?.call::<_, ()>((text,))
            }
            unix_socket::Event::Image(bytes) => {
                let callback = match self.external.registrations.borrow().get(&id) {
                    Some(Registration::Socket {
                        image: Some(callback),
                        ..
                    }) => callback.clone(),
                    _ => return Ok(()),
                };
                let len = bytes.len();
                if !self.external.can_receive_image(len) {
                    let error = ServiceError {
                        code: "TooLarge",
                        message: "release socket image handles before receiving more images".into(),
                    };
                    self.frame_streams.close(id);
                    return self.finish_socket(ctx, self.external.cancel(id), Some(error));
                }
                let callback = callback.restore(ctx)?;
                callback.call::<_, ()>((self.image_handle(ctx, bytes, ImageLease::Socket(len))?,))
            }
            unix_socket::Event::FrameStream(stream_id) => {
                let (stream, callback) = {
                    let registrations = self.external.registrations.borrow();
                    let Some(Registration::Socket {
                        endpoint: Endpoint::Live(socket),
                        frame_stream,
                        ..
                    }) = registrations.get(&id)
                    else {
                        return Ok(());
                    };
                    (socket.frame_stream(stream_id), frame_stream.clone())
                };
                let Some(stream) = stream else {
                    return Ok(());
                };
                let Some(callback) = callback else {
                    stream.close();
                    return Ok(());
                };
                let token = self.frame_streams.open_live(id, stream_id, stream);
                self.notify_frame_stream(ctx, token, stream_id, callback)
            }
            unix_socket::Event::Close(error) => {
                self.frame_streams.close(id);
                let registration = self.external.registrations.borrow_mut().remove(&id);
                self.finish_socket(ctx, registration, error)
            }
        }
    }

    fn deliver_frame(
        self: &Rc<Self>,
        ctx: &Ctx<'_>,
        socket: u64,
        frame: crate::frame_stream::SyntheticFrame,
    ) -> rquickjs::Result<()> {
        let callback = match self.external.registrations.borrow().get(&socket) {
            Some(Registration::Socket {
                frame_stream: Some(callback),
                ..
            }) => callback.clone(),
            _ => return Ok(()),
        };
        let stream_id = frame.stream_id;
        let token = match self.frame_streams.submit(socket, frame) {
            Ok(Some(token)) => token,
            Ok(None) => return Ok(()),
            Err(()) => {
                self.frame_streams.close(socket);
                return self.finish_socket(
                    ctx,
                    self.external.cancel(socket),
                    Some(ServiceError {
                        code: "E_PROTOCOL",
                        message: "session frame stream limit reached".into(),
                    }),
                );
            }
        };
        self.notify_frame_stream(ctx, token, stream_id, callback)
    }

    fn notify_frame_stream(
        self: &Rc<Self>,
        ctx: &Ctx<'_>,
        token: u64,
        stream_id: u64,
        callback: Callback,
    ) -> rquickjs::Result<()> {
        let handle = Object::new(ctx.clone())?;
        handle.prop("kind", data_property("frame-stream"))?;
        handle.prop("streamId", data_property(stream_id as f64))?;
        handle.prop("src", data_property(self.frame_streams.src(token)))?;
        let width = Rc::downgrade(&self.frame_streams);
        let height = width.clone();
        handle.prop(
            "width",
            rquickjs::object::Accessor::new_get(move || {
                width.upgrade().map_or(0, |s| s.size(token).width)
            }),
        )?;
        handle.prop(
            "height",
            rquickjs::object::Accessor::new_get(move || {
                height.upgrade().map_or(0, |s| s.size(token).height)
            }),
        )?;
        let weak = Rc::downgrade(self);
        handle.prop(
            "release",
            data_property(Func::from(move || {
                if let Some(work) = weak.upgrade() {
                    work.frame_streams.release(token);
                    work.frame_streams
                        .changed
                        .store(true, std::sync::atomic::Ordering::Release);
                    (work.wake)();
                }
            })),
        )?;
        callback.restore(ctx)?.call::<_, ()>((handle,))
    }

    fn finish_socket(
        &self,
        ctx: &Ctx<'_>,
        registration: Option<Registration>,
        error: Option<ServiceError>,
    ) -> rquickjs::Result<()> {
        let Some(Registration::Socket {
            endpoint, close, ..
        }) = registration
        else {
            return Ok(());
        };
        if let Endpoint::Live(socket) = endpoint {
            socket.retire();
        }
        let Some(callback) = close else {
            return Ok(());
        };
        callback
            .restore(ctx)?
            .call::<_, ()>((error_value(ctx, error)?,))
    }

    fn deliver_command(
        &self,
        ctx: &Ctx<'_>,
        id: u64,
        event: command::Event,
    ) -> rquickjs::Result<()> {
        let (text, callback) = match event {
            command::Event::Stdout(text) => {
                (text, self.command_line_callback(id, CommandPipe::Stdout))
            }
            command::Event::Stderr(text) => {
                (text, self.command_line_callback(id, CommandPipe::Stderr))
            }
            terminal => {
                let registration = self.external.registrations.borrow_mut().remove(&id);
                if let Some(Registration::Command {
                    endpoint,
                    callbacks,
                }) = registration
                {
                    drop(endpoint);
                    return self.finish_command(ctx, callbacks, terminal);
                }
                return Ok(());
            }
        };
        if let Some(callback) = callback {
            callback.restore(ctx)?.call::<_, ()>((text,))?;
        }
        Ok(())
    }

    fn command_line_callback(&self, id: u64, pipe: CommandPipe) -> Option<Callback> {
        let registrations = self.external.registrations.borrow();
        let Registration::Command {
            callbacks: CommandCallbacks::Streaming { stdout, stderr, .. },
            ..
        } = registrations.get(&id)?
        else {
            return None;
        };
        match pipe {
            CommandPipe::Stdout => stdout.clone(),
            CommandPipe::Stderr => stderr.clone(),
        }
    }

    fn finish_command(
        &self,
        ctx: &Ctx<'_>,
        callbacks: CommandCallbacks,
        terminal: command::Event,
    ) -> rquickjs::Result<()> {
        match (callbacks, terminal) {
            (CommandCallbacks::Buffered { resolve, .. }, command::Event::Output(output)) => {
                let result = Object::new(ctx.clone())?;
                result.prop("status", data_property(status_value(ctx, output.status)))?;
                result.prop("stdout", data_property(output.stdout))?;
                result.prop("stderr", data_property(output.stderr))?;
                resolve.restore(ctx)?.call::<_, ()>((result,))
            }
            (CommandCallbacks::Buffered { reject, .. }, command::Event::Error(error)) => reject
                .restore(ctx)?
                .call::<_, ()>((service_error(ctx, error.code, &error.message)?,)),
            (
                CommandCallbacks::Streaming {
                    exit: Some(callback),
                    ..
                },
                command::Event::Exit(status),
            ) => callback
                .restore(ctx)?
                .call::<_, ()>((status_value(ctx, status),)),
            (
                CommandCallbacks::Streaming {
                    error: Some(callback),
                    ..
                },
                command::Event::Error(error),
            ) => callback.restore(ctx)?.call::<_, ()>((service_error(
                ctx,
                error.code,
                &error.message,
            )?,)),
            _ => Ok(()),
        }
    }

    fn control_reply(
        &self,
        ctx: &Ctx<'_>,
        id: u64,
        args: &[String],
    ) -> rquickjs::Result<Result<String, String>> {
        let callback = match self.external.registrations.borrow().get(&id) {
            Some(Registration::Control(callback)) => callback.clone(),
            _ => return Ok(Err("control handler unsubscribed".into())),
        };
        let array = Array::new(ctx.clone())?;
        for (index, arg) in args.iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| {
                Exception::throw_range(ctx, "control arguments exceed array limits")
            })?;
            array.as_object().prop(index, data_property(arg.as_str()))?;
        }
        let value: Value = callback.restore(ctx)?.call((array,))?;
        if value.is_promise() {
            return Ok(Err("control reply must be synchronous".into()));
        }
        if value.is_undefined() {
            return Ok(Ok(String::new()));
        }
        let text = match value.try_into_string() {
            Ok(text) => Some(text),
            Err(value) => ctx.json_stringify(value)?,
        };
        let Some(text) = text else {
            return Ok(Err("control reply must be text or JSON".into()));
        };
        let text = text.to_cstring()?;
        if text.len() > MAX_REPLY_BYTES {
            return Ok(Err("control reply exceeds 64 KiB".into()));
        }
        Ok(checked_utf8(&text)
            .map(str::to_owned)
            .map_err(|_| "control reply is not UTF-8".into()))
    }
}

enum CommandPipe {
    Stdout,
    Stderr,
}

fn status_value<'js>(ctx: &Ctx<'js>, status: Option<i32>) -> Value<'js> {
    status.map_or_else(
        || Value::new_null(ctx.clone()),
        |status| Value::new_int(ctx.clone(), status),
    )
}

fn error_value<'js>(ctx: &Ctx<'js>, error: Option<ServiceError>) -> rquickjs::Result<Value<'js>> {
    match error {
        Some(error) => Ok(service_error(ctx, error.code, &error.message)?.into_value()),
        None => Ok(Value::new_null(ctx.clone())),
    }
}

fn checked_utf8<'a>(string: &'a rquickjs::CString<'_>) -> Result<&'a str, std::str::Utf8Error> {
    // SAFETY: QuickJS owns these `len` initialized bytes for the CString's lifetime.
    let bytes = unsafe { std::slice::from_raw_parts(string.as_ptr().cast::<u8>(), string.len()) };
    std::str::from_utf8(bytes)
}
