//! POSIX system files. Workspace handles do not use this authority or resolver.

mod external;
mod jobs;
mod metadata;
mod path;
mod preview;
mod thumbnail;
mod trash;
mod volumes;

pub use external::{external_url, launch_external};

pub use jobs::{
    Conflict, ConflictAction, ConflictResolution, EntryRef, Job, JobResult, Progress, run_job,
};
use metadata::paginate;
pub use metadata::{DirPage, EntryInfo, EntryKind, ReadOptions, SortBy, SortDirection, mime};
pub use path::{Root, Scope, locate_dropped_entry};
pub use preview::{PreviewSystemFiles, valid_preview_path};
pub use thumbnail::{
    ThumbnailSize, cache_home, cached_thumbnail, file_uri, thumbnail_key, thumbnail_mtime,
};
pub use trash::{Trash, TrashEntry};
pub use volumes::{Volume, list_volumes};
