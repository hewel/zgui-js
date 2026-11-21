//! App-scoped SQLite engine with WAL read/write connection split, FTS5, and hard-timeout transactions.
#![forbid(unsafe_code)]

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

use rusqlite::{Connection, OpenFlags, types::Value as RusqliteValue, types::ValueRef};

use crate::ServiceError;

pub const MAX_QUERY_ROWS: usize = 10_000;
pub const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_BATCH_STATEMENTS: usize = 1_000;
pub const MAX_SQL_BYTES: usize = 1024 * 1024;
pub const DEFAULT_TX_TIMEOUT_MS: u64 = 5_000;
pub const MAX_TX_TIMEOUT_MS: u64 = 30_000;

const READER_POOL_SIZE: usize = 2;
const COMMAND_QUEUE_CAPACITY: usize = 256;
const EVENT_QUEUE_CAPACITY: usize = 512;

static MEMORY_DB_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Validated database name: 1..=64 ASCII alphanumeric, `_`, or `-`, starting with alphanumeric.
#[must_use]
pub fn is_valid_database_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
}

/// Validates an application ID as a single safe directory component (no traversal or separators).
#[must_use]
pub fn is_safe_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && app_id.len() <= 128
        && app_id != "."
        && app_id != ".."
        && app_id.trim() == app_id
        && !app_id.contains(['/', '\\', '\0'])
        && !app_id.chars().any(char::is_control)
}

/// Resolves the live database path `$XDG_DATA_HOME/<appId>/<name>.sqlite3`.
pub fn resolve_database_path(app_id: &str, db_name: &str) -> Result<PathBuf, ServiceError> {
    if !is_safe_app_id(app_id) {
        return Err(ServiceError::new(
            "InvalidPath",
            format!("invalid application ID for SQLite storage: {app_id:?}"),
        ));
    }
    if !is_valid_database_name(db_name) {
        return Err(ServiceError::new(
            "InvalidPath",
            format!("invalid SQLite database name: {db_name:?}"),
        ));
    }
    let base = resolve_data_home()?;
    Ok(base.join(app_id).join(format!("{db_name}.sqlite3")))
}

fn resolve_data_home() -> Result<PathBuf, ServiceError> {
    if let Ok(xdg) = env::var("XDG_DATA_HOME")
        && !xdg.is_empty()
    {
        let path = PathBuf::from(&xdg);
        if path.is_absolute() {
            return Ok(path);
        }
    }
    if let Ok(home) = env::var("HOME")
        && !home.is_empty()
    {
        let path = PathBuf::from(&home);
        if path.is_absolute() {
            return Ok(path.join(".local/share"));
        }
    }
    Err(ServiceError::new(
        "InvalidState",
        "neither XDG_DATA_HOME nor HOME is set to an absolute path",
    ))
}

/// Owned SQL parameter or column cell value.
#[derive(Clone, Debug, PartialEq)]
pub enum SqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl SqlValue {
    fn to_rusqlite(&self) -> RusqliteValue {
        match self {
            Self::Null => RusqliteValue::Null,
            Self::Integer(v) => RusqliteValue::Integer(*v),
            Self::Real(v) => RusqliteValue::Real(*v),
            Self::Text(v) => RusqliteValue::Text(v.clone()),
            Self::Blob(v) => RusqliteValue::Blob(v.clone()),
        }
    }

    fn approx_bytes(&self) -> usize {
        match self {
            Self::Null | Self::Integer(_) | Self::Real(_) => 8,
            Self::Text(s) => s.len(),
            Self::Blob(b) => b.len(),
        }
    }
}

/// Strictly parameterized SQL bindings (positional or named).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum SqlParams {
    #[default]
    None,
    Positional(Vec<SqlValue>),
    Named(Vec<(String, SqlValue)>),
}

/// One returned row preserving column names and order.
pub type SqlRow = Vec<(String, SqlValue)>;

/// Result of a write statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SqlExecuteResult {
    pub rows_affected: u64,
    pub last_insert_rowid: Option<i64>,
}

/// One statement inside an atomic `batch(...)`.
#[derive(Clone, Debug)]
pub struct SqlBatchStatement {
    pub sql: String,
    pub params: SqlParams,
}

/// Asynchronous completion delivered back to the owner thread.
#[derive(Debug)]
pub enum SqliteEvent {
    Query {
        op_id: u64,
        result: Result<Vec<SqlRow>, ServiceError>,
    },
    Execute {
        op_id: u64,
        result: Result<SqlExecuteResult, ServiceError>,
    },
    Batch {
        op_id: u64,
        result: Result<Vec<SqlExecuteResult>, ServiceError>,
    },
    TxBegun {
        op_id: u64,
        tx_id: u64,
        result: Result<(), ServiceError>,
    },
    TxSettled {
        op_id: u64,
        tx_id: u64,
        result: Result<(), ServiceError>,
    },
    TxTimedOut {
        tx_id: u64,
    },
}

