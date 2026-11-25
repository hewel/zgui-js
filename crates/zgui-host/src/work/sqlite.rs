//! Session-owned app-scoped SQLite handles and async worker completions.
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use host_services::sqlite::{
    DEFAULT_TX_TIMEOUT_MS, MAX_BATCH_STATEMENTS, MAX_QUERY_ROWS, MAX_SQL_BYTES, MAX_TX_TIMEOUT_MS,
    SqlBatchStatement, SqlExecuteResult, SqlParams, SqlRow, SqlValue, SqliteDatabase, SqliteEvent,
    SqliteTransaction, is_valid_database_name, resolve_database_path,
};
use rquickjs::{
    Array, Ctx, Exception, Function, IntoJs, Object, Persistent, TypedArray, Value,
    function::{Func, Opt},
};

use super::{Mode, Work, data_property, owner, service_error};
use crate::{assets::{AssetSource, fixture::FixtureRuntime}, package::Package};

const MAX_OPEN_DATABASES: usize = 16;
const MAX_PENDING_SQL_OPS: usize = 256;
const MAX_BLOB_PARAM_BYTES: usize = 8 * 1024 * 1024;
const MAX_SAFE_INT_F64: f64 = 9_007_199_254_740_991.0;

struct OpenDb {
    db: SqliteDatabase,
}

struct ActiveTx {
    db_id: u64,
    tx: SqliteTransaction,
    timeout_reject: Option<Persistent<Function<'static>>>,
}

struct PendingSqlOp {
    db_id: u64,
    resolve: Persistent<Function<'static>>,
    reject: Persistent<Function<'static>>,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct SqliteCounts {
    pub(crate) databases: usize,
    pub(crate) transactions: usize,
}

pub(super) struct SqliteState {
    package: Option<Rc<Package>>,
    capabilities: Arc<crate::package::Capabilities>,
    fixture: Option<Rc<FixtureRuntime>>,
    memory_scope: u64,
    seeded_memory_dbs: RefCell<std::collections::HashSet<String>>,
    databases: RefCell<BTreeMap<u64, OpenDb>>,
    transactions: RefCell<BTreeMap<u64, ActiveTx>>,
    pending: RefCell<HashMap<u64, PendingSqlOp>>,
    ready: RefCell<Option<(u64, SqliteEvent)>>,
    next_id: Cell<u64>,
}

impl SqliteState {
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
            memory_scope: SqliteDatabase::next_memory_scope(),
            seeded_memory_dbs: RefCell::new(std::collections::HashSet::new()),
            databases: RefCell::new(BTreeMap::new()),
            transactions: RefCell::new(BTreeMap::new()),
            pending: RefCell::new(HashMap::new()),
            ready: RefCell::new(None),
            next_id: Cell::new(1),
        }
    }

    fn alloc_id(&self, ctx: &Ctx<'_>) -> rquickjs::Result<u64> {
        let id = self.next_id.get();
        let next = id
            .checked_add(1)
            .ok_or_else(|| Exception::throw_range(ctx, "SQLite operation identifiers exhausted"))?;
        self.next_id.set(next);
        Ok(id)
    }

    pub(super) fn counts(&self) -> SqliteCounts {
        SqliteCounts {
            databases: self.databases.borrow().len(),
            transactions: self.transactions.borrow().len(),
        }
    }

    pub(super) fn pending_count(&self) -> usize {
        self.pending.borrow().len()
    }

    fn take_event(&self) -> Option<(u64, SqliteEvent)> {
        if let Some(ev) = self.ready.borrow_mut().take() {
            return Some(ev);
        }
        let databases = self.databases.borrow();
        for (&db_id, open) in databases.iter() {
            if let Some(ev) = open.db.try_recv() {
                return Some((db_id, ev));
            }
        }
        None
    }

    pub(super) fn has_events(&self) -> bool {
        if self.ready.borrow().is_some() {
            return true;
        }
        if let Some(ev) = self.take_event() {
            *self.ready.borrow_mut() = Some(ev);
            return true;
        }
        false
    }

    pub(super) fn clear(&self) {
        for (_, tx) in std::mem::take(&mut *self.transactions.borrow_mut()) {
            tx.tx.abort();
        }
        for (_, open) in std::mem::take(&mut *self.databases.borrow_mut()) {
            open.db.close();
        }
        self.pending.borrow_mut().clear();
        self.ready.borrow_mut().take();
    }
}

