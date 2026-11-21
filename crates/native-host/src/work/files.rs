//! Session-owned local file handles, XDG Portal dialogs, persistent bookmarks, and watchers.
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};

use host_services::{
    ServiceError,
    files::{
        DEFAULT_WATCH_DEBOUNCE_MS, DialogFilter, DialogOptions, DirEntry, FileMode, FileStat,
        HandleKind, MAX_READ_BYTES, MAX_WATCH_DEBOUNCE_MS, MAX_WRITE_BYTES, MIN_WATCH_DEBOUNCE_MS,
        WatchEvent, atomic_write_bytes,
        grants::{
            BookmarkRecord, list_grants, persist_grant, resolve_grant_store_path, restore_grant,
            revoke_grant,
        },
        mkdir_scoped,
        portal::{PortalDialogKind, run_file_chooser},
        read_dir_entries, read_file_bytes, read_file_text, remove_scoped, rename_scoped,
        resolve_documents_dir, resolve_scoped_path, stat_path,
        vfs::{PreviewFs, ScriptedDialogKind},
        watcher::DirectoryWatcher,
    },
};
use rquickjs::{
    Array, Class, Ctx, Exception, Function, IntoJs, JsLifetime, Object, Persistent, TypedArray,
    Value,
    class::Trace,
    function::{Func, Opt},
};

use super::{Mode, Work, data_property, owner, service_error};
use crate::{
    assets::AssetSource,
    package::{FileDirectoryGrant, Package},
};

const MAX_FILE_HANDLES: usize = 256;
const MAX_FS_WATCHERS: usize = 32;
const MAX_PENDING_FILE_OPS: usize = 256;

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct FileHandleBrand {
    #[qjs(skip_trace)]
    id: u64,
    #[qjs(skip_trace)]
    generation: u64,
    #[qjs(skip_trace)]
    kind: HandleKind,
}

#[derive(Clone, Debug)]
pub(crate) struct OpenHandle {
    kind: HandleKind,
    path: String,
    mode: FileMode,
}

struct PendingFileOp {
    resolve: Persistent<Function<'static>>,
    reject: Persistent<Function<'static>>,
}

enum ActiveWatcher {
    Live(DirectoryWatcher),
    Preview(u64),
}

struct WatcherEntry {
    watcher: ActiveWatcher,
    callback: Persistent<Function<'static>>,
}

pub(crate) enum FileCompletionValue {
    Void,
    Bool(bool),
    Text(String),
    Bytes(Vec<u8>),
    Stat(FileStat),
    DirEntries(Vec<DirEntry>),
    Handle(Option<OpenHandle>),
    BookmarkToken(String),
    Bookmarks(Vec<BookmarkRecord>),
}

pub(crate) enum FileEvent {
    Completion {
        op_id: u64,
        result: Result<FileCompletionValue, ServiceError>,
    },
    Watch {
        watcher_id: u64,
        events: Vec<WatchEvent>,
    },
}

enum FilesBackend {
    Live {
        app_id: String,
        allowlist: Vec<(String, PathBuf, FileMode)>,
        tx: SyncSender<FileEvent>,
        rx: Receiver<FileEvent>,
    },
    Preview {
        vfs: RefCell<PreviewFs>,
        allowlist: Vec<(String, String, FileMode)>,
        queue: RefCell<VecDeque<FileEvent>>,
    },
}

#[derive(Clone, Copy, Default)]
pub(crate) struct FileCounts {
    pub(crate) handles: usize,
    pub(crate) watchers: usize,
}

pub(super) struct FilesState {
    backend: FilesBackend,
    handles: RefCell<BTreeMap<u64, OpenHandle>>,
    watchers: RefCell<BTreeMap<u64, WatcherEntry>>,
    pending: RefCell<HashMap<u64, PendingFileOp>>,
    ready: RefCell<Option<FileEvent>>,
    next_id: Cell<u64>,
}

impl FilesState {
    pub(super) fn new(
        package: Option<Rc<Package>>,
        source: &AssetSource,
        wake: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let grants: Vec<FileDirectoryGrant> = package
            .as_ref()
            .map(|p| p.manifest.capabilities.files.directories.clone())
            .unwrap_or_default();
        let backend = match source {
            AssetSource::Fixture(fixture) => {
                let mut vfs = fixture.build_preview_fs();
                vfs.set_wake(wake);
                let allowlist = grants
                    .into_iter()
                    .map(|g| {
                        let resolved = fixture.resolve_preview_file_root(&g.path);
                        (g.path, resolved, g.mode)
                    })
                    .collect();
                FilesBackend::Preview {
                    vfs: RefCell::new(vfs),
                    allowlist,
                    queue: RefCell::new(VecDeque::new()),
                }
            }
            AssetSource::Live(_) => {
                let app_id = package
                    .as_ref()
                    .map(|p| p.manifest.id.clone())
                    .unwrap_or_else(|| "zgui-app".to_owned());
                let allowlist = grants
                    .into_iter()
                    .filter_map(|g| {
                        resolve_live_dir_template(&g.path).map(|p| (g.path, p, g.mode))
                    })
                    .collect();
                let (tx, rx) = mpsc::sync_channel(512);
                FilesBackend::Live {
                    app_id,
                    allowlist,
                    tx,
                    rx,
                }
            }
        };
        Self {
            backend,
            handles: RefCell::new(BTreeMap::new()),
            watchers: RefCell::new(BTreeMap::new()),
            pending: RefCell::new(HashMap::new()),
            ready: RefCell::new(None),
            next_id: Cell::new(1),
        }
    }

    fn alloc_id(&self, ctx: &Ctx<'_>) -> rquickjs::Result<u64> {
        let id = self.next_id.get();
        let next = id
            .checked_add(1)
            .ok_or_else(|| Exception::throw_range(ctx, "file service identifiers exhausted"))?;
        self.next_id.set(next);
        Ok(id)
    }

    pub(super) fn counts(&self) -> FileCounts {
        FileCounts {
            handles: self.handles.borrow().len(),
            watchers: self.watchers.borrow().len(),
        }
    }

    pub(super) fn pending_count(&self) -> usize {
        self.pending.borrow().len()
    }