struct ReadJob {
    op_id: u64,
    sql: String,
    params: SqlParams,
    max_rows: usize,
}

enum WriteJob {
    Execute {
        op_id: u64,
        sql: String,
        params: SqlParams,
    },
    Batch {
        op_id: u64,
        statements: Vec<SqlBatchStatement>,
    },
    BeginTx {
        op_id: u64,
        tx_id: u64,
        timeout: Duration,
        timed_out: Arc<AtomicBool>,
        commands: Receiver<TxCommand>,
    },
}

enum TxCommand {
    Query {
        op_id: u64,
        sql: String,
        params: SqlParams,
        max_rows: usize,
    },
    Execute {
        op_id: u64,
        sql: String,
        params: SqlParams,
    },
    Commit {
        op_id: u64,
    },
    Rollback {
        op_id: u64,
    },
}

/// Owner-thread handle to an active interactive SQLite transaction.
///
/// Dropping this handle without committing automatically disconnects the writer's
/// transaction channel and triggers an immediate `ROLLBACK` on the writer thread.
pub struct SqliteTransaction {
    sender: Mutex<Option<SyncSender<TxCommand>>>,
    timed_out: Arc<AtomicBool>,
    settled: AtomicBool,
}

impl SqliteTransaction {
    pub fn query(
        &self,
        op_id: u64,
        sql: String,
        params: SqlParams,
        max_rows: usize,
    ) -> Result<(), ServiceError> {
        self.send(TxCommand::Query {
            op_id,
            sql,
            params,
            max_rows: clamp_max_rows(max_rows),
        })
    }

    pub fn execute(&self, op_id: u64, sql: String, params: SqlParams) -> Result<(), ServiceError> {
        self.send(TxCommand::Execute { op_id, sql, params })
    }

    pub fn commit(&self, op_id: u64) -> Result<(), ServiceError> {
        self.finish_with(TxCommand::Commit { op_id })
    }

    pub fn rollback(&self, op_id: u64) -> Result<(), ServiceError> {
        self.finish_with(TxCommand::Rollback { op_id })
    }

    /// Immediately aborts an unsettled transaction by dropping its channel (writer rolls back).
    pub fn abort(&self) {
        self.settled.store(true, Ordering::Release);
        if let Ok(mut guard) = self.sender.lock() {
            guard.take();
        }
    }

    pub fn is_timed_out(&self) -> bool {
        self.timed_out.load(Ordering::Acquire)
    }

    fn send(&self, command: TxCommand) -> Result<(), ServiceError> {
        if self.timed_out.load(Ordering::Acquire) {
            return Err(ServiceError::new(
                "E_TX_TIMEOUT",
                "SQLite transaction timed out and was rolled back",
            ));
        }
        let guard = self
            .sender
            .lock()
            .map_err(|_| ServiceError::new("InvalidState", "transaction state poisoned"))?;
        let Some(sender) = guard.as_ref() else {
            return Err(ServiceError::new(
                "InvalidState",
                "SQLite transaction is already closed",
            ));
        };
        sender.try_send(command).map_err(|err| match err {
            TrySendError::Full(_) => {
                ServiceError::new("E_LIMIT", "SQLite transaction command queue is full")
            }
            TrySendError::Disconnected(_) => {
                if self.timed_out.load(Ordering::Acquire) {
                    ServiceError::new(
                        "E_TX_TIMEOUT",
                        "SQLite transaction timed out and was rolled back",
                    )
                } else {
                    ServiceError::new("InvalidState", "SQLite transaction is no longer active")
                }
            }
        })
    }

    fn finish_with(&self, command: TxCommand) -> Result<(), ServiceError> {
        if self.timed_out.load(Ordering::Acquire) {
            return Err(ServiceError::new(
                "E_TX_TIMEOUT",
                "SQLite transaction timed out and was rolled back",
            ));
        }
        let sender = {
            let mut guard = self
                .sender
                .lock()
                .map_err(|_| ServiceError::new("InvalidState", "transaction state poisoned"))?;
            self.settled.store(true, Ordering::Release);
            guard.take()
        };
        let Some(sender) = sender else {
            return Err(ServiceError::new(
                "InvalidState",
                "SQLite transaction is already closed",
            ));
        };
        sender.try_send(command).map_err(|err| match err {
            TrySendError::Full(_) => {
                ServiceError::new("E_LIMIT", "SQLite transaction command queue is full")
            }
            TrySendError::Disconnected(_) => {
                if self.timed_out.load(Ordering::Acquire) {
                    ServiceError::new(
                        "E_TX_TIMEOUT",
                        "SQLite transaction timed out and was rolled back",
                    )
                } else {
                    ServiceError::new("InvalidState", "SQLite transaction is no longer active")
                }
            }
        })
    }
}