impl Work {
    pub(super) fn install_sqlite<'js>(
        self: &Rc<Self>,
        services: &Object<'js>,
    ) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        services.prop(
            "openDatabase",
            data_property(Func::from(
                move |ctx: Ctx<'js>, name: Opt<Value<'js>>, extra: Opt<Value<'js>>| {
                    if extra.0.is_some() {
                        return Err(Exception::throw_type(
                            &ctx,
                            "openDatabase requires exactly one string database name",
                        ));
                    }
                    let name = name.0.ok_or_else(|| {
                        Exception::throw_type(&ctx, "openDatabase requires a database name string")
                    })?;
                    owner(&weak, &ctx)?.open_sqlite_database(&ctx, name)
                },
            )),
        )
    }

    pub(crate) fn sqlite_counts(&self) -> SqliteCounts {
        self.sqlite.counts()
    }

    pub(crate) fn has_pending_sqlite_ops(&self) -> bool {
        self.sqlite.pending_count() > 0
    }

    pub(crate) fn has_sqlite_events(&self) -> bool {
        self.sqlite.has_events()
    }

    pub(crate) fn next_sqlite_event(&self) -> Option<(u64, SqliteEvent)> {
        if self.mode.get() != Mode::Mounted {
            return None;
        }
        self.sqlite.take_event()
    }

    fn open_sqlite_database<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        name_val: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        let name = read_bounded_utf8(ctx, name_val, 128, "database name")?;
        if !is_valid_database_name(&name) {
            return Err(throw_service_error(
                ctx,
                "InvalidPath",
                &format!("invalid SQLite database name: {name:?}"),
            ));
        }
        let Some(package) = self.sqlite.package.as_ref() else {
            return Err(throw_service_error(
                ctx,
                "E_CAPABILITY",
                "SQLite requires a packaged application with capabilities.sqlite.databases",
            ));
        };
        if !self
            .sqlite
            .capabilities
            .sqlite
            .databases
            .iter()
            .any(|declared| declared == &name)
        {
            return Err(throw_service_error(
                ctx,
                "E_CAPABILITY",
                &format!("SQLite database {name:?} is not declared in capabilities.sqlite.databases"),
            ));
        }
        if self.sqlite.databases.borrow().len() >= MAX_OPEN_DATABASES {
            return Err(Exception::throw_range(
                ctx,
                "maximum open SQLite database limit reached",
            ));
        }

        let db = if let Some(fixture) = &self.sqlite.fixture {
            let needs_seed = !self.sqlite.seeded_memory_dbs.borrow().contains(&name);
            let seeds = if needs_seed {
                fixture.sqlite_seed_statements(&name)
            } else {
                Vec::new()
            };
            let opened = SqliteDatabase::open_memory_in_scope(
                self.sqlite.memory_scope,
                &name,
                &seeds,
                self.wake.clone(),
            );
            if opened.is_ok() && needs_seed {
                self.sqlite.seeded_memory_dbs.borrow_mut().insert(name.clone());
            }
            opened
        } else {
            let path = resolve_database_path(&package.manifest.id, &name)
                .map_err(|err| throw_service_error(ctx, err.code, &err.message))?;
            SqliteDatabase::open_file(&path, self.wake.clone())
        }
        .map_err(|err| throw_service_error(ctx, err.code, &err.message))?;

        let db_id = self.sqlite.alloc_id(ctx)?;
        self.sqlite
            .databases
            .borrow_mut()
            .insert(db_id, OpenDb { db });

        let handle = Object::new(ctx.clone())?;
        handle.prop("name", data_property(name.as_str()))?;

        let weak = Rc::downgrade(self);
        handle.prop(
            "query",
            data_property(Func::from(
                move |ctx: Ctx<'js>,
                      sql: Value<'js>,
                      params: Opt<Value<'js>>,
                      options: Opt<Value<'js>>| {
                    owner(&weak, &ctx)?.sqlite_query(&ctx, db_id, sql, params.0, options.0)
                },
            )),
        )?;

        let weak = Rc::downgrade(self);
        handle.prop(
            "execute",
            data_property(Func::from(
                move |ctx: Ctx<'js>, sql: Value<'js>, params: Opt<Value<'js>>| {
                    owner(&weak, &ctx)?.sqlite_execute(&ctx, db_id, sql, params.0)
                },
            )),
        )?;

        let weak = Rc::downgrade(self);
        handle.prop(
            "batch",
            data_property(Func::from(move |ctx: Ctx<'js>, statements: Value<'js>| {
                owner(&weak, &ctx)?.sqlite_batch(&ctx, db_id, statements)
            })),
        )?;

        let weak = Rc::downgrade(self);
        handle.prop(
            "__beginTransaction",
            data_property(Func::from(move |ctx: Ctx<'js>, options: Opt<Value<'js>>| {
                owner(&weak, &ctx)?.sqlite_begin_transaction(&ctx, db_id, options.0)
            })),
        )?;

        let weak = Rc::downgrade(self);
        handle.prop(
            "close",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                if let Some(work) = weak.upgrade() {
                    work.close_sqlite_database(&ctx, db_id)?;
                }
                Ok::<(), rquickjs::Error>(())
            })),
        )?;

        Ok(handle)
    }

    fn sqlite_request_object<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        db_id: u64,
        submit: impl FnOnce(u64) -> Result<(), host_services::ServiceError>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        if self.sqlite.pending.borrow().len() >= MAX_PENDING_SQL_OPS {
            return Err(Exception::throw_range(
                ctx,
                "concurrent SQLite operation limit reached",
            ));
        }
        let op_id = self.sqlite.alloc_id(ctx)?;
        let (promise, resolve, reject) = ctx.promise()?;
        let object = Object::new(ctx.clone())?;
        object.prop("promise", data_property(promise))?;
        let weak = Rc::downgrade(self);
        object.prop(
            "cancel",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                owner(&weak, &ctx)?.cancel_sqlite_op(&ctx, op_id)
            })),
        )?;
        if let Err(err) = submit(op_id) {
            let js_err = service_error(ctx, err.code, &err.message)?;
            reject.call::<_, ()>((js_err,))?;
            return Ok(object);
        }
        self.sqlite.pending.borrow_mut().insert(
            op_id,
            PendingSqlOp {
                db_id,
                resolve: Persistent::save(ctx, resolve),
                reject: Persistent::save(ctx, reject),
            },
        );
        Ok(object)
    }

    fn sqlite_query<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        db_id: u64,
        sql_val: Value<'js>,
        params_val: Option<Value<'js>>,
        options_val: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let sql = read_bounded_utf8(ctx, sql_val, MAX_SQL_BYTES, "SQL query")?;
        let params = parse_sql_params(ctx, params_val)?;
        let max_rows = parse_query_max_rows(ctx, options_val)?;
        self.sqlite_request_object(ctx, db_id, |op_id| {
            let dbs = self.sqlite.databases.borrow();
            let Some(open) = dbs.get(&db_id) else {
                return Err(host_services::ServiceError {
                    code: "InvalidState",
                    message: "SQLite database is closed".into(),
                });
            };
            open.db.query(op_id, sql, params, max_rows)
        })
    }

    fn sqlite_execute<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        db_id: u64,
        sql_val: Value<'js>,
        params_val: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let sql = read_bounded_utf8(ctx, sql_val, MAX_SQL_BYTES, "SQL statement")?;
        let params = parse_sql_params(ctx, params_val)?;
        self.sqlite_request_object(ctx, db_id, |op_id| {
            let dbs = self.sqlite.databases.borrow();
            let Some(open) = dbs.get(&db_id) else {
                return Err(host_services::ServiceError {
                    code: "InvalidState",
                    message: "SQLite database is closed".into(),
                });
            };
            open.db.execute(op_id, sql, params)
        })
    }

    fn sqlite_batch<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        db_id: u64,
        statements_val: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        let array = statements_val
            .into_array()
            .ok_or_else(|| Exception::throw_type(ctx, "batch statements must be an array"))?;
        let len = array.len();
        if len > MAX_BATCH_STATEMENTS {
            return Err(Exception::throw_range(
                ctx,
                &format!("SQL batch exceeds maximum of {MAX_BATCH_STATEMENTS} statements"),
            ));
        }
        let mut statements = Vec::with_capacity(len);
        for idx in 0..len {
            let item: Value = array.get(idx)?;
            let obj = item.into_object().ok_or_else(|| {
                Exception::throw_type(ctx, "each SQL batch entry must be an object { sql, params? }")
            })?;
            let sql_val: Value = obj.get("sql")?;
            let sql = read_bounded_utf8(ctx, sql_val, MAX_SQL_BYTES, "SQL batch statement")?;
            let params_val: Value = obj.get("params")?;
            let params = parse_sql_params(ctx, Some(params_val))?;
            statements.push(SqlBatchStatement { sql, params });
        }
        self.sqlite_request_object(ctx, db_id, |op_id| {
            let dbs = self.sqlite.databases.borrow();
            let Some(open) = dbs.get(&db_id) else {
                return Err(host_services::ServiceError {
                    code: "InvalidState",
                    message: "SQLite database is closed".into(),
                });
            };
            open.db.batch(op_id, statements)
        })
    }

    fn sqlite_begin_transaction<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        db_id: u64,
        options_val: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        let timeout_ms = parse_tx_timeout_ms(ctx, options_val)?;
        if self.sqlite.pending.borrow().len() >= MAX_PENDING_SQL_OPS {
            return Err(Exception::throw_range(
                ctx,
                "concurrent SQLite operation limit reached",
            ));
        }
        let op_id = self.sqlite.alloc_id(ctx)?;
        let tx_id = self.sqlite.alloc_id(ctx)?;
        let (begin_promise, begin_resolve, begin_reject) = ctx.promise()?;
        let (timeout_promise, _timeout_resolve, timeout_reject) = ctx.promise()?;

        let tx_handle = {
            let dbs = self.sqlite.databases.borrow();
            let Some(open) = dbs.get(&db_id) else {
                return Err(throw_service_error(
                    ctx,
                    "InvalidState",
                    "SQLite database is closed",
                ));
            };
            open.db
                .begin_transaction(op_id, tx_id, Duration::from_millis(timeout_ms))
                .map_err(|err| throw_service_error(ctx, err.code, &err.message))?
        };

        self.sqlite.transactions.borrow_mut().insert(
            tx_id,
            ActiveTx {
                db_id,
                tx: tx_handle,
                timeout_reject: Some(Persistent::save(ctx, timeout_reject)),
            },
        );
        self.sqlite.pending.borrow_mut().insert(
            op_id,
            PendingSqlOp {
                db_id,
                resolve: Persistent::save(ctx, begin_resolve),
                reject: Persistent::save(ctx, begin_reject),
            },
        );

        let tx_obj = Object::new(ctx.clone())?;
        tx_obj.prop("beginPromise", data_property(begin_promise))?;
        tx_obj.prop("timeoutPromise", data_property(timeout_promise))?;

        let weak = Rc::downgrade(self);
        tx_obj.prop(
            "query",
            data_property(Func::from(
                move |ctx: Ctx<'js>,
                      sql: Value<'js>,
                      params: Opt<Value<'js>>,
                      options: Opt<Value<'js>>| {
                    owner(&weak, &ctx)?.sqlite_tx_query(&ctx, db_id, tx_id, sql, params.0, options.0)
                },
            )),
        )?;

        let weak = Rc::downgrade(self);
        tx_obj.prop(
            "execute",
            data_property(Func::from(
                move |ctx: Ctx<'js>, sql: Value<'js>, params: Opt<Value<'js>>| {
                    owner(&weak, &ctx)?.sqlite_tx_execute(&ctx, db_id, tx_id, sql, params.0)
                },
            )),
        )?;

        let weak = Rc::downgrade(self);
        tx_obj.prop(
            "commit",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                owner(&weak, &ctx)?.sqlite_tx_finish(&ctx, db_id, tx_id, true)
            })),
        )?;

        let weak = Rc::downgrade(self);
        tx_obj.prop(
            "rollback",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                owner(&weak, &ctx)?.sqlite_tx_finish(&ctx, db_id, tx_id, false)
            })),
        )?;

        let weak = Rc::downgrade(self);
        tx_obj.prop(
            "abort",
            data_property(Func::from(move || {
                if let Some(work) = weak.upgrade()
                    && let Some(active) = work.sqlite.transactions.borrow_mut().remove(&tx_id)
                {
                    active.tx.abort();
                }
            })),
        )?;

        Ok(tx_obj)
    }

    fn sqlite_tx_query<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        db_id: u64,
        tx_id: u64,
        sql_val: Value<'js>,
        params_val: Option<Value<'js>>,
        options_val: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let sql = read_bounded_utf8(ctx, sql_val, MAX_SQL_BYTES, "SQL transaction query")?;
        let params = parse_sql_params(ctx, params_val)?;
        let max_rows = parse_query_max_rows(ctx, options_val)?;
        self.sqlite_request_object(ctx, db_id, |op_id| {
            let txs = self.sqlite.transactions.borrow();
            let Some(active) = txs.get(&tx_id) else {
                return Err(host_services::ServiceError {
                    code: "InvalidState",
                    message: "SQLite transaction is no longer active".into(),
                });
            };
            active.tx.query(op_id, sql, params, max_rows)
        })
    }

    fn sqlite_tx_execute<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        db_id: u64,
        tx_id: u64,
        sql_val: Value<'js>,
        params_val: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let sql = read_bounded_utf8(ctx, sql_val, MAX_SQL_BYTES, "SQL transaction statement")?;
        let params = parse_sql_params(ctx, params_val)?;
        self.sqlite_request_object(ctx, db_id, |op_id| {
            let txs = self.sqlite.transactions.borrow();
            let Some(active) = txs.get(&tx_id) else {
                return Err(host_services::ServiceError {
                    code: "InvalidState",
                    message: "SQLite transaction is no longer active".into(),
                });
            };
            active.tx.execute(op_id, sql, params)
        })
    }

    fn sqlite_tx_finish<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        db_id: u64,
        tx_id: u64,
        commit: bool,
    ) -> rquickjs::Result<Object<'js>> {
        self.sqlite_request_object(ctx, db_id, |op_id| {
            let txs = self.sqlite.transactions.borrow();
            let Some(active) = txs.get(&tx_id) else {
                return Err(host_services::ServiceError {
                    code: "InvalidState",
                    message: "SQLite transaction is no longer active".into(),
                });
            };
            if commit {
                active.tx.commit(op_id)
            } else {
                active.tx.rollback(op_id)
            }
        })
    }

    fn cancel_sqlite_op<'js>(&self, ctx: &Ctx<'js>, op_id: u64) -> rquickjs::Result<()> {
        let Some(pending) = self.sqlite.pending.borrow_mut().remove(&op_id) else {
            return Ok(());
        };
        pending
            .reject
            .restore(ctx)?
            .call::<_, ()>((service_error(ctx, "Cancelled", "SQLite request cancelled")?,))
    }

    fn close_sqlite_database<'js>(&self, ctx: &Ctx<'js>, db_id: u64) -> rquickjs::Result<()> {
        let tx_ids: Vec<u64> = self
            .sqlite
            .transactions
            .borrow()
            .iter()
            .filter_map(|(&tx_id, active)| (active.db_id == db_id).then_some(tx_id))
            .collect();
        for tx_id in tx_ids {
            if let Some(active) = self.sqlite.transactions.borrow_mut().remove(&tx_id) {
                active.tx.abort();
            }
        }
        if let Some(open) = self.sqlite.databases.borrow_mut().remove(&db_id) {
            open.db.close();
        }
        let op_ids: Vec<u64> = self
            .sqlite
            .pending
            .borrow()
            .iter()
            .filter_map(|(&op_id, op)| (op.db_id == db_id).then_some(op_id))
            .collect();
        for op_id in op_ids {
            if let Some(pending) = self.sqlite.pending.borrow_mut().remove(&op_id) {
                let err = service_error(ctx, "Cancelled", "SQLite database closed")?;
                let _ = pending.reject.restore(ctx)?.call::<_, ()>((err,));
            }
        }
        Ok(())
    }

    pub(crate) fn settle_sqlite<'js>(
        &self,
        ctx: &Ctx<'js>,
        _db_id: u64,
        event: SqliteEvent,
    ) -> rquickjs::Result<()> {
        if self.mode.get() != Mode::Mounted {
            return Ok(());
        }
        match event {
            SqliteEvent::Query { op_id, result } => {
                let Some(pending) = self.sqlite.pending.borrow_mut().remove(&op_id) else {
                    return Ok(());
                };
                match result {
                    Ok(rows) => {
                        let js_rows = sql_rows_to_js(ctx, rows)?;
                        pending.resolve.restore(ctx)?.call::<_, ()>((js_rows,))
                    }
                    Err(err) => {
                        let js_err = service_error(ctx, err.code, &err.message)?;
                        pending.reject.restore(ctx)?.call::<_, ()>((js_err,))
                    }
                }
            }
            SqliteEvent::Execute { op_id, result } => {
                let Some(pending) = self.sqlite.pending.borrow_mut().remove(&op_id) else {
                    return Ok(());
                };
                match result {
                    Ok(exec) => {
                        let js_exec = sql_execute_to_js(ctx, exec)?;
                        pending.resolve.restore(ctx)?.call::<_, ()>((js_exec,))
                    }
                    Err(err) => {
                        let js_err = service_error(ctx, err.code, &err.message)?;
                        pending.reject.restore(ctx)?.call::<_, ()>((js_err,))
                    }
                }
            }
            SqliteEvent::Batch { op_id, result } => {
                let Some(pending) = self.sqlite.pending.borrow_mut().remove(&op_id) else {
                    return Ok(());
                };
                match result {
                    Ok(results) => {
                        let array = Array::new(ctx.clone())?;
                        for (idx, exec) in results.into_iter().enumerate() {
                            array.set(idx, sql_execute_to_js(ctx, exec)?)?;
                        }
                        pending.resolve.restore(ctx)?.call::<_, ()>((array,))
                    }
                    Err(err) => {
                        let js_err = service_error(ctx, err.code, &err.message)?;
                        pending.reject.restore(ctx)?.call::<_, ()>((js_err,))
                    }
                }
            }
            SqliteEvent::TxBegun {
                op_id,
                tx_id,
                result,
            } => {
                if result.is_err() {
                    self.sqlite.transactions.borrow_mut().remove(&tx_id);
                }
                let Some(pending) = self.sqlite.pending.borrow_mut().remove(&op_id) else {
                    return Ok(());
                };
                match result {
                    Ok(()) => pending
                        .resolve
                        .restore(ctx)?
                        .call::<_, ()>((Value::new_undefined(ctx.clone()),)),
                    Err(err) => {
                        let js_err = service_error(ctx, err.code, &err.message)?;
                        pending.reject.restore(ctx)?.call::<_, ()>((js_err,))
                    }
                }
            }
            SqliteEvent::TxSettled {
                op_id,
                tx_id,
                result,
            } => {
                self.sqlite.transactions.borrow_mut().remove(&tx_id);
                let Some(pending) = self.sqlite.pending.borrow_mut().remove(&op_id) else {
                    return Ok(());
                };
                match result {
                    Ok(()) => pending
                        .resolve
                        .restore(ctx)?
                        .call::<_, ()>((Value::new_undefined(ctx.clone()),)),
                    Err(err) => {
                        let js_err = service_error(ctx, err.code, &err.message)?;
                        pending.reject.restore(ctx)?.call::<_, ()>((js_err,))
                    }
                }
            }
            SqliteEvent::TxTimedOut { tx_id } => {
                if let Some(active) = self.sqlite.transactions.borrow_mut().remove(&tx_id)
                    && let Some(reject) = active.timeout_reject
                {
                    let js_err = service_error(
                        ctx,
                        "E_TX_TIMEOUT",
                        "SQLite transaction timed out and was rolled back",
                    )?;
                    return reject.restore(ctx)?.call::<_, ()>((js_err,));
                }
                Ok(())
            }
        }
    }
}

