//! Deterministic socket, subprocess, and inbound-control scripts.
use super::{
    ByteCount, ContractFailure, FixtureRuntime, InjectedError, ScheduleKey, ScheduleOrder,
    Scheduled, Services, State, failure,
};
use crate::{
    bounded_fs::relative_path,
    package::Package,
    schema::{deserialize_object, object_struct},
};
use host_services::{command, control, unix_socket};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};

const MAX_SCRIPT_ITEMS: usize = 1024;

object_struct! {
    #[serde(deny_unknown_fields)]
    pub(super) struct SocketScript {
        path: String,
        framing: Framing,
        send: Vec<String>,
        events: Vec<SocketStep>,
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Framing {
    Line,
    Length,
}

impl From<unix_socket::Framing> for Framing {
    fn from(framing: unix_socket::Framing) -> Self {
        match framing {
            unix_socket::Framing::Line => Self::Line,
            unix_socket::Framing::Length => Self::Length,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum SocketStep {
    Message {
        at_ms: u32,
        text: String,
    },
    Image {
        at_ms: u32,
        asset: String,
    },
    Close {
        at_ms: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<InjectedError>,
    },
}

impl SocketStep {
    fn time(&self) -> u32 {
        match self {
            Self::Message { at_ms, .. } | Self::Image { at_ms, .. } | Self::Close { at_ms, .. } => {
                *at_ms
            }
        }
    }
}

object_struct! {
    #[serde(deny_unknown_fields)]
    pub(super) struct CommandScript {
        program: String,
        args: Vec<String>,
        mode: CommandMode,
        events: Vec<CommandStep>,
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum CommandMode {
    Buffered,
    Streaming,
}

impl From<command::Mode> for CommandMode {
    fn from(mode: command::Mode) -> Self {
        match mode {
            command::Mode::Buffered => Self::Buffered,
            command::Mode::Stream => Self::Streaming,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum CommandStep {
    Stdout {
        at_ms: u32,
        text: String,
    },
    Stderr {
        at_ms: u32,
        text: String,
    },
    Output {
        at_ms: u32,
        #[serde(deserialize_with = "Deserialize::deserialize")]
        status: Option<i32>,
        stdout: String,
        stderr: String,
    },
    Exit {
        at_ms: u32,
        #[serde(deserialize_with = "Deserialize::deserialize")]
        status: Option<i32>,
    },
    Error {
        at_ms: u32,
        error: InjectedError,
    },
}

impl CommandStep {
    fn event(&self) -> (u32, command::Event) {
        match self {
            Self::Stdout { at_ms, text } => (*at_ms, command::Event::Stdout(text.clone())),
            Self::Stderr { at_ms, text } => (*at_ms, command::Event::Stderr(text.clone())),
            Self::Output {
                at_ms,
                status,
                stdout,
                stderr,
            } => (
                *at_ms,
                command::Event::Output(command::Output {
                    status: *status,
                    stdout: stdout.clone(),
                    stderr: stderr.clone(),
                }),
            ),
            Self::Exit { at_ms, status } => (*at_ms, command::Event::Exit(*status)),
            Self::Error { at_ms, error } => (*at_ms, command::Event::Error(error.service_error())),
        }
    }

    fn time(&self) -> u32 {
        match self {
            Self::Stdout { at_ms, .. }
            | Self::Stderr { at_ms, .. }
            | Self::Output { at_ms, .. }
            | Self::Exit { at_ms, .. }
            | Self::Error { at_ms, .. } => *at_ms,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ControlProvider {}

impl<'de> Deserialize<'de> for ControlProvider {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Empty {}
        deserialize_object::<D, Empty>(deserializer).map(|_| Self {})
    }
}

object_struct! {
    #[serde(deny_unknown_fields)]
    /// One inbound argument vector and its expected host-visible reply.
    pub(crate) struct ControlMessage {
        args: Vec<String>,
        reply: ExpectedReply,
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum ExpectedReply {
    Text(String),
    Error(String),
}

impl ExpectedReply {
    fn as_result(&self) -> Result<&str, &str> {
        match self {
            Self::Text(text) => Ok(text),
            Self::Error(error) => Err(error),
        }
    }
}

impl ControlMessage {
    pub(super) fn validate(&self) -> Result<(), String> {
        let mut bytes = ByteCount(0);
        serde_json::to_writer(&mut bytes, &self.args).map_err(|error| error.to_string())?;
        if self.args.len() > control::MAX_ARGS || bytes.0 > control::MAX_REQUEST_BYTES {
            return Err("control.message args exceed the control request limit".into());
        }
        let (Ok(reply) | Err(reply)) = self.reply.as_result();
        if reply.len() > control::MAX_REPLY_BYTES {
            return Err("control.message reply exceeds the control reply limit".into());
        }
        Ok(())
    }
}

impl Services {
    pub(super) fn validate_external(&self) -> Result<(), String> {
        if let Some(sockets) = &self.sockets {
            if sockets.len() > MAX_SCRIPT_ITEMS {
                return Err("fixture exceeds 1024 socket scripts".into());
            }
            for script in sockets {
                script.validate()?;
            }
        }
        if let Some(commands) = &self.commands {
            if commands.len() > MAX_SCRIPT_ITEMS {
                return Err("fixture exceeds 1024 command scripts".into());
            }
            for script in commands {
                script.validate()?;
            }
        }
        Ok(())
    }
}

impl SocketScript {
    fn validate(&self) -> Result<(), String> {
        if self.path == "/" || !crate::runtime_paths::canonical_absolute(&self.path) {
            return Err("fixture socket path must be canonical absolute text".into());
        }
        if self.send.len() > MAX_SCRIPT_ITEMS || self.events.len() > MAX_SCRIPT_ITEMS {
            return Err("fixture socket script exceeds 1024 sends or events".into());
        }
        if self.framing == Framing::Line && self.send.iter().any(|message| message.contains('\n')) {
            return Err("fixture line socket send cannot contain a newline".into());
        }
        if !self.events.iter().map(SocketStep::time).is_sorted() {
            return Err("fixture socket events must be ordered by atMs".into());
        }
        for (index, step) in self.events.iter().enumerate() {
            match step {
                SocketStep::Message { text, .. }
                    if self.framing == Framing::Line && text.contains('\n') =>
                {
                    return Err("fixture line socket message cannot contain a newline".into());
                }
                SocketStep::Image { asset, .. }
                    if self.framing != Framing::Length || relative_path(asset).is_err() =>
                {
                    return Err("fixture socket image requires length framing and a canonical package asset path".into());
                }
                SocketStep::Close { error, .. } => {
                    if index + 1 != self.events.len() {
                        return Err("fixture socket close must be the final event".into());
                    }
                    if let Some(error) = error {
                        error.validate()?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl CommandScript {
    fn validate(&self) -> Result<(), String> {
        if self.program.trim_matches(' ').is_empty()
            || self.program.chars().any(char::is_control)
            || self.args.iter().any(|arg| arg.contains('\0'))
        {
            return Err(
                "fixture command must name an executable and use NUL-free arguments".into(),
            );
        }
        if self.events.len() > MAX_SCRIPT_ITEMS
            || !self.events.iter().map(CommandStep::time).is_sorted()
        {
            return Err("fixture command permits up to 1024 events ordered by atMs".into());
        }
        for (index, step) in self.events.iter().enumerate() {
            match (self.mode, step) {
                (CommandMode::Buffered, CommandStep::Output { .. })
                | (CommandMode::Streaming, CommandStep::Exit { .. })
                | (_, CommandStep::Error { .. }) => {
                    if index + 1 != self.events.len() {
                        return Err("fixture command completion must be its final event".into());
                    }
                    if let CommandStep::Error { error, .. } = step {
                        error.validate()?;
                    }
                }
                (
                    CommandMode::Streaming,
                    CommandStep::Stdout { text, .. } | CommandStep::Stderr { text, .. },
                ) => {
                    if text.contains('\n') {
                        return Err("fixture command streaming line must not contain LF".into());
                    }
                    if text.len() > command::MAX_LINE_BYTES {
                        return Err("fixture command streaming line exceeds the byte limit".into());
                    }
                }
                _ => return Err("fixture command event is incompatible with its mode".into()),
            }
        }
        Ok(())
    }
}

/// One owned outcome ready for a Bridge turn in its originating generation.
#[derive(Debug)]
pub(crate) enum ExternalDelivery {
    /// One framed message, image, or close notification for the socket ID.
    Socket(u64, unix_socket::Event),
    /// One command stream event or buffered completion for the command ID.
    Command(u64, command::Event),
    /// One inbound control message awaiting a checked reply.
    Control {
        listener: u64,
        request: u64,
        args: Vec<String>,
    },
}

pub(super) enum ExternalJob {
    Socket(u64, unix_socket::Event),
    Command(u64, command::Event),
    Control {
        listener: u64,
        request: u64,
        message: ControlMessage,
    },
}

impl ExternalJob {
    fn registration(&self) -> u64 {
        match self {
            Self::Socket(id, _) | Self::Command(id, _) => *id,
            Self::Control { listener, .. } => *listener,
        }
    }
}

struct SocketConnection {
    generation: u64,
    script: usize,
    next_send: usize,
}

struct PendingReply {
    generation: u64,
    listener: u64,
    expected: ExpectedReply,
}

#[derive(Default)]
pub(super) struct ExternalState {
    next_socket: usize,
    next_command: usize,
    sockets: BTreeMap<u64, SocketConnection>,
    // A replacement may register while its predecessor is still active. Only
    // the oldest surviving registration receives inbound requests.
    listeners: BTreeMap<u64, u64>,
    deliveries: BTreeMap<u64, VecDeque<ExternalJob>>,
    replies: BTreeMap<u64, PendingReply>,
}

impl ExternalState {
    pub(super) fn delivery_count(&self) -> usize {
        self.deliveries.values().map(VecDeque::len).sum()
    }

    pub(super) fn dispose(&mut self) {
        self.sockets.clear();
        self.listeners.clear();
        self.deliveries.clear();
        self.replies.clear();
    }
}

impl FixtureRuntime {
    /// Record a sticky contract violation, including capability denials caught in JavaScript.
    pub(crate) fn external_violation(&self, message: impl Into<String>) -> ContractFailure {
        self.malformed(message)
    }

    fn external_failure(&self, state: &mut State, message: impl Into<String>) -> ContractFailure {
        self.record_failure(state, failure("E_FIXTURE_CONTRACT", message))
    }

    fn external_id(&self, state: &mut State) -> Result<u64, ContractFailure> {
        Self::allocate_id(state)
            .ok_or_else(|| self.external_failure(state, "fixture registration ID exhausted"))
    }

    /// Admit the next exact socket script and schedule its events without connecting to the OS.
    ///
    /// # Errors
    /// Fails the fixture on missing providers, unmatched scripts, or unavailable package bytes.
    pub(crate) fn open_socket(
        &self,
        generation: u64,
        id: u64,
        path: &str,
        framing: unix_socket::Framing,
        package: Option<&Package>,
    ) -> Result<(), ContractFailure> {
        let mut state = self.state.borrow_mut();
        self.admit(&mut state, "sockets", "connectUnixSocket", id, generation)?;
        let framing = Framing::from(framing);
        self.record(
            &mut state,
            "arguments",
            json!({"requestId":id,"path":path,"framing":framing}),
        );
        let Some(scripts) = &self.definition.services.sockets else {
            return Err(self.record_failure(
                &mut state,
                failure(
                    "E_FIXTURE_MISSING_SERVICE",
                    "preview fixture does not provide connectUnixSocket",
                ),
            ));
        };
        let script_index = state.external.next_socket;
        let Some(script) = scripts.get(script_index) else {
            return Err(self.external_failure(
                &mut state,
                format!("no fixture socket script remains for {path:?} ({framing:?})"),
            ));
        };
        if script.path != path || script.framing != framing {
            return Err(self.external_failure(
                &mut state,
                format!(
                    "expected socket {:?} ({:?}), received {path:?} ({framing:?})",
                    script.path, script.framing
                ),
            ));
        }
        let now = self.environment.now_us();
        for (index, step) in script.events.iter().enumerate() {
            let event = match step {
                SocketStep::Message { text, .. } => unix_socket::Event::Message(text.clone()),
                SocketStep::Image { asset, .. } => {
                    let bytes = package.and_then(|package| package.asset_data(asset)).ok_or_else(|| {
                        self.external_failure(&mut state, format!("fixture socket image {asset:?} is not declared in the loaded package"))
                    })?;
                    if bytes.len() > unix_socket::MAX_MESSAGE_BYTES {
                        return Err(self.external_failure(
                            &mut state,
                            format!(
                                "fixture socket image {asset:?} exceeds the socket frame byte limit"
                            ),
                        ));
                    }
                    unix_socket::Event::Image(bytes)
                }
                SocketStep::Close { error, .. } => {
                    unix_socket::Event::Close(error.as_ref().map(InjectedError::service_error))
                }
            };
            self.schedule_external(
                &mut state,
                (now, step.time()),
                generation,
                index,
                ExternalJob::Socket(id, event),
            )?;
        }
        state.external.next_socket += 1;
        state.external.sockets.insert(
            id,
            SocketConnection {
                generation,
                script: script_index,
                next_send: 0,
            },
        );
        Ok(())
    }

    /// Match the next expected outbound text frame for this socket.
    ///
    /// # Errors
    /// Fails the fixture when the socket or exact outbound frame is not scripted.
    pub(crate) fn socket_send(&self, id: u64, message: &str) -> Result<(), ContractFailure> {
        let mut state = self.state.borrow_mut();
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        let Some((generation, script, next_send)) = state
            .external
            .sockets
            .get(&id)
            .map(|socket| (socket.generation, socket.script, socket.next_send))
        else {
            return Err(
                self.external_failure(&mut state, format!("send on inactive fixture socket {id}"))
            );
        };
        let request = self.external_id(&mut state)?;
        self.admit(&mut state, "sockets", "send", request, generation)?;
        self.record(
            &mut state,
            "arguments",
            json!({"requestId":request,"socketId":id,"message":message}),
        );
        let expected = self
            .definition
            .services
            .sockets
            .as_ref()
            .and_then(|scripts| scripts.get(script))
            .and_then(|script| script.send.get(next_send));
        if expected.map(String::as_str) != Some(message) {
            return Err(self.external_failure(
                &mut state,
                format!("fixture socket {id} expected send {expected:?}, received {message:?}"),
            ));
        }
        if let Some(socket) = state.external.sockets.get_mut(&id) {
            socket.next_send += 1;
        }
        Ok(())
    }

    /// Admit the next exact subprocess script without launching a program.
    ///
    /// # Errors
    /// Fails the fixture when the executable, ordered arguments, or mode are not scripted.
    pub(crate) fn spawn_command(
        &self,
        generation: u64,
        id: u64,
        program: &str,
        args: &[String],
        mode: command::Mode,
    ) -> Result<(), ContractFailure> {
        let mut state = self.state.borrow_mut();
        self.admit(&mut state, "commands", "spawnCommand", id, generation)?;
        let mode = CommandMode::from(mode);
        self.record(
            &mut state,
            "arguments",
            json!({"requestId":id,"program":program,"args":args,"mode":mode}),
        );
        let Some(scripts) = &self.definition.services.commands else {
            return Err(self.record_failure(
                &mut state,
                failure(
                    "E_FIXTURE_MISSING_SERVICE",
                    "preview fixture does not provide spawnCommand",
                ),
            ));
        };
        let Some(script) = scripts.get(state.external.next_command) else {
            return Err(self.external_failure(
                &mut state,
                format!("no fixture command script remains for {program:?} {args:?} ({mode:?})"),
            ));
        };
        if script.program != program || script.args != args || script.mode != mode {
            return Err(self.external_failure(
                &mut state,
                format!(
                    "expected command {:?} {:?} ({:?}), received {program:?} {args:?} ({mode:?})",
                    script.program, script.args, script.mode
                ),
            ));
        }
        let now = self.environment.now_us();
        for (index, step) in script.events.iter().enumerate() {
            let (at_ms, event) = step.event();
            self.schedule_external(
                &mut state,
                (now, at_ms),
                generation,
                index,
                ExternalJob::Command(id, event),
            )?;
        }
        state.external.next_command += 1;
        Ok(())
    }

    fn schedule_external(
        &self,
        state: &mut State,
        (now, at_ms): (u64, u32),
        generation: u64,
        index: usize,
        job: ExternalJob,
    ) -> Result<(), ContractFailure> {
        let deadline = now
            .checked_add(u64::from(at_ms) * 1000)
            .ok_or_else(|| self.external_failure(state, "fixture deadline overflow"))?;
        state.scheduled.insert(
            ScheduleKey {
                deadline_us: deadline,
                order: ScheduleOrder::Call {
                    id: job.registration(),
                    index,
                },
            },
            Scheduled::External { generation, job },
        );
        Ok(())
    }

    /// Register the generation's sole control callback, staging replacements until retirement.
    ///
    /// # Errors
    /// Fails the fixture if the provider is missing or a callback already exists for the generation.
    pub(crate) fn register_control(&self, generation: u64, id: u64) -> Result<(), ContractFailure> {
        let mut state = self.state.borrow_mut();
        self.admit(&mut state, "control", "onControlMessage", id, generation)?;
        if self.definition.services.control.is_none() {
            return Err(self.record_failure(
                &mut state,
                failure(
                    "E_FIXTURE_MISSING_SERVICE",
                    "preview fixture does not provide onControlMessage",
                ),
            ));
        }
        if state.external.listeners.contains_key(&generation) {
            return Err(self.external_failure(
                &mut state,
                "fixture has duplicate control listeners for one generation",
            ));
        }
        state.external.listeners.insert(generation, id);
        Ok(())
    }

    pub(super) fn dispatch_control(
        &self,
        state: &mut State,
        message: ControlMessage,
    ) -> Result<(), ContractFailure> {
        let Some((&generation, &listener)) = state.external.listeners.first_key_value() else {
            return Err(
                self.external_failure(state, "fixture control.message has no active listener")
            );
        };
        let request = self.external_id(state)?;
        self.record(state, "state", json!({"channel":"control.message","requestId":request,"listenerId":listener,"args":&message.args}));
        self.enqueue_external(
            state,
            generation,
            ExternalJob::Control {
                listener,
                request,
                message,
            },
        )
    }

    pub(super) fn enqueue_external(
        &self,
        state: &mut State,
        generation: u64,
        job: ExternalJob,
    ) -> Result<(), ContractFailure> {
        self.check_delivery_limit(state)?;
        state
            .external
            .deliveries
            .entry(generation)
            .or_default()
            .push_back(job);
        Ok(())
    }

    /// Report whether this generation has owner-thread work ready for the bridge.
    pub(crate) fn has_external(&self, generation: u64) -> bool {
        self.state
            .borrow()
            .external
            .deliveries
            .get(&generation)
            .is_some_and(|queue| !queue.is_empty())
    }

    /// Take this generation's oldest ready event, never entering the application directly.
    pub(crate) fn take_external(&self, generation: u64) -> Option<ExternalDelivery> {
        let mut state = self.state.borrow_mut();
        if state.failure.is_some() {
            return None;
        }
        let queue = state.external.deliveries.get_mut(&generation)?;
        let job = queue.pop_front()?;
        if queue.is_empty() {
            state.external.deliveries.remove(&generation);
        }
        match job {
            ExternalJob::Socket(id, event) => {
                if matches!(&event, unix_socket::Event::Close(_))
                    && !self.close_fixture_socket(&mut state, id)
                {
                    return None;
                }
                let error_code = match &event {
                    unix_socket::Event::Close(Some(error)) => Some(error.code),
                    _ => None,
                };
                let detail = match &event {
                    unix_socket::Event::Message(text) => {
                        json!({"requestId":id,"event":"message","text":text})
                    }
                    unix_socket::Event::Image(bytes) => {
                        json!({"requestId":id,"event":"image","byteLength":bytes.len()})
                    }
                    unix_socket::Event::Close(_) => {
                        json!({"requestId":id,"event":"close","errorCode":error_code})
                    }
                };
                self.deliver_external_response(&mut state, id, detail, error_code);
                Some(ExternalDelivery::Socket(id, event))
            }
            ExternalJob::Command(id, event) => {
                let (detail, error_code) = match &event {
                    command::Event::Stdout(text) => {
                        (json!({"requestId":id,"event":"stdout","text":text}), None)
                    }
                    command::Event::Stderr(text) => {
                        (json!({"requestId":id,"event":"stderr","text":text}), None)
                    }
                    command::Event::Output(output) => (
                        json!({"requestId":id,"event":"output","status":output.status,"stdout":output.stdout,"stderr":output.stderr}),
                        None,
                    ),
                    command::Event::Exit(status) => {
                        (json!({"requestId":id,"event":"exit","status":status}), None)
                    }
                    command::Event::Error(error) => (
                        json!({"requestId":id,"event":"error","errorCode":error.code}),
                        Some(error.code),
                    ),
                };
                self.deliver_external_response(&mut state, id, detail, error_code);
                Some(ExternalDelivery::Command(id, event))
            }
            ExternalJob::Control {
                listener,
                request,
                message,
            } => {
                state.external.replies.insert(
                    request,
                    PendingReply {
                        generation,
                        listener,
                        expected: message.reply,
                    },
                );
                Some(ExternalDelivery::Control {
                    listener,
                    request,
                    args: message.args,
                })
            }
        }
    }

    fn deliver_external_response(
        &self,
        state: &mut State,
        id: u64,
        detail: Value,
        error_code: Option<&'static str>,
    ) {
        if let Some(code) = error_code
            && let Some(call) = state.calls.get_mut(&id)
        {
            call.error = Some(code);
            call.error_delivered = true;
        }
        self.record(state, "response", detail);
    }

    /// Record and check the exact opaque reply for one pending control request.
    ///
    /// # Errors
    /// Fails the fixture for duplicate, unknown, oversized, or mismatched replies.
    pub(crate) fn external_reply(
        &self,
        id: u64,
        reply: Result<String, String>,
    ) -> Result<(), ContractFailure> {
        let mut state = self.state.borrow_mut();
        if let Some(error) = &state.failure {
            return Err(error.clone());
        }
        let Some(pending) = state.external.replies.remove(&id) else {
            return Err(self.external_failure(
                &mut state,
                format!("reply for unknown fixture control request {id}"),
            ));
        };
        self.check_control_reply(
            &mut state,
            id,
            pending.generation,
            pending.listener,
            pending.expected.as_result(),
            reply.as_deref().map_err(String::as_str),
        )
    }

    fn check_control_reply(
        &self,
        state: &mut State,
        id: u64,
        generation: u64,
        listener: u64,
        expected: Result<&str, &str>,
        actual: Result<&str, &str>,
    ) -> Result<(), ContractFailure> {
        self.admit(state, "control", "reply", id, generation)?;
        let detail = match actual {
            Ok(text) => json!({"requestId":id,"listenerId":listener,"text":text}),
            Err(error) => json!({"requestId":id,"listenerId":listener,"error":error}),
        };
        self.record(state, "response", detail);
        if actual != expected {
            return Err(self.external_failure(
                state,
                format!("fixture control request {id} expected {expected:?}, received {actual:?}"),
            ));
        }
        Ok(())
    }

    fn missing_socket_sends(&self, id: u64, socket: &SocketConnection) -> Option<String> {
        let script = self
            .definition
            .services
            .sockets
            .as_ref()?
            .get(socket.script)?;
        let missing = script.send.len() - socket.next_send;
        (missing > 0).then(|| {
            format!(
                "fixture socket {id} ({:?}) is missing {missing} expected sends",
                script.path
            )
        })
    }

    fn close_fixture_socket(&self, state: &mut State, id: u64) -> bool {
        if let Some(socket) = state.external.sockets.remove(&id)
            && let Some(message) = self.missing_socket_sends(id, &socket)
        {
            self.external_failure(state, message);
            return false;
        }
        true
    }

    /// Withdraw a registration, rejecting its queued control requests and discarding other work.
    pub(crate) fn external_cancel(&self, id: u64) {
        let mut state = self.state.borrow_mut();
        if state.disposed {
            return;
        }
        let Some(generation) = state.calls.get(&id).map(|call| call.generation) else {
            return;
        };
        self.close_fixture_socket(&mut state, id);
        if state.external.listeners.get(&generation) == Some(&id) {
            state.external.listeners.remove(&generation);
        }
        state.scheduled.retain(
            |_, item| !matches!(item, Scheduled::External { job, .. } if job.registration() == id),
        );
        if let Some(queue) = state.external.deliveries.get_mut(&generation) {
            // Move the queue out while recording replies in the same fixture state.
            let mut queue = std::mem::take(queue);
            queue.retain(|job| {
                if job.registration() != id {
                    return true;
                }
                if let ExternalJob::Control {
                    request, message, ..
                } = job
                {
                    // Reply mismatches are sticky, including host-generated rejections.
                    let _ = self.check_control_reply(
                        &mut state,
                        *request,
                        generation,
                        id,
                        message.reply.as_result(),
                        Err("control handler unsubscribed"),
                    );
                }
                false
            });
            if queue.is_empty() {
                state.external.deliveries.remove(&generation);
            } else {
                state.external.deliveries.insert(generation, queue);
            }
        }
        // A control handler may unsubscribe itself before returning its reply.
        self.record(&mut state, "cancellation", json!({"requestId":id}));
    }

    /// Discard the retiring generation's remaining work without disturbing a staged replacement.
    pub(crate) fn retire_external_generation(&self, generation: u64) {
        let mut state = self.state.borrow_mut();
        if state.disposed {
            return;
        }
        let services = &self.definition.services;
        if services.sockets.is_none() && services.commands.is_none() && services.control.is_none() {
            return;
        }
        state.scheduled.retain(|_, item| !matches!(item, Scheduled::External { generation: owner, .. } if *owner == generation));
        state.external.deliveries.remove(&generation);
        state.external.listeners.remove(&generation);
        state
            .external
            .sockets
            .retain(|_, socket| socket.generation != generation);
        state
            .external
            .replies
            .retain(|_, pending| pending.generation != generation);
        self.record(
            &mut state,
            "generationRetired",
            json!({"generation":generation}),
        );
    }

    pub(super) fn finalize_external(&self, state: &State) -> Result<(), ContractFailure> {
        if let Some((&id, _)) = state.external.replies.first_key_value() {
            return Err(failure(
                "E_FIXTURE_CONTRACT",
                format!("fixture control request {id} has no reply"),
            ));
        }
        if let Some(message) = state
            .external
            .sockets
            .iter()
            .find_map(|(&id, socket)| self.missing_socket_sends(id, socket))
        {
            return Err(failure("E_FIXTURE_CONTRACT", message));
        }
        for (service, declared, consumed) in [
            (
                "socket",
                self.definition
                    .services
                    .sockets
                    .as_ref()
                    .map_or(0, Vec::len),
                state.external.next_socket,
            ),
            (
                "command",
                self.definition
                    .services
                    .commands
                    .as_ref()
                    .map_or(0, Vec::len),
                state.external.next_command,
            ),
        ] {
            if consumed != declared {
                return Err(failure(
                    "E_FIXTURE_CONTRACT",
                    format!("fixture consumed {consumed} of {declared} {service} scripts"),
                ));
            }
        }
        Ok(())
    }
}