    fn take_event(&self) -> Option<FileEvent> {
        if let Some(ev) = self.ready.borrow_mut().take() {
            return Some(ev);
        }
        match &self.backend {
            FilesBackend::Live { rx, .. } => {
                if let Ok(ev) = rx.try_recv() {
                    return Some(ev);
                }
            }
            FilesBackend::Preview { queue, .. } => {
                if let Some(ev) = queue.borrow_mut().pop_front() {
                    return Some(ev);
                }
            }
        }
        // Poll active watchers.
        let watcher_ids: Vec<u64> = self.watchers.borrow().keys().copied().collect();
        for watcher_id in watcher_ids {
            let events = {
                let watchers = self.watchers.borrow();
                let Some(entry) = watchers.get(&watcher_id) else {
                    continue;
                };
                match &entry.watcher {
                    ActiveWatcher::Live(w) => w.try_recv(),
                    ActiveWatcher::Preview(preview_id) => {
                        if let FilesBackend::Preview { vfs, .. } = &self.backend {
                            vfs.borrow_mut().take_watcher_events(*preview_id)
                        } else {
                            None
                        }
                    }
                }
            };
            if let Some(events) = events {
                return Some(FileEvent::Watch { watcher_id, events });
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
        let watchers = std::mem::take(&mut *self.watchers.borrow_mut());
        for (_, entry) in watchers {
            match entry.watcher {
                ActiveWatcher::Live(w) => w.stop(),
                ActiveWatcher::Preview(id) => {
                    if let FilesBackend::Preview { vfs, .. } = &self.backend {
                        vfs.borrow_mut().remove_watcher(id);
                    }
                }
            }
        }
        self.handles.borrow_mut().clear();
        self.pending.borrow_mut().clear();
        self.ready.borrow_mut().take();
        if let FilesBackend::Preview { queue, .. } = &self.backend {
            queue.borrow_mut().clear();
        }
    }
}

impl Work {
    pub(super) fn install_files<'js>(
        self: &Rc<Self>,
        services: &Object<'js>,
    ) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        services.prop(
            "openFileDialog",
            data_property(Func::from(move |ctx: Ctx<'js>, options: Opt<Value<'js>>| {
                owner(&weak, &ctx)?.start_dialog(&ctx, PortalDialogKind::OpenFile, options.0)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "saveFileDialog",
            data_property(Func::from(move |ctx: Ctx<'js>, options: Opt<Value<'js>>| {
                owner(&weak, &ctx)?.start_dialog(&ctx, PortalDialogKind::SaveFile, options.0)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "openDirectoryDialog",
            data_property(Func::from(move |ctx: Ctx<'js>, options: Opt<Value<'js>>| {
                owner(&weak, &ctx)?.start_dialog(&ctx, PortalDialogKind::OpenDirectory, options.0)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "openAuthorizedDirectory",
            data_property(Func::from(move |ctx: Ctx<'js>, path: Value<'js>| {
                owner(&weak, &ctx)?.open_authorized_directory(&ctx, path)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "persistHandle",
            data_property(Func::from(move |ctx: Ctx<'js>, handle: Value<'js>| {
                owner(&weak, &ctx)?.persist_file_handle(&ctx, handle)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "restoreBookmark",
            data_property(Func::from(move |ctx: Ctx<'js>, token: Value<'js>| {
                owner(&weak, &ctx)?.restore_file_bookmark(&ctx, token)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "listBookmarks",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                owner(&weak, &ctx)?.list_file_bookmarks(&ctx)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "revokeBookmark",
            data_property(Func::from(move |ctx: Ctx<'js>, token: Value<'js>| {
                owner(&weak, &ctx)?.revoke_file_bookmark(&ctx, token)
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "watchDirectory",
            data_property(Func::from(
                move |ctx: Ctx<'js>,
                      handle: Value<'js>,
                      options: Value<'js>,
                      callback: Function<'js>| {
                    owner(&weak, &ctx)?.start_watch_directory(&ctx, handle, options, callback)
                },
            )),
        )
    }

    pub(crate) fn file_counts(&self) -> FileCounts {
        self.files.counts()
    }

    pub(crate) fn has_pending_file_ops(&self) -> bool {
        self.files.pending_count() > 0
    }

    pub(crate) fn has_file_events(&self) -> bool {
        self.files.has_events()
    }

    pub(crate) fn next_file_event(&self) -> Option<FileEvent> {
        if self.mode.get() != Mode::Mounted {
            return None;
        }
        self.files.take_event()
    }

    fn require_file_gesture(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        self.require_creation(ctx)?;
        if self.in_user_gesture() {
            return Ok(());
        }
        Err(throw_file_error(
            ctx,
            "E_CAPABILITY",
            "file picker dialogs require an active user gesture",
        ))
    }

    fn create_file_request<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
    ) -> rquickjs::Result<(u64, Object<'js>)> {
        self.require_creation(ctx)?;
        if self.files.pending.borrow().len() >= MAX_PENDING_FILE_OPS {
            return Err(Exception::throw_range(
                ctx,
                "concurrent file service operation limit reached",
            ));
        }
        let op_id = self.files.alloc_id(ctx)?;
        let (promise, resolve, reject) = ctx.promise()?;
        let object = Object::new(ctx.clone())?;
        object.prop("promise", data_property(promise))?;
        let weak = Rc::downgrade(self);
        object.prop(
            "cancel",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                owner(&weak, &ctx)?.cancel_file_op(&ctx, op_id)
            })),
        )?;
        self.files.pending.borrow_mut().insert(
            op_id,
            PendingFileOp {
                resolve: Persistent::save(ctx, resolve),
                reject: Persistent::save(ctx, reject),
            },
        );
        Ok((op_id, object))
    }

    fn push_preview_completion(&self, op_id: u64, result: Result<FileCompletionValue, ServiceError>) {
        if let FilesBackend::Preview { queue, .. } = &self.files.backend {
            queue
                .borrow_mut()
                .push_back(FileEvent::Completion { op_id, result });
            (self.wake)();
        }
    }

    fn spawn_live_op(
        &self,
        op_id: u64,
        work: impl FnOnce() -> Result<FileCompletionValue, ServiceError> + Send + 'static,
    ) {
        if let FilesBackend::Live { tx, .. } = &self.files.backend {
            let tx_worker = tx.clone();
            let wake_worker = self.wake.clone();
            let started = thread::Builder::new()
                .name("zgui-file-io".to_owned())
                .spawn(move || {
                    let result = work();
                    if tx_worker.send(FileEvent::Completion { op_id, result }).is_ok() {
                        wake_worker();
                    }
                });
            if let Err(err) = started {
                let _ = tx.try_send(FileEvent::Completion {
                    op_id,
                    result: Err(ServiceError {
                        code: "Io",
                        message: err.to_string(),
                    }),
                });
                (self.wake)();
            }
        }
    }

    fn start_dialog<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        kind: PortalDialogKind,
        options_val: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_file_gesture(ctx)?;
        let (options, read_only) = parse_dialog_options(ctx, options_val)?;
        let mode = if kind == PortalDialogKind::SaveFile || !read_only {
            FileMode::ReadWrite
        } else {
            FileMode::Read
        };
        let handle_kind = match kind {
            PortalDialogKind::OpenFile | PortalDialogKind::SaveFile => HandleKind::File,
            PortalDialogKind::OpenDirectory => HandleKind::Dir,
        };
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let scripted_kind = match kind {
                    PortalDialogKind::OpenFile => ScriptedDialogKind::OpenFile,
                    PortalDialogKind::SaveFile => ScriptedDialogKind::SaveFile,
                    PortalDialogKind::OpenDirectory => ScriptedDialogKind::OpenDirectory,
                };
                let res = vfs.borrow_mut().take_dialog(scripted_kind).map(|maybe| {
                    FileCompletionValue::Handle(maybe.map(|path| OpenHandle {
                        kind: handle_kind,
                        path,
                        mode,
                    }))
                });
                self.push_preview_completion(op_id, res);
            }
            FilesBackend::Live { .. } => {
                self.spawn_live_op(op_id, move || {
                    let picked = run_file_chooser(kind, &options)?;
                    let handle = match picked {
                        None => None,
                        Some(path) => {
                            let canonical = if kind == PortalDialogKind::SaveFile {
                                let parent = path.parent().ok_or_else(|| ServiceError {
                                    code: "InvalidPath",
                                    message: "save target has no parent directory".into(),
                                })?;
                                let leaf = path.file_name().ok_or_else(|| ServiceError {
                                    code: "InvalidPath",
                                    message: "save target has no filename".into(),
                                })?;
                                std::fs::canonicalize(parent)?.join(leaf)
                            } else {
                                std::fs::canonicalize(&path)?
                            };
                            Some(OpenHandle {
                                kind: handle_kind,
                                path: canonical.to_string_lossy().into_owned(),
                                mode,
                            })
                        }
                    };
                    Ok(FileCompletionValue::Handle(handle))
                });
            }
        }
        Ok(req)
    }

    fn open_authorized_directory<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        path_val: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        self.require_creation(ctx)?;
        let requested = read_bounded_utf8(ctx, path_val, 4096, "directory path")?;
        let (resolved_path, mode) = match &self.files.backend {
            FilesBackend::Preview { vfs, allowlist, .. } => {
                let found = allowlist
                    .iter()
                    .find(|(tpl, resolved, _)| tpl == &requested || resolved == &requested)
                    .map(|(_, resolved, mode)| (resolved.clone(), *mode));
                let Some((resolved, mode)) = found else {
                    return Err(throw_file_error(
                        ctx,
                        "E_CAPABILITY",
                        &format!(
                            "directory {requested:?} is not authorized by capabilities.files.directories"
                        ),
                    ));
                };
                vfs.borrow()
                    .stat(&resolved)
                    .and_then(|s| {
                        if s.is_dir {
                            Ok(())
                        } else {
                            Err(ServiceError {
                                code: "InvalidPath",
                                message: "authorized path is not a directory".into(),
                            })
                        }
                    })
                    .map_err(|e| throw_file_error(ctx, e.code, &e.message))?;
                (resolved, mode)
            }
            FilesBackend::Live { allowlist, .. } => {
                let found = allowlist
                    .iter()
                    .find(|(tpl, resolved, _)| {
                        tpl == &requested || resolved.to_string_lossy() == requested
                    })
                    .map(|(_, resolved, mode)| (resolved.clone(), *mode));
                let Some((resolved, mode)) = found else {
                    return Err(throw_file_error(
                        ctx,
                        "E_CAPABILITY",
                        &format!(
                            "directory {requested:?} is not authorized by capabilities.files.directories"
                        ),
                    ));
                };
                let canonical = std::fs::canonicalize(&resolved)
                    .map_err(|e| throw_file_error(ctx, "NotFound", &e.to_string()))?;
                if !canonical.is_dir() {
                    return Err(throw_file_error(
                        ctx,
                        "InvalidPath",
                        "authorized path is not a directory",
                    ));
                }
                (canonical.to_string_lossy().into_owned(), mode)
            }
        };
        self.register_handle(
            ctx,
            OpenHandle {
                kind: HandleKind::Dir,
                path: resolved_path,
                mode,
            },
        )
    }

    fn register_handle<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle: OpenHandle,
    ) -> rquickjs::Result<Object<'js>> {
        if self.files.handles.borrow().len() >= MAX_FILE_HANDLES {
            return Err(Exception::throw_range(
                ctx,
                "maximum open file handle limit reached",
            ));
        }
        let handle_id = self.files.alloc_id(ctx)?;
        let kind = handle.kind;
        let mode = handle.mode;
        let name = handle
            .path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("/")
            .to_owned();
        self.files.handles.borrow_mut().insert(handle_id, handle);

        let obj = Object::new(ctx.clone())?;
        let brand = Class::instance(
            ctx.clone(),
            FileHandleBrand {
                id: handle_id,
                generation: self.generation,
                kind,
            },
        )?;
        obj.set("__brand", brand)?;
        obj.prop(
            "kind",
            data_property(match kind {
                HandleKind::File => "file",
                HandleKind::Dir => "dir",
            }),
        )?;
        obj.prop("name", data_property(name.as_str()))?;
        obj.prop("mode", data_property(mode.as_str()))?;

        let weak = Rc::downgrade(self);
        obj.prop(
            "close",
            data_property(Func::from(move || {
                if let Some(work) = weak.upgrade() {
                    work.files.handles.borrow_mut().remove(&handle_id);
                }
            })),
        )?;

        match kind {
            HandleKind::File => {
                let weak = Rc::downgrade(self);
                obj.prop(
                    "readText",
                    data_property(Func::from(move |ctx: Ctx<'js>, opts: Opt<Value<'js>>| {
                        owner(&weak, &ctx)?.file_read_text(&ctx, handle_id, None, opts.0)
                    })),
                )?;
                let weak = Rc::downgrade(self);
                obj.prop(
                    "readBytes",
                    data_property(Func::from(move |ctx: Ctx<'js>, opts: Opt<Value<'js>>| {
                        owner(&weak, &ctx)?.file_read_bytes(&ctx, handle_id, None, opts.0)
                    })),
                )?;
                let weak = Rc::downgrade(self);
                obj.prop(
                    "writeText",
                    data_property(Func::from(move |ctx: Ctx<'js>, text: Value<'js>| {
                        owner(&weak, &ctx)?.file_write_text(&ctx, handle_id, None, text)
                    })),
                )?;
                let weak = Rc::downgrade(self);
                obj.prop(
                    "writeBytes",
                    data_property(Func::from(move |ctx: Ctx<'js>, bytes: Value<'js>| {
                        owner(&weak, &ctx)?.file_write_bytes(&ctx, handle_id, None, bytes)
                    })),
                )?;
                let weak = Rc::downgrade(self);
                obj.prop(
                    "stat",
                    data_property(Func::from(move |ctx: Ctx<'js>| {
                        owner(&weak, &ctx)?.file_stat(&ctx, handle_id, None)
                    })),
                )?;
            }
            HandleKind::Dir => {
                let weak = Rc::downgrade(self);
                obj.prop(
                    "readText",
                    data_property(Func::from(
                        move |ctx: Ctx<'js>, rel: Value<'js>, opts: Opt<Value<'js>>| {
                            owner(&weak, &ctx)?.file_read_text(&ctx, handle_id, Some(rel), opts.0)
                        },
                    )),
                )?;
                let weak = Rc::downgrade(self);
                obj.prop(
                    "readBytes",
                    data_property(Func::from(
                        move |ctx: Ctx<'js>, rel: Value<'js>, opts: Opt<Value<'js>>| {
                            owner(&weak, &ctx)?.file_read_bytes(&ctx, handle_id, Some(rel), opts.0)
                        },
                    )),
                )?;
                let weak = Rc::downgrade(self);
                obj.prop(
                    "writeText",
                    data_property(Func::from(
                        move |ctx: Ctx<'js>, rel: Value<'js>, text: Value<'js>| {
                            owner(&weak, &ctx)?.file_write_text(&ctx, handle_id, Some(rel), text)
                        },
                    )),
                )?;
                let weak = Rc::downgrade(self);
                obj.prop(
                    "writeBytes",
                    data_property(Func::from(
                        move |ctx: Ctx<'js>, rel: Value<'js>, bytes: Value<'js>| {
                            owner(&weak, &ctx)?.file_write_bytes(&ctx, handle_id, Some(rel), bytes)
                        },
                    )),
                )?;
                let weak = Rc::downgrade(self);
                obj.prop(
                    "stat",
                    data_property(Func::from(move |ctx: Ctx<'js>, rel: Opt<Value<'js>>| {
                        owner(&weak, &ctx)?.file_stat(&ctx, handle_id, rel.0)
                    })),
                )?;
                let weak = Rc::downgrade(self);
                obj.prop(
                    "readDir",
                    data_property(Func::from(move |ctx: Ctx<'js>, rel: Opt<Value<'js>>| {
                        owner(&weak, &ctx)?.dir_read_dir(&ctx, handle_id, rel.0)
                    })),
                )?;
                let weak = Rc::downgrade(self);
                obj.prop(
                    "mkdir",
                    data_property(Func::from(
                        move |ctx: Ctx<'js>, rel: Value<'js>, opts: Opt<Value<'js>>| {
                            owner(&weak, &ctx)?.dir_mkdir(&ctx, handle_id, rel, opts.0)
                        },
                    )),
                )?;
                let weak = Rc::downgrade(self);
                obj.prop(
                    "rename",
                    data_property(Func::from(
                        move |ctx: Ctx<'js>, from_rel: Value<'js>, to_rel: Value<'js>| {
                            owner(&weak, &ctx)?.dir_rename(&ctx, handle_id, from_rel, to_rel)
                        },
                    )),
                )?;
                let weak = Rc::downgrade(self);
                obj.prop(
                    "remove",
                    data_property(Func::from(
                        move |ctx: Ctx<'js>, rel: Value<'js>, opts: Opt<Value<'js>>| {
                            owner(&weak, &ctx)?.dir_remove(&ctx, handle_id, rel, opts.0)
                        },
                    )),
                )?;
            }
        }
        Ok(obj)
    }

    fn lookup_handle(&self, ctx: &Ctx<'_>, handle_id: u64) -> rquickjs::Result<OpenHandle> {
        self.files
            .handles
            .borrow()
            .get(&handle_id)
            .cloned()
            .ok_or_else(|| throw_file_error(ctx, "InvalidState", "file handle is closed"))
    }

    fn extract_brand(
        &self,
        ctx: &Ctx<'_>,
        value: Value<'_>,
        expected_kind: Option<HandleKind>,
    ) -> rquickjs::Result<u64> {
        let obj = value
            .into_object()
            .ok_or_else(|| Exception::throw_type(ctx, "expected a live FileHandle or DirHandle"))?;
        let brand_val: Value = obj.get("__brand")?;
        let Ok(brand) = Class::<FileHandleBrand>::from_value(&brand_val) else {
            return Err(Exception::throw_type(
                ctx,
                "invalid or forged file handle object",
            ));
        };
        let b = brand.borrow();
        if b.generation != self.generation {
            return Err(throw_file_error(
                ctx,
                "InvalidState",
                "file handle belongs to another session",
            ));
        }
        if let Some(kind) = expected_kind
            && b.kind != kind
        {
            return Err(Exception::throw_type(
                ctx,
                "expected a DirHandle for this operation",
            ));
        }
        Ok(b.id)
    }

    fn file_read_text<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle_id: u64,
        rel_val: Option<Value<'js>>,
        opts_val: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let handle = self.lookup_handle(ctx, handle_id)?;
        let rel = parse_optional_rel(ctx, rel_val)?;
        let max_bytes = parse_max_bytes(ctx, opts_val)?;
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let res = (|| {
                    let vfs = vfs.borrow();
                    let target = match (handle.kind, rel.as_deref()) {
                        (HandleKind::File, _) => handle.path.clone(),
                        (HandleKind::Dir, Some(r)) => vfs.resolve_scoped(&handle.path, r, false)?,
                        (HandleKind::Dir, None) => {
                            return Err(ServiceError {
                                code: "InvalidPath",
                                message: "dir.readText requires a relative file path".into(),
                            });
                        }
                    };
                    vfs.read_text(&target, max_bytes).map(FileCompletionValue::Text)
                })();
                self.push_preview_completion(op_id, res);
            }
            FilesBackend::Live { .. } => {
                self.spawn_live_op(op_id, move || {
                    let target = match (handle.kind, rel.as_deref()) {
                        (HandleKind::File, _) => PathBuf::from(&handle.path),
                        (HandleKind::Dir, Some(r)) => {
                            resolve_scoped_path(Path::new(&handle.path), r, false)?
                        }
                        (HandleKind::Dir, None) => {
                            return Err(ServiceError {
                                code: "InvalidPath",
                                message: "dir.readText requires a relative file path".into(),
                            });
                        }
                    };
                    read_file_text(&target, max_bytes).map(FileCompletionValue::Text)
                });
            }
        }
        Ok(req)
    }

    fn file_read_bytes<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle_id: u64,
        rel_val: Option<Value<'js>>,
        opts_val: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let handle = self.lookup_handle(ctx, handle_id)?;
        let rel = parse_optional_rel(ctx, rel_val)?;
        let max_bytes = parse_max_bytes(ctx, opts_val)?;
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let res = (|| {
                    let vfs = vfs.borrow();
                    let target = match (handle.kind, rel.as_deref()) {
                        (HandleKind::File, _) => handle.path.clone(),
                        (HandleKind::Dir, Some(r)) => vfs.resolve_scoped(&handle.path, r, false)?,
                        (HandleKind::Dir, None) => {
                            return Err(ServiceError {
                                code: "InvalidPath",
                                message: "dir.readBytes requires a relative file path".into(),
                            });
                        }
                    };
                    vfs.read_bytes(&target, max_bytes).map(FileCompletionValue::Bytes)
                })();
                self.push_preview_completion(op_id, res);
            }
            FilesBackend::Live { .. } => {
                self.spawn_live_op(op_id, move || {
                    let target = match (handle.kind, rel.as_deref()) {
                        (HandleKind::File, _) => PathBuf::from(&handle.path),
                        (HandleKind::Dir, Some(r)) => {
                            resolve_scoped_path(Path::new(&handle.path), r, false)?
                        }
                        (HandleKind::Dir, None) => {
                            return Err(ServiceError {
                                code: "InvalidPath",
                                message: "dir.readBytes requires a relative file path".into(),
                            });
                        }
                    };
                    read_file_bytes(&target, max_bytes).map(FileCompletionValue::Bytes)
                });
            }
        }
        Ok(req)
    }

    fn file_write_text<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle_id: u64,
        rel_val: Option<Value<'js>>,
        text_val: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        let handle = self.lookup_handle(ctx, handle_id)?;
        if !handle.mode.can_write() {
            return Err(throw_file_error(
                ctx,
                "PermissionDenied",
                "file handle is read-only",
            ));
        }
        let rel = parse_optional_rel(ctx, rel_val)?;
        let text = read_bounded_utf8(ctx, text_val, MAX_WRITE_BYTES, "file text")?;
        self.write_bytes_inner(ctx, handle, rel, text.into_bytes())
    }

    fn file_write_bytes<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle_id: u64,
        rel_val: Option<Value<'js>>,
        bytes_val: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        let handle = self.lookup_handle(ctx, handle_id)?;
        if !handle.mode.can_write() {
            return Err(throw_file_error(
                ctx,
                "PermissionDenied",
                "file handle is read-only",
            ));
        }
        let rel = parse_optional_rel(ctx, rel_val)?;
        let view = TypedArray::<u8>::from_value(bytes_val)
            .map_err(|_| Exception::throw_type(ctx, "writeBytes requires a Uint8Array"))?;
        let raw = view.as_raw().ok_or_else(|| {
            Exception::throw_type(ctx, "writeBytes Uint8Array is detached or out of bounds")
        })?;
        if raw.len() > MAX_WRITE_BYTES {
            return Err(Exception::throw_range(
                ctx,
                &format!("write payload exceeds maximum of {MAX_WRITE_BYTES} bytes"),
            ));
        }
        // SAFETY: `view` keeps the backing ArrayBuffer alive while copied to owned Vec<u8>.
        let bytes = unsafe { raw.as_ref() }.to_vec();
        self.write_bytes_inner(ctx, handle, rel, bytes)
    }

    fn write_bytes_inner<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle: OpenHandle,
        rel: Option<String>,
        bytes: Vec<u8>,
    ) -> rquickjs::Result<Object<'js>> {
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let res = (|| {
                    let mut vfs = vfs.borrow_mut();
                    let target = match (handle.kind, rel.as_deref()) {
                        (HandleKind::File, _) => handle.path.clone(),
                        (HandleKind::Dir, Some(r)) => vfs.resolve_scoped(&handle.path, r, true)?,
                        (HandleKind::Dir, None) => {
                            return Err(ServiceError {
                                code: "InvalidPath",
                                message: "dir.write requires a relative file path".into(),
                            });
                        }
                    };
                    vfs.write_bytes(&target, bytes, 0)
                        .map(|()| FileCompletionValue::Void)
                })();
                self.push_preview_completion(op_id, res);
            }
            FilesBackend::Live { .. } => {
                self.spawn_live_op(op_id, move || {
                    let target = match (handle.kind, rel.as_deref()) {
                        (HandleKind::File, _) => PathBuf::from(&handle.path),
                        (HandleKind::Dir, Some(r)) => {
                            resolve_scoped_path(Path::new(&handle.path), r, true)?
                        }
                        (HandleKind::Dir, None) => {
                            return Err(ServiceError {
                                code: "InvalidPath",
                                message: "dir.write requires a relative file path".into(),
                            });
                        }
                    };
                    atomic_write_bytes(&target, &bytes, None).map(|()| FileCompletionValue::Void)
                });
            }
        }
        Ok(req)
    }

    fn file_stat<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle_id: u64,
        rel_val: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let handle = self.lookup_handle(ctx, handle_id)?;
        let rel = parse_optional_rel(ctx, rel_val)?;
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let res = (|| {
                    let vfs = vfs.borrow();
                    let target = match (handle.kind, rel.as_deref()) {
                        (HandleKind::Dir, Some(r)) if !r.is_empty() => {
                            vfs.resolve_scoped(&handle.path, r, false)?
                        }
                        _ => handle.path.clone(),
                    };
                    vfs.stat(&target).map(FileCompletionValue::Stat)
                })();
                self.push_preview_completion(op_id, res);
            }
            FilesBackend::Live { .. } => {
                self.spawn_live_op(op_id, move || {
                    let target = match (handle.kind, rel.as_deref()) {
                        (HandleKind::Dir, Some(r)) if !r.is_empty() => {
                            resolve_scoped_path(Path::new(&handle.path), r, false)?
                        }
                        _ => PathBuf::from(&handle.path),
                    };
                    stat_path(&target).map(FileCompletionValue::Stat)
                });
            }
        }
        Ok(req)
    }

    fn dir_read_dir<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle_id: u64,
        rel_val: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let handle = self.lookup_handle(ctx, handle_id)?;
        let rel = parse_optional_rel(ctx, rel_val)?;
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let res = (|| {
                    let vfs = vfs.borrow();
                    let target = match rel.as_deref() {
                        Some(r) if !r.is_empty() => vfs.resolve_scoped(&handle.path, r, false)?,
                        _ => handle.path.clone(),
                    };
                    vfs.read_dir(&handle.path, &target)
                        .map(FileCompletionValue::DirEntries)
                })();
                self.push_preview_completion(op_id, res);
            }
            FilesBackend::Live { .. } => {
                self.spawn_live_op(op_id, move || {
                    let root = Path::new(&handle.path);
                    let target = match rel.as_deref() {
                        Some(r) if !r.is_empty() => resolve_scoped_path(root, r, false)?,
                        _ => root.to_path_buf(),
                    };
                    read_dir_entries(root, &target).map(FileCompletionValue::DirEntries)
                });
            }
        }
        Ok(req)
    }

    fn dir_mkdir<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle_id: u64,
        rel_val: Value<'js>,
        opts_val: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let handle = self.lookup_handle(ctx, handle_id)?;
        if !handle.mode.can_write() {
            return Err(throw_file_error(
                ctx,
                "PermissionDenied",
                "directory handle is read-only",
            ));
        }
        let rel = read_bounded_utf8(ctx, rel_val, 4096, "relative directory path")?;
        let recursive = parse_recursive_option(ctx, opts_val)?;
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let res = vfs
                    .borrow_mut()
                    .mkdir(&handle.path, &rel, recursive)
                    .map(|()| FileCompletionValue::Void);
                self.push_preview_completion(op_id, res);
            }
            FilesBackend::Live { .. } => {
                self.spawn_live_op(op_id, move || {
                    mkdir_scoped(Path::new(&handle.path), &rel, recursive)
                        .map(|()| FileCompletionValue::Void)
                });
            }
        }
        Ok(req)
    }

    fn dir_rename<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle_id: u64,
        from_val: Value<'js>,
        to_val: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        let handle = self.lookup_handle(ctx, handle_id)?;
        if !handle.mode.can_write() {
            return Err(throw_file_error(
                ctx,
                "PermissionDenied",
                "directory handle is read-only",
            ));
        }
        let from_rel = read_bounded_utf8(ctx, from_val, 4096, "rename source path")?;
        let to_rel = read_bounded_utf8(ctx, to_val, 4096, "rename target path")?;
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let res = vfs
                    .borrow_mut()
                    .rename(&handle.path, &from_rel, &to_rel)
                    .map(|()| FileCompletionValue::Void);
                self.push_preview_completion(op_id, res);
            }
            FilesBackend::Live { .. } => {
                self.spawn_live_op(op_id, move || {
                    rename_scoped(Path::new(&handle.path), &from_rel, &to_rel)
                        .map(|()| FileCompletionValue::Void)
                });
            }
        }
        Ok(req)
    }

    fn dir_remove<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle_id: u64,
        rel_val: Value<'js>,
        opts_val: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let handle = self.lookup_handle(ctx, handle_id)?;
        if !handle.mode.can_write() {
            return Err(throw_file_error(
                ctx,
                "PermissionDenied",
                "directory handle is read-only",
            ));
        }
        let rel = read_bounded_utf8(ctx, rel_val, 4096, "remove path")?;
        let recursive = parse_recursive_option(ctx, opts_val)?;
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let res = vfs
                    .borrow_mut()
                    .remove(&handle.path, &rel, recursive)
                    .map(|()| FileCompletionValue::Void);
                self.push_preview_completion(op_id, res);
            }
            FilesBackend::Live { .. } => {
                self.spawn_live_op(op_id, move || {
                    remove_scoped(Path::new(&handle.path), &rel, recursive)
                        .map(|()| FileCompletionValue::Void)
                });
            }
        }
        Ok(req)
    }

    fn persist_file_handle<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle_val: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        let handle_id = self.extract_brand(ctx, handle_val, None)?;
        let handle = self.lookup_handle(ctx, handle_id)?;
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let record =
                    vfs.borrow_mut()
                        .persist_bookmark(handle.kind, &handle.path, handle.mode, 0);
                self.push_preview_completion(
                    op_id,
                    Ok(FileCompletionValue::BookmarkToken(record.token)),
                );
            }
            FilesBackend::Live { app_id, .. } => {
                let app_id = app_id.clone();
                self.spawn_live_op(op_id, move || {
                    let store = resolve_grant_store_path(&app_id)?;
                    let rec = persist_grant(
                        &store,
                        &app_id,
                        handle.kind,
                        Path::new(&handle.path),
                        handle.mode,
                    )?;
                    Ok(FileCompletionValue::BookmarkToken(rec.token))
                });
            }
        }
        Ok(req)
    }

    fn restore_file_bookmark<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        token_val: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        let token = read_bounded_utf8(ctx, token_val, 256, "bookmark token")?;
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let res = vfs.borrow().restore_bookmark(&token).map(|rec| {
                    FileCompletionValue::Handle(Some(OpenHandle {
                        kind: rec.kind,
                        path: rec.path,
                        mode: rec.mode,
                    }))
                });
                self.push_preview_completion(op_id, res);
            }
            FilesBackend::Live { app_id, .. } => {
                let app_id = app_id.clone();
                self.spawn_live_op(op_id, move || {
                    let store = resolve_grant_store_path(&app_id)?;
                    let rec = restore_grant(&store, &token)?;
                    let canonical = std::fs::canonicalize(&rec.path)?;
                    Ok(FileCompletionValue::Handle(Some(OpenHandle {
                        kind: rec.kind,
                        path: canonical.to_string_lossy().into_owned(),
                        mode: rec.mode,
                    })))
                });
            }
        }
        Ok(req)
    }

    fn list_file_bookmarks<'js>(self: &Rc<Self>, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let list = vfs.borrow().list_bookmarks();
                self.push_preview_completion(op_id, Ok(FileCompletionValue::Bookmarks(list)));
            }
            FilesBackend::Live { app_id, .. } => {
                let app_id = app_id.clone();
                self.spawn_live_op(op_id, move || {
                    let store = resolve_grant_store_path(&app_id)?;
                    list_grants(&store).map(FileCompletionValue::Bookmarks)
                });
            }
        }
        Ok(req)
    }

    fn revoke_file_bookmark<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        token_val: Value<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        let token = read_bounded_utf8(ctx, token_val, 256, "bookmark token")?;
        let (op_id, req) = self.create_file_request(ctx)?;
        match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let removed = vfs.borrow_mut().revoke_bookmark(&token);
                self.push_preview_completion(op_id, Ok(FileCompletionValue::Bool(removed)));
            }
            FilesBackend::Live { app_id, .. } => {
                let app_id = app_id.clone();
                self.spawn_live_op(op_id, move || {
                    let store = resolve_grant_store_path(&app_id)?;
                    revoke_grant(&store, &token).map(FileCompletionValue::Bool)
                });
            }
        }
        Ok(req)
    }

    fn start_watch_directory<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        handle_val: Value<'js>,
        options_val: Value<'js>,
        callback: Function<'js>,
    ) -> rquickjs::Result<Function<'js>> {
        self.require_creation(ctx)?;
        if self.files.watchers.borrow().len() >= MAX_FS_WATCHERS {
            return Err(Exception::throw_range(
                ctx,
                "maximum active directory watcher limit reached",
            ));
        }
        let handle_id = self.extract_brand(ctx, handle_val, Some(HandleKind::Dir))?;
        let handle = self.lookup_handle(ctx, handle_id)?;
        let (recursive, debounce_ms) = parse_watch_options(ctx, options_val)?;
        let watcher_id = self.files.alloc_id(ctx)?;

        let watcher = match &self.files.backend {
            FilesBackend::Preview { vfs, .. } => {
                let pid = vfs.borrow_mut().add_watcher(&handle.path, recursive);
                ActiveWatcher::Preview(pid)
            }
            FilesBackend::Live { .. } => {
                let live = DirectoryWatcher::start(
                    PathBuf::from(&handle.path),
                    recursive,
                    debounce_ms,
                    self.wake.clone(),
                )
                .map_err(|err| throw_file_error(ctx, err.code, &err.message))?;
                ActiveWatcher::Live(live)
            }
        };

        self.files.watchers.borrow_mut().insert(
            watcher_id,
            WatcherEntry {
                watcher,
                callback: Persistent::save(ctx, callback),
            },
        );

        let weak = Rc::downgrade(self);
        Function::new(ctx.clone(), move || {
            if let Some(work) = weak.upgrade()
                && let Some(entry) = work.files.watchers.borrow_mut().remove(&watcher_id)
            {
                match entry.watcher {
                    ActiveWatcher::Live(w) => w.stop(),
                    ActiveWatcher::Preview(pid) => {
                        if let FilesBackend::Preview { vfs, .. } = &work.files.backend {
                            vfs.borrow_mut().remove_watcher(pid);
                        }
                    }
                }
            }
        })
    }

    fn cancel_file_op<'js>(&self, ctx: &Ctx<'js>, op_id: u64) -> rquickjs::Result<()> {
        let Some(pending) = self.files.pending.borrow_mut().remove(&op_id) else {
            return Ok(());
        };
        pending
            .reject
            .restore(ctx)?
            .call::<_, ()>((service_error(ctx, "Cancelled", "file operation cancelled")?,))
    }

    pub(crate) fn settle_file_event<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        event: FileEvent,
    ) -> rquickjs::Result<()> {
        if self.mode.get() != Mode::Mounted {
            return Ok(());
        }
        match event {
            FileEvent::Completion { op_id, result } => {
                let Some(pending) = self.files.pending.borrow_mut().remove(&op_id) else {
                    return Ok(());
                };
                match result {
                    Ok(value) => {
                        let js_val = match value {
                            FileCompletionValue::Void => Value::new_undefined(ctx.clone()),
                            FileCompletionValue::Bool(b) => b.into_js(ctx)?,
                            FileCompletionValue::Text(s) => s.into_js(ctx)?,
                            FileCompletionValue::Bytes(b) => {
                                TypedArray::<u8>::new(ctx.clone(), b)?.into_value()
                            }
                            FileCompletionValue::Stat(s) => stat_to_js(ctx, s)?.into_value(),
                            FileCompletionValue::DirEntries(entries) => {
                                dir_entries_to_js(ctx, entries)?.into_value()
                            }
                            FileCompletionValue::Handle(None) => Value::new_null(ctx.clone()),
                            FileCompletionValue::Handle(Some(h)) => {
                                self.register_handle(ctx, h)?.into_value()
                            }
                            FileCompletionValue::BookmarkToken(tok) => tok.into_js(ctx)?,
                            FileCompletionValue::Bookmarks(list) => {
                                bookmarks_to_js(ctx, list)?.into_value()
                            }
                        };
                        pending.resolve.restore(ctx)?.call::<_, ()>((js_val,))
                    }
                    Err(err) => {
                        let js_err = service_error(ctx, err.code, &err.message)?;
                        pending.reject.restore(ctx)?.call::<_, ()>((js_err,))
                    }
                }
            }
            FileEvent::Watch { watcher_id, events } => {
                let callback = self
                    .files
                    .watchers
                    .borrow()
                    .get(&watcher_id)
                    .map(|entry| entry.callback.clone());
                let Some(callback) = callback else {
                    return Ok(());
                };
                let js_events = watch_events_to_js(ctx, events)?;
                callback.restore(ctx)?.call::<_, ()>((js_events,))
            }
        }
    }
}