fn throw_service_error(ctx: &Ctx<'_>, code: &str, message: &str) -> rquickjs::Error {
    match service_error(ctx, code, message) {
        Ok(err) => ctx.throw(err.into_value()),
        Err(err) => err,
    }
}

fn read_bounded_utf8(
    ctx: &Ctx<'_>,
    value: Value<'_>,
    limit: usize,
    label: &str,
) -> rquickjs::Result<String> {
    let string = value
        .into_string()
        .ok_or_else(|| Exception::throw_type(ctx, &format!("{label} must be a string")))?
        .to_cstring()?;
    if string.len() > limit {
        return Err(Exception::throw_range(
            ctx,
            &format!("{label} exceeds {limit} UTF-8 bytes"),
        ));
    }
    // SAFETY: QuickJS owns these `len` initialized bytes for `string`'s lifetime.
    let raw = unsafe { std::slice::from_raw_parts(string.as_ptr().cast::<u8>(), string.len()) };
    std::str::from_utf8(raw)
        .map(str::to_owned)
        .map_err(|_| Exception::throw_type(ctx, &format!("{label} contains invalid UTF-8")))
}

fn parse_sql_value(ctx: &Ctx<'_>, value: Value<'_>) -> rquickjs::Result<SqlValue> {
    if value.is_null() || value.is_undefined() {
        return Ok(SqlValue::Null);
    }
    if let Some(num) = value.as_number() {
        if !num.is_finite() {
            return Err(Exception::throw_range(
                ctx,
                "SQL numeric parameter must be finite",
            ));
        }
        if num.fract() == 0.0 && (-MAX_SAFE_INT_F64..=MAX_SAFE_INT_F64).contains(&num) {
            return Ok(SqlValue::Integer(num as i64));
        }
        return Ok(SqlValue::Real(num));
    }
    if value.is_string() {
        let text = read_bounded_utf8(ctx, value, MAX_BLOB_PARAM_BYTES, "SQL string parameter")?;
        return Ok(SqlValue::Text(text));
    }
    if let Ok(view) = TypedArray::<u8>::from_value(value) {
        let raw = view.as_raw().ok_or_else(|| {
            Exception::throw_type(ctx, "SQL Uint8Array parameter is detached or out of bounds")
        })?;
        if raw.len() > MAX_BLOB_PARAM_BYTES {
            return Err(Exception::throw_range(
                ctx,
                "SQL blob parameter exceeds 8 MiB limit",
            ));
        }
        // SAFETY: `view` keeps the backing ArrayBuffer alive while copied into owned Vec<u8>.
        return Ok(SqlValue::Blob(unsafe { raw.as_ref() }.to_vec()));
    }
    Err(Exception::throw_type(
        ctx,
        "SQL parameters must be null, finite number, string, or Uint8Array",
    ))
}