/// Asynchronous SQLite database with a dedicated serialized writer thread and concurrent reader pool.
pub struct SqliteDatabase {
    writer: Mutex<Option<SyncSender<WriteJob>>>,
    readers: Mutex<Option<SyncSender<ReadJob>>>,
    events: Mutex<Receiver<SqliteEvent>>,
    closed: AtomicBool,
}

impl SqliteDatabase {
    /// Opens a durable WAL-mode SQLite database at `path` (creating parent directories with 0700).
    pub fn open_file(
        path: &Path,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, ServiceError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
        let writer_conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(map_sqlite_error)?;
        configure_writer(&writer_conn, true)?;

        let mut reader_conns = Vec::with_capacity(READER_POOL_SIZE);
        for _ in 0..READER_POOL_SIZE {
            let conn = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(map_sqlite_error)?;
            configure_reader(&conn)?;
            reader_conns.push(conn);
        }
        Self::spawn(writer_conn, reader_conns, wake)
    }

    /// Allocates a process-unique scope identifier for isolating in-memory preview databases per session.
    pub fn next_memory_scope() -> u64 {
        MEMORY_DB_COUNTER.fetch_add(1, Ordering::Relaxed)
    }

    /// Opens an isolated in-memory SQLite database (zero disk files) and runs optional seed SQL.
    pub fn open_memory(
        name: &str,
        seed_statements: &[String],
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, ServiceError> {
        Self::open_memory_in_scope(Self::next_memory_scope(), name, seed_statements, wake)
    }

    /// Opens a session-scoped in-memory SQLite database shared across handles with the same `(scope, name)`.
    pub fn open_memory_in_scope(
        scope: u64,
        name: &str,
        seed_statements: &[String],
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, ServiceError> {
        if !is_valid_database_name(name) {
            return Err(ServiceError::new(
                "InvalidPath",
                format!("invalid SQLite database name: {name:?}"),
            ));
        }
        let uri = format!("file:zgui_preview_{scope}_{name}?mode=memory&cache=shared");
        let writer_conn = Connection::open_with_flags(
            &uri,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(map_sqlite_error)?;
        configure_writer(&writer_conn, false)?;
        for statement in seed_statements {
            writer_conn
                .execute_batch(statement)
                .map_err(map_sqlite_error)?;
        }

        let mut reader_conns = Vec::with_capacity(READER_POOL_SIZE);
        for _ in 0..READER_POOL_SIZE {
            let conn = Connection::open_with_flags(
                &uri,
                OpenFlags::SQLITE_OPEN_READ_ONLY
                    | OpenFlags::SQLITE_OPEN_URI
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(map_sqlite_error)?;
            configure_reader(&conn)?;
            reader_conns.push(conn);
        }
        Self::spawn(writer_conn, reader_conns, wake)
    }

    fn spawn(
        writer_conn: Connection,
        reader_conns: Vec<Connection>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, ServiceError> {
        let (event_tx, event_rx) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let (write_tx, write_rx) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let (read_tx, read_rx) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);

        let writer_events = event_tx.clone();
        let writer_wake = wake.clone();
        thread::Builder::new()
            .name("zgui-sqlite-writer".to_owned())
            .spawn(move || writer_loop(writer_conn, write_rx, writer_events, writer_wake))?;

        let shared_read_rx = Arc::new(Mutex::new(read_rx));
        for (index, conn) in reader_conns.into_iter().enumerate() {
            let rx = shared_read_rx.clone();
            let tx = event_tx.clone();
            let wake_cb = wake.clone();
            thread::Builder::new()
                .name(format!("zgui-sqlite-reader-{index}"))
                .spawn(move || reader_loop(conn, rx, tx, wake_cb))?;
        }

        Ok(Self {
            writer: Mutex::new(Some(write_tx)),
            readers: Mutex::new(Some(read_tx)),
            events: Mutex::new(event_rx),
            closed: AtomicBool::new(false),
        })
    }

    /// Submits a read-only parameterized query to the concurrent reader pool.
    pub fn query(
        &self,
        op_id: u64,
        sql: String,
        params: SqlParams,
        max_rows: usize,
    ) -> Result<(), ServiceError> {
        let guard = self
            .readers
            .lock()
            .map_err(|_| ServiceError::new("InvalidState", "database state poisoned"))?;
        let Some(sender) = guard.as_ref() else {
            return Err(ServiceError::new("InvalidState", "SQLite database is closed"));
        };
        sender
            .try_send(ReadJob {
                op_id,
                sql,
                params,
                max_rows: clamp_max_rows(max_rows),
            })
            .map_err(|err| match err {
                TrySendError::Full(_) => {
                    ServiceError::new("E_LIMIT", "SQLite read queue is full")
                }
                TrySendError::Disconnected(_) => {
                    ServiceError::new("InvalidState", "SQLite reader pool has stopped")
                }
            })
    }

    /// Submits a parameterized write statement to the serialized writer thread.
    pub fn execute(&self, op_id: u64, sql: String, params: SqlParams) -> Result<(), ServiceError> {
        self.send_write(WriteJob::Execute { op_id, sql, params })
    }

    /// Submits an atomic batch of statements to the serialized writer thread.
    pub fn batch(
        &self,
        op_id: u64,
        statements: Vec<SqlBatchStatement>,
    ) -> Result<(), ServiceError> {
        self.send_write(WriteJob::Batch { op_id, statements })
    }

    /// Begins an interactive write transaction with a host-enforced hard timeout.
    pub fn begin_transaction(
        &self,
        op_id: u64,
        tx_id: u64,
        timeout: Duration,
    ) -> Result<SqliteTransaction, ServiceError> {
        let clamped = timeout.clamp(
            Duration::from_millis(1),
            Duration::from_millis(MAX_TX_TIMEOUT_MS),
        );
        let (tx_sender, tx_receiver) = mpsc::sync_channel(64);
        let timed_out = Arc::new(AtomicBool::new(false));
        self.send_write(WriteJob::BeginTx {
            op_id,
            tx_id,
            timeout: clamped,
            timed_out: timed_out.clone(),
            commands: tx_receiver,
        })?;
        Ok(SqliteTransaction {
            sender: Mutex::new(Some(tx_sender)),
            timed_out,
            settled: AtomicBool::new(false),
        })
    }

    /// Polls one completed event without blocking.
    pub fn try_recv(&self) -> Option<SqliteEvent> {
        self.events.lock().ok()?.try_recv().ok()
    }

    /// Closes admission and signals worker threads to exit without blocking the caller.
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Ok(mut guard) = self.writer.lock() {
            guard.take();
        }
        if let Ok(mut guard) = self.readers.lock() {
            guard.take();
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn send_write(&self, job: WriteJob) -> Result<(), ServiceError> {
        let guard = self
            .writer
            .lock()
            .map_err(|_| ServiceError::new("InvalidState", "database state poisoned"))?;
        let Some(sender) = guard.as_ref() else {
            return Err(ServiceError::new("InvalidState", "SQLite database is closed"));
        };
        sender.try_send(job).map_err(|err| match err {
            TrySendError::Full(_) => ServiceError::new("E_LIMIT", "SQLite write queue is full"),
            TrySendError::Disconnected(_) => {
                ServiceError::new("InvalidState", "SQLite writer has stopped")
            }
        })
    }
}

impl Drop for SqliteDatabase {
    fn drop(&mut self) {
        self.close();
    }
}

fn configure_writer(conn: &Connection, enable_wal: bool) -> Result<(), ServiceError> {
    if enable_wal {
        let _: String = conn
            .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
            .map_err(map_sqlite_error)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(map_sqlite_error)?;
    }
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(map_sqlite_error)?;
    conn.pragma_update(None, "busy_timeout", 5000_i64)
        .map_err(map_sqlite_error)?;
    Ok(())
}

fn configure_reader(conn: &Connection) -> Result<(), ServiceError> {
    conn.pragma_update(None, "query_only", "ON")
        .map_err(map_sqlite_error)?;
    conn.pragma_update(None, "busy_timeout", 5000_i64)
        .map_err(map_sqlite_error)?;
    Ok(())
}

fn clamp_max_rows(max_rows: usize) -> usize {
    if max_rows == 0 {
        MAX_QUERY_ROWS
    } else {
        max_rows.min(MAX_QUERY_ROWS)
    }
}

fn emit(tx: &SyncSender<SqliteEvent>, wake: &Arc<dyn Fn() + Send + Sync>, event: SqliteEvent) {
    if tx.send(event).is_ok() {
        wake();
    }
}

fn reader_loop(
    conn: Connection,
    rx: Arc<Mutex<Receiver<ReadJob>>>,
    events: SyncSender<SqliteEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    loop {
        let job = {
            let Ok(guard) = rx.lock() else { return };
            match guard.recv() {
                Ok(job) => job,
                Err(_) => return,
            }
        };
        let result = run_query(&conn, &job.sql, &job.params, job.max_rows);
        emit(
            &events,
            &wake,
            SqliteEvent::Query {
                op_id: job.op_id,
                result,
            },
        );
    }
}

fn writer_loop(
    mut conn: Connection,
    rx: Receiver<WriteJob>,
    events: SyncSender<SqliteEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    while let Ok(job) = rx.recv() {
        match job {
            WriteJob::Execute { op_id, sql, params } => {
                let result = run_execute(&conn, &sql, &params);
                emit(&events, &wake, SqliteEvent::Execute { op_id, result });
            }
            WriteJob::Batch { op_id, statements } => {
                let result = run_batch(&mut conn, &statements);
                emit(&events, &wake, SqliteEvent::Batch { op_id, result });
            }
            WriteJob::BeginTx {
                op_id,
                tx_id,
                timeout,
                timed_out,
                commands,
            } => {
                run_interactive_tx(
                    &conn, op_id, tx_id, timeout, timed_out, commands, &events, &wake,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_interactive_tx(
    conn: &Connection,
    begin_op_id: u64,
    tx_id: u64,
    timeout: Duration,
    timed_out: Arc<AtomicBool>,
    commands: Receiver<TxCommand>,
    events: &SyncSender<SqliteEvent>,
    wake: &Arc<dyn Fn() + Send + Sync>,
) {
    if let Err(err) = conn.execute_batch("BEGIN IMMEDIATE") {
        emit(
            events,
            wake,
            SqliteEvent::TxBegun {
                op_id: begin_op_id,
                tx_id,
                result: Err(map_sqlite_error(err)),
            },
        );
        return;
    }
    let deadline = Instant::now() + timeout;
    emit(
        events,
        wake,
        SqliteEvent::TxBegun {
            op_id: begin_op_id,
            tx_id,
            result: Ok(()),
        },
    );

    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            rollback_quietly(conn);
            timed_out.store(true, Ordering::Release);
            emit(events, wake, SqliteEvent::TxTimedOut { tx_id });
            return;
        };
        match commands.recv_timeout(remaining) {
            Ok(TxCommand::Query {
                op_id,
                sql,
                params,
                max_rows,
            }) => {
                let result = run_query(conn, &sql, &params, max_rows);
                emit(events, wake, SqliteEvent::Query { op_id, result });
            }
            Ok(TxCommand::Execute { op_id, sql, params }) => {
                let result = run_execute(conn, &sql, &params);
                emit(events, wake, SqliteEvent::Execute { op_id, result });
            }
            Ok(TxCommand::Commit { op_id }) => {
                let result = conn
                    .execute_batch("COMMIT")
                    .map_err(|err| {
                        rollback_quietly(conn);
                        map_sqlite_error(err)
                    });
                emit(events, wake, SqliteEvent::TxSettled { op_id, tx_id, result });
                return;
            }
            Ok(TxCommand::Rollback { op_id }) => {
                let result = conn.execute_batch("ROLLBACK").map_err(map_sqlite_error);
                emit(events, wake, SqliteEvent::TxSettled { op_id, tx_id, result });
                return;
            }
            Err(RecvTimeoutError::Timeout) => {
                rollback_quietly(conn);
                timed_out.store(true, Ordering::Release);
                emit(events, wake, SqliteEvent::TxTimedOut { tx_id });
                return;
            }
            Err(RecvTimeoutError::Disconnected) => {
                rollback_quietly(conn);
                return;
            }
        }
    }
}

fn rollback_quietly(conn: &Connection) {
    if !conn.is_autocommit() {
        let _ = conn.execute_batch("ROLLBACK");
    }
}

fn bind_params(stmt: &mut rusqlite::Statement<'_>, params: &SqlParams) -> Result<(), ServiceError> {
    match params {
        SqlParams::None => {
            if stmt.parameter_count() > 0 {
                return Err(ServiceError::new(
                    "InvalidArgument",
                    format!(
                        "SQL statement expects {} parameters, got 0",
                        stmt.parameter_count()
                    ),
                ));
            }
            Ok(())
        }
        SqlParams::Positional(values) => {
            if values.len() != stmt.parameter_count() {
                return Err(ServiceError::new(
                    "InvalidArgument",
                    format!(
                        "SQL statement expects {} positional parameters, got {}",
                        stmt.parameter_count(),
                        values.len()
                    ),
                ));
            }
            for (index, value) in values.iter().enumerate() {
                stmt.raw_bind_parameter(index + 1, value.to_rusqlite())
                    .map_err(map_sqlite_error)?;
            }
            Ok(())
        }
        SqlParams::Named(entries) => {
            for (name, value) in entries {
                let prefixed;
                let key = if name.starts_with([':', '@', '$']) {
                    name.as_str()
                } else {
                    prefixed = format!(":{name}");
                    prefixed.as_str()
                };
                let index = stmt.parameter_index(key).map_err(map_sqlite_error)?.ok_or_else(
                    || {
                        ServiceError::new(
                            "InvalidArgument",
                            format!("unknown named SQL parameter {name:?}"),
                        )
                    },
                )?;
                stmt.raw_bind_parameter(index, value.to_rusqlite())
                    .map_err(map_sqlite_error)?;
            }
            Ok(())
        }
    }
}

fn run_query(
    conn: &Connection,
    sql: &str,
    params: &SqlParams,
    max_rows: usize,
) -> Result<Vec<SqlRow>, ServiceError> {
    let mut stmt = conn.prepare(sql).map_err(map_sqlite_error)?;
    if !stmt.readonly() {
        return Err(ServiceError::new(
            "ReadOnly",
            "query() only accepts read-only SQL statements; use execute() for writes",
        ));
    }
    bind_params(&mut stmt, params)?;
    let column_names: Vec<String> = stmt
        .column_names()
        .map(str::to_owned)
        .collect();
    let col_count = column_names.len();
    let mut rows_iter = stmt.raw_query();
    let mut out = Vec::new();
    let mut total_bytes: usize = 0;

    while let Some(row) = rows_iter.next().map_err(map_sqlite_error)? {
        if out.len() >= max_rows {
            return Err(ServiceError::new(
                "E_LIMIT",
                format!("SQL query exceeded row limit of {max_rows}"),
            ));
        }
        let mut record = Vec::with_capacity(col_count);
        for (idx, col_name) in column_names.iter().enumerate() {
            let val = match row.get_ref(idx).map_err(map_sqlite_error)? {
                ValueRef::Null => SqlValue::Null,
                ValueRef::Integer(v) => SqlValue::Integer(v),
                ValueRef::Real(v) => SqlValue::Real(v),
                ValueRef::Text(bytes) => {
                    let text = std::str::from_utf8(bytes)
                        .map_err(|e| ServiceError::new("InvalidUtf8", e.to_string()))?;
                    SqlValue::Text(text.to_owned())
                }
                ValueRef::Blob(bytes) => SqlValue::Blob(bytes.to_vec()),
            };
            total_bytes = total_bytes
                .saturating_add(col_name.len())
                .saturating_add(val.approx_bytes());
            if total_bytes > MAX_RESULT_BYTES {
                return Err(ServiceError::new(
                    "E_LIMIT",
                    format!("SQL query result exceeded byte limit of {MAX_RESULT_BYTES} bytes"),
                ));
            }
            record.push((col_name.clone(), val));
        }
        out.push(record);
    }
    Ok(out)
}

fn run_execute(
    conn: &Connection,
    sql: &str,
    params: &SqlParams,
) -> Result<SqlExecuteResult, ServiceError> {
    let mut stmt = conn.prepare(sql).map_err(map_sqlite_error)?;
    bind_params(&mut stmt, params)?;
    let rows_affected = stmt.raw_execute().map_err(map_sqlite_error)? as u64;
    let rowid = conn.last_insert_rowid();
    let last_insert_rowid = (rows_affected > 0 && rowid != 0).then_some(rowid);
    Ok(SqlExecuteResult {
        rows_affected,
        last_insert_rowid,
    })
}

fn run_batch(
    conn: &mut Connection,
    statements: &[SqlBatchStatement],
) -> Result<Vec<SqlExecuteResult>, ServiceError> {
    if statements.len() > MAX_BATCH_STATEMENTS {
        return Err(ServiceError::new(
            "E_LIMIT",
            format!("SQL batch exceeds maximum of {MAX_BATCH_STATEMENTS} statements"),
        ));
    }
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(map_sqlite_error)?;
    let mut results = Vec::with_capacity(statements.len());
    for item in statements {
        match run_execute(conn, &item.sql, &item.params) {
            Ok(res) => results.push(res),
            Err(err) => {
                rollback_quietly(conn);
                return Err(err);
            }
        }
    }
    if let Err(err) = conn.execute_batch("COMMIT") {
        rollback_quietly(conn);
        return Err(map_sqlite_error(err));
    }
    Ok(results)
}

fn map_sqlite_error(error: rusqlite::Error) -> ServiceError {
    match &error {
        rusqlite::Error::MultipleStatement => {
            ServiceError::new("InvalidArgument", "multiple SQL statements are not allowed in a single call")
        }
        rusqlite::Error::SqliteFailure(ffi_err, _) => {
            use rusqlite::ErrorCode;
            let code = match ffi_err.code {
                ErrorCode::ConstraintViolation => "Constraint",
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => "Busy",
                ErrorCode::ReadOnly => "ReadOnly",
                ErrorCode::DiskFull | ErrorCode::TooBig => "E_LIMIT",
                _ => "SqlError",
            };
            ServiceError::new(code, error.to_string())
        }
        _ => ServiceError::new("SqlError", error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop_wake() -> Arc<dyn Fn() + Send + Sync> {
        Arc::new(|| {})
    }

    fn wait_event(db: &SqliteDatabase, timeout: Duration) -> SqliteEvent {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Some(ev) = db.try_recv() {
                return ev;
            }
            thread::sleep(Duration::from_millis(2));
        }
        panic!("timed out waiting for SQLite event");
    }

    #[test]
    fn database_name_and_path_sanitization_rejects_traversal() {
        assert!(is_valid_database_name("main"));
        assert!(is_valid_database_name("notes_v2-cache"));
        assert!(!is_valid_database_name(""));
        assert!(!is_valid_database_name("../escape"));
        assert!(!is_valid_database_name("a/b"));
        assert!(!is_valid_database_name("-leading-dash"));
        assert!(!is_valid_database_name("has space"));

        assert!(is_safe_app_id("md-editor"));
        assert!(is_safe_app_id("org.example.studio"));
        assert!(!is_safe_app_id(".."));
        assert!(!is_safe_app_id("foo/bar"));
        assert!(!is_safe_app_id(" leading"));
        assert!(resolve_database_path("../bad", "main").is_err());
        assert!(resolve_database_path("good-app", "../bad").is_err());
    }

    #[test]
    fn wal_allows_concurrent_reads_while_writer_holds_open_transaction() {
        let dir = env::temp_dir().join(format!(
            "zgui-sqlite-wal-test-{}",
            MEMORY_DB_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let path = dir.join("main.sqlite3");
        let db = SqliteDatabase::open_file(&path, noop_wake()).expect("open file db");

        db.execute(
            1,
            "CREATE TABLE items (id INTEGER PRIMARY KEY, label TEXT NOT NULL)".into(),
            SqlParams::None,
        )
        .unwrap();
        assert!(matches!(
            wait_event(&db, Duration::from_secs(2)),
            SqliteEvent::Execute { op_id: 1, result: Ok(_) }
        ));

        db.execute(
            2,
            "INSERT INTO items (label) VALUES (?1)".into(),
            SqlParams::Positional(vec![SqlValue::Text("committed".into())]),
        )
        .unwrap();
        assert!(matches!(
            wait_event(&db, Duration::from_secs(2)),
            SqliteEvent::Execute { op_id: 2, result: Ok(_) }
        ));

        // Open an interactive write transaction and insert an uncommitted row.
        let tx = db
            .begin_transaction(3, 10, Duration::from_secs(5))
            .expect("begin tx");
        assert!(matches!(
            wait_event(&db, Duration::from_secs(2)),
            SqliteEvent::TxBegun { op_id: 3, tx_id: 10, result: Ok(()) }
        ));

        tx.execute(
            4,
            "INSERT INTO items (label) VALUES (:label)".into(),
            SqlParams::Named(vec![("label".into(), SqlValue::Text("uncommitted".into()))]),
        )
        .unwrap();
        assert!(matches!(
            wait_event(&db, Duration::from_secs(2)),
            SqliteEvent::Execute { op_id: 4, result: Ok(_) }
        ));

        // While the writer holds the transaction, the reader pool reads the committed WAL snapshot concurrently!
        db.query(
            5,
            "SELECT label FROM items ORDER BY id".into(),
            SqlParams::None,
            MAX_QUERY_ROWS,
        )
        .unwrap();
        match wait_event(&db, Duration::from_secs(2)) {
            SqliteEvent::Query {
                op_id: 5,
                result: Ok(rows),
            } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0][0].1, SqlValue::Text("committed".into()));
            }
            other => panic!("expected committed WAL snapshot on reader pool, got {other:?}"),
        }

        // Meanwhile tx.query sees both committed and uncommitted rows.
        tx.query(
            6,
            "SELECT label FROM items ORDER BY id".into(),
            SqlParams::None,
            MAX_QUERY_ROWS,
        )
        .unwrap();
        match wait_event(&db, Duration::from_secs(2)) {
            SqliteEvent::Query {
                op_id: 6,
                result: Ok(rows),
            } => {
                assert_eq!(rows.len(), 2);
            }
            other => panic!("expected 2 rows inside tx, got {other:?}"),
        }

        tx.commit(7).unwrap();
        assert!(matches!(
            wait_event(&db, Duration::from_secs(2)),
            SqliteEvent::TxSettled { op_id: 7, tx_id: 10, result: Ok(()) }
        ));

        db.close();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn interactive_transaction_timeout_auto_rolls_back_and_releases_writer_lock() {
        let db = SqliteDatabase::open_memory(
            "tx_timeout",
            &["CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)".into()],
            noop_wake(),
        )
        .expect("open memory db");

        let tx = db
            .begin_transaction(1, 42, Duration::from_millis(60))
            .expect("begin tx");
        assert!(matches!(
            wait_event(&db, Duration::from_secs(2)),
            SqliteEvent::TxBegun { op_id: 1, tx_id: 42, result: Ok(()) }
        ));

        tx.execute(
            2,
            "INSERT INTO notes (body) VALUES (?1)".into(),
            SqlParams::Positional(vec![SqlValue::Text("should-rollback".into())]),
        )
        .unwrap();
        assert!(matches!(
            wait_event(&db, Duration::from_secs(2)),
            SqliteEvent::Execute { op_id: 2, result: Ok(_) }
        ));

        // Do not commit: wait for host hard timeout.
        assert!(matches!(
            wait_event(&db, Duration::from_secs(2)),
            SqliteEvent::TxTimedOut { tx_id: 42 }
        ));
        assert!(tx.is_timed_out());

        // Subsequent operations on the expired transaction fail with E_TX_TIMEOUT.
        let err = tx.commit(3).expect_err("commit after timeout must fail");
        assert_eq!(err.code, "E_TX_TIMEOUT");

        // Writer lock is released and table is empty (rolled back).
        db.execute(
            4,
            "INSERT INTO notes (body) VALUES (?1)".into(),
            SqlParams::Positional(vec![SqlValue::Text("after-timeout".into())]),
        )
        .unwrap();
        assert!(matches!(
            wait_event(&db, Duration::from_secs(2)),
            SqliteEvent::Execute { op_id: 4, result: Ok(_) }
        ));

        db.query(5, "SELECT body FROM notes".into(), SqlParams::None, 100)
            .unwrap();
        match wait_event(&db, Duration::from_secs(2)) {
            SqliteEvent::Query {
                op_id: 5,
                result: Ok(rows),
            } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0][0].1, SqlValue::Text("after-timeout".into()));
            }
            other => panic!("unexpected query outcome: {other:?}"),
        }
        db.close();
    }

    #[test]
    fn fts5_full_text_search_and_readonly_query_guard_work() {
        let db = SqliteDatabase::open_memory(
            "fts5_test",
            &[
                "CREATE VIRTUAL TABLE docs_fts USING fts5(title, body)".into(),
                "INSERT INTO docs_fts (title, body) VALUES ('ADR 0004', 'Local-first Markdown Studio with SQLite FTS5')".into(),
                "INSERT INTO docs_fts (title, body) VALUES ('Wayland', 'Clipboard image PNG negotiation')".into(),
            ],
            noop_wake(),
        )
        .expect("open fts5 memory db");

        db.query(
            1,
            "SELECT title FROM docs_fts WHERE docs_fts MATCH ?1".into(),
            SqlParams::Positional(vec![SqlValue::Text("SQLite".into())]),
            100,
        )
        .unwrap();
        match wait_event(&db, Duration::from_secs(2)) {
            SqliteEvent::Query {
                op_id: 1,
                result: Ok(rows),
            } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0][0].1, SqlValue::Text("ADR 0004".into()));
            }
            other => panic!("expected FTS5 match, got {other:?}"),
        }

        // Write attempted via query() is rejected as ReadOnly.
        db.query(
            2,
            "DELETE FROM docs_fts".into(),
            SqlParams::None,
            100,
        )
        .unwrap();
        match wait_event(&db, Duration::from_secs(2)) {
            SqliteEvent::Query {
                op_id: 2,
                result: Err(err),
            } => assert_eq!(err.code, "ReadOnly"),
            other => panic!("expected ReadOnly error, got {other:?}"),
        }
        db.close();
    }
}