fn resolve_live_dir_template(template: &str) -> Option<PathBuf> {
    if let Some(rest) = template.strip_prefix("$XDG_DOCUMENTS_DIR") {
        let base = resolve_documents_dir().ok()?;
        let rel = rest.trim_start_matches('/');
        return Some(if rel.is_empty() { base } else { base.join(rel) });
    }
    if let Some(rest) = template.strip_prefix("$XDG_DATA_HOME") {
        let base = std::env::var("XDG_DATA_HOME")
            .ok()
            .filter(|v| !v.is_empty() && Path::new(v).is_absolute())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .filter(|v| !v.is_empty() && Path::new(v).is_absolute())
                    .map(|h| PathBuf::from(h).join(".local/share"))
            })?;
        let rel = rest.trim_start_matches('/');
        return Some(if rel.is_empty() { base } else { base.join(rel) });
    }
    if let Some(rest) = template.strip_prefix("$HOME") {
        let base = std::env::var("HOME")
            .ok()
            .filter(|v| !v.is_empty() && Path::new(v).is_absolute())
            .map(PathBuf::from)?;
        let rel = rest.trim_start_matches('/');
        return Some(if rel.is_empty() { base } else { base.join(rel) });
    }
    let p = PathBuf::from(template);
    p.is_absolute().then_some(p)
}