fn parse_sql_params(ctx: &Ctx<'_>, value: Option<Value<'_>>) -> rquickjs::Result<SqlParams> {
    let Some(value) = value else {
        return Ok(SqlParams::None);
    };
    if value.is_undefined() || value.is_null() {
        return Ok(SqlParams::None);
    }
    if value.is_array() {
        let array = value.into_array().expect("checked array");
        let len = array.len();
        if len > 1024 {
            return Err(Exception::throw_range(
                ctx,
                "SQL positional parameters exceed 1024 entries",
            ));
        }
        let mut params = Vec::with_capacity(len);
        for idx in 0..len {
            let item: Value = array.get(idx)?;
            params.push(parse_sql_value(ctx, item)?);
        }
        return Ok(SqlParams::Positional(params));
    }
    if let Some(obj) = value.into_object() {
        let mut entries = Vec::new();
        for prop in obj.props::<String, Value>() {
            let (key, val) = prop?;
            if entries.len() >= 1024 {
                return Err(Exception::throw_range(
                    ctx,
                    "SQL named parameters exceed 1024 entries",
                ));
            }
            entries.push((key, parse_sql_value(ctx, val)?));
        }
        return Ok(SqlParams::Named(entries));
    }
    Err(Exception::throw_type(
        ctx,
        "SQL params must be an array of positional values or a named parameter object",
    ))
}

fn parse_query_max_rows(ctx: &Ctx<'_>, options: Option<Value<'_>>) -> rquickjs::Result<usize> {
    let Some(value) = options else {
        return Ok(MAX_QUERY_ROWS);
    };
    if value.is_undefined() || value.is_null() {
        return Ok(MAX_QUERY_ROWS);
    }
    let obj = value
        .into_object()
        .ok_or_else(|| Exception::throw_type(ctx, "query options must be an object"))?;
    let max_val: Value = obj.get("maxRows")?;
    if max_val.is_undefined() {
        return Ok(MAX_QUERY_ROWS);
    }
    let Some(num) = max_val.as_number() else {
        return Err(Exception::throw_type(
            ctx,
            "query options.maxRows must be a positive integer",
        ));
    };
    if !num.is_finite() || num.fract() != 0.0 || num < 1.0 || num > MAX_QUERY_ROWS as f64 {
        return Err(Exception::throw_range(
            ctx,
            &format!("query options.maxRows must be between 1 and {MAX_QUERY_ROWS}"),
        ));
    }
    Ok(num as usize)
}

fn parse_tx_timeout_ms(ctx: &Ctx<'_>, options: Option<Value<'_>>) -> rquickjs::Result<u64> {
    let Some(value) = options else {
        return Ok(DEFAULT_TX_TIMEOUT_MS);
    };
    if value.is_undefined() || value.is_null() {
        return Ok(DEFAULT_TX_TIMEOUT_MS);
    }
    let obj = value
        .into_object()
        .ok_or_else(|| Exception::throw_type(ctx, "transaction options must be an object"))?;
    let timeout_val: Value = obj.get("timeoutMs")?;
    if timeout_val.is_undefined() {
        return Ok(DEFAULT_TX_TIMEOUT_MS);
    }
    let Some(num) = timeout_val.as_number() else {
        return Err(Exception::throw_type(
            ctx,
            "transaction options.timeoutMs must be a positive integer",
        ));
    };
    if !num.is_finite() || num.fract() != 0.0 || num < 1.0 || num > MAX_TX_TIMEOUT_MS as f64 {
        return Err(Exception::throw_range(
            ctx,
            &format!("transaction options.timeoutMs must be between 1 and {MAX_TX_TIMEOUT_MS}"),
        ));
    }
    Ok(num as u64)
}