fn throw_file_error(ctx: &Ctx<'_>, code: &str, message: &str) -> rquickjs::Error {
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

fn parse_optional_rel(ctx: &Ctx<'_>, val: Option<Value<'_>>) -> rquickjs::Result<Option<String>> {
    let Some(v) = val else {
        return Ok(None);
    };
    if v.is_undefined() || v.is_null() {
        return Ok(None);
    }
    Ok(Some(read_bounded_utf8(ctx, v, 4096, "relative path")?))
}

fn parse_max_bytes(ctx: &Ctx<'_>, val: Option<Value<'_>>) -> rquickjs::Result<usize> {
    let Some(v) = val else {
        return Ok(MAX_READ_BYTES);
    };
    if v.is_undefined() || v.is_null() {
        return Ok(MAX_READ_BYTES);
    }
    let obj = v
        .into_object()
        .ok_or_else(|| Exception::throw_type(ctx, "read options must be an object"))?;
    let max_val: Value = obj.get("maxBytes")?;
    if max_val.is_undefined() {
        return Ok(MAX_READ_BYTES);
    }
    let Some(num) = max_val.as_number() else {
        return Err(Exception::throw_type(
            ctx,
            "options.maxBytes must be a positive integer",
        ));
    };
    if !num.is_finite() || num.fract() != 0.0 || num < 1.0 || num > MAX_READ_BYTES as f64 {
        return Err(Exception::throw_range(
            ctx,
            &format!("options.maxBytes must be between 1 and {MAX_READ_BYTES}"),
        ));
    }
    Ok(num as usize)
}

fn parse_recursive_option(ctx: &Ctx<'_>, val: Option<Value<'_>>) -> rquickjs::Result<bool> {
    let Some(v) = val else {
        return Ok(false);
    };
    if v.is_undefined() || v.is_null() {
        return Ok(false);
    }
    let obj = v
        .into_object()
        .ok_or_else(|| Exception::throw_type(ctx, "options must be an object"))?;
    let rec: Value = obj.get("recursive")?;
    if rec.is_undefined() {
        return Ok(false);
    }
    rec.as_bool()
        .ok_or_else(|| Exception::throw_type(ctx, "options.recursive must be a boolean"))
}

fn parse_dialog_options(
    ctx: &Ctx<'_>,
    val: Option<Value<'_>>,
) -> rquickjs::Result<(DialogOptions, bool)> {
    let Some(v) = val else {
        return Ok((DialogOptions::default(), false));
    };
    if v.is_undefined() || v.is_null() {
        return Ok((DialogOptions::default(), false));
    }
    let obj = v
        .into_object()
        .ok_or_else(|| Exception::throw_type(ctx, "dialog options must be an object"))?;
    let mut opts = DialogOptions::default();
    let title_val: Value = obj.get("title")?;
    if !title_val.is_undefined() {
        opts.title = Some(read_bounded_utf8(ctx, title_val, 256, "dialog title")?);
    }
    let suggested_val: Value = obj.get("suggestedName")?;
    if !suggested_val.is_undefined() {
        opts.suggested_name = Some(read_bounded_utf8(
            ctx,
            suggested_val,
            256,
            "dialog suggestedName",
        )?);
    }
    let ro_val: Value = obj.get("readOnly")?;
    let read_only = if ro_val.is_undefined() {
        false
    } else {
        ro_val
            .as_bool()
            .ok_or_else(|| Exception::throw_type(ctx, "dialog options.readOnly must be a boolean"))?
    };
    let filters_val: Value = obj.get("filters")?;
    if !filters_val.is_undefined() {
        let arr = filters_val
            .into_array()
            .ok_or_else(|| Exception::throw_type(ctx, "dialog options.filters must be an array"))?;
        for idx in 0..arr.len().min(32) {
            let item: Value = arr.get(idx)?;
            let fobj = item.into_object().ok_or_else(|| {
                Exception::throw_type(ctx, "each dialog filter must be an object")
            })?;
            let name = read_bounded_utf8(ctx, fobj.get("name")?, 128, "filter name")?;
            let exts_val: Value = fobj.get("extensions")?;
            let exts_arr = exts_val.into_array().ok_or_else(|| {
                Exception::throw_type(ctx, "filter.extensions must be a string array")
            })?;
            let mut extensions = Vec::with_capacity(exts_arr.len());
            for j in 0..exts_arr.len().min(64) {
                extensions.push(read_bounded_utf8(
                    ctx,
                    exts_arr.get(j)?,
                    32,
                    "filter extension",
                )?);
            }
            opts.filters.push(DialogFilter { name, extensions });
        }
    }
    Ok((opts, read_only))
}

fn parse_watch_options(ctx: &Ctx<'_>, val: Value<'_>) -> rquickjs::Result<(bool, u64)> {
    if val.is_undefined() || val.is_null() {
        return Ok((false, DEFAULT_WATCH_DEBOUNCE_MS));
    }
    let obj = val
        .into_object()
        .ok_or_else(|| Exception::throw_type(ctx, "watchDirectory options must be an object"))?;
    let rec_val: Value = obj.get("recursive")?;
    let recursive = if rec_val.is_undefined() {
        false
    } else {
        rec_val
            .as_bool()
            .ok_or_else(|| Exception::throw_type(ctx, "options.recursive must be a boolean"))?
    };
    let deb_val: Value = obj.get("debounceMs")?;
    let debounce_ms = if deb_val.is_undefined() {
        DEFAULT_WATCH_DEBOUNCE_MS
    } else {
        let Some(num) = deb_val.as_number() else {
            return Err(Exception::throw_type(
                ctx,
                "options.debounceMs must be a positive integer",
            ));
        };
        if !num.is_finite()
            || num.fract() != 0.0
            || num < MIN_WATCH_DEBOUNCE_MS as f64
            || num > MAX_WATCH_DEBOUNCE_MS as f64
        {
            return Err(Exception::throw_range(
                ctx,
                &format!(
                    "options.debounceMs must be between {MIN_WATCH_DEBOUNCE_MS} and {MAX_WATCH_DEBOUNCE_MS}"
                ),
            ));
        }
        num as u64
    };
    Ok((recursive, debounce_ms))
}

fn stat_to_js<'js>(ctx: &Ctx<'js>, s: FileStat) -> rquickjs::Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.prop("isFile", data_property(s.is_file))?;
    obj.prop("isDir", data_property(s.is_dir))?;
    obj.prop("size", data_property(s.size as f64))?;
    match s.modified_ms {
        Some(ms) => obj.prop("modifiedMs", data_property(ms as f64))?,
        None => obj.prop("modifiedMs", data_property(Value::new_null(ctx.clone())))?,
    }
    Ok(obj)
}