fn sql_execute_to_js<'js>(ctx: &Ctx<'js>, exec: SqlExecuteResult) -> rquickjs::Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.prop("rowsAffected", data_property(exec.rows_affected as f64))?;
    match exec.last_insert_rowid {
        Some(rowid) => obj.prop("lastInsertRowid", data_property(rowid as f64))?,
        None => obj.prop("lastInsertRowid", data_property(Value::new_null(ctx.clone())))?,
    }
    Ok(obj)
}

fn sql_rows_to_js<'js>(ctx: &Ctx<'js>, rows: Vec<SqlRow>) -> rquickjs::Result<Array<'js>> {
    let array = Array::new(ctx.clone())?;
    for (row_idx, row) in rows.into_iter().enumerate() {
        let obj = Object::new(ctx.clone())?;
        for (col, val) in row {
            let js_val = match val {
                SqlValue::Null => Value::new_null(ctx.clone()),
                SqlValue::Integer(v) => (v as f64).into_js(ctx)?,
                SqlValue::Real(v) => v.into_js(ctx)?,
                SqlValue::Text(s) => s.into_js(ctx)?,
                SqlValue::Blob(bytes) => TypedArray::<u8>::new(ctx.clone(), bytes)?.into_value(),
            };
            obj.prop(col.as_str(), data_property(js_val))?;
        }
        array.set(row_idx, obj)?;
    }
    Ok(array)
}