fn dir_entries_to_js<'js>(ctx: &Ctx<'js>, entries: Vec<DirEntry>) -> rquickjs::Result<Array<'js>> {
    let arr = Array::new(ctx.clone())?;
    for (idx, entry) in entries.into_iter().enumerate() {
        let obj = Object::new(ctx.clone())?;
        obj.prop("name", data_property(entry.name.as_str()))?;
        obj.prop("isFile", data_property(entry.is_file))?;
        obj.prop("isDir", data_property(entry.is_dir))?;
        obj.prop("size", data_property(entry.size as f64))?;
        arr.set(idx, obj)?;
    }
    Ok(arr)
}

fn bookmarks_to_js<'js>(
    ctx: &Ctx<'js>,
    records: Vec<BookmarkRecord>,
) -> rquickjs::Result<Array<'js>> {
    let arr = Array::new(ctx.clone())?;
    for (idx, rec) in records.into_iter().enumerate() {
        let obj = Object::new(ctx.clone())?;
        obj.prop("token", data_property(rec.token.as_str()))?;
        obj.prop(
            "kind",
            data_property(match rec.kind {
                HandleKind::File => "file",
                HandleKind::Dir => "dir",
            }),
        )?;
        obj.prop("label", data_property(rec.label.as_str()))?;
        obj.prop("mode", data_property(rec.mode.as_str()))?;
        obj.prop("createdMs", data_property(rec.created_ms as f64))?;
        arr.set(idx, obj)?;
    }
    Ok(arr)
}

fn watch_events_to_js<'js>(ctx: &Ctx<'js>, events: Vec<WatchEvent>) -> rquickjs::Result<Array<'js>> {
    let arr = Array::new(ctx.clone())?;
    for (idx, ev) in events.into_iter().enumerate() {
        let obj = Object::new(ctx.clone())?;
        obj.prop("path", data_property(ev.path.as_str()))?;
        obj.prop("kind", data_property(ev.kind.as_str()))?;
        arr.set(idx, obj)?;
    }
    Ok(arr)
}
