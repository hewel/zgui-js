use host_services::system_files::{PreviewSystemFiles, ThumbnailSize, Volume, valid_preview_path};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{decode_base64, unique_entries};
use crate::schema::object_struct;

object_struct! {
    #[derive(Default)]
    #[serde(default, rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct SystemFilesFixture {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        dirs: Vec<String>,
        #[serde(deserialize_with = "unique_entries", skip_serializing_if = "BTreeMap::is_empty")]
        files: BTreeMap<String, String>,
        #[serde(deserialize_with = "unique_entries", skip_serializing_if = "BTreeMap::is_empty")]
        symlinks: BTreeMap<String, String>,
        #[serde(deserialize_with = "unique_entries", skip_serializing_if = "BTreeMap::is_empty")]
        metadata: BTreeMap<String, Metadata>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        generated: Vec<Generated>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        volumes: Vec<Volume>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        thumbnails: Vec<Thumbnail>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        dialogs: Vec<Option<PathBuf>>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        trash_unavailable: bool,
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Metadata {
    mode: Option<u32>,
    modified_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Generated {
    directory: PathBuf,
    prefix: String,
    suffix: String,
    count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Thumbnail {
    path: PathBuf,
    #[serde(default)]
    size: ThumbnailSize,
    mtime: i64,
    png: String,
}

impl SystemFilesFixture {
    pub(super) fn validate(&self) -> Result<(), String> {
        for path in self
            .dirs
            .iter()
            .chain(self.files.keys())
            .chain(self.symlinks.keys())
            .chain(self.metadata.keys())
        {
            if !valid_preview_path(path) {
                return Err(format!(
                    "fixture systemFiles path is not canonical absolute: {path:?}"
                ));
            }
        }
        for path in self
            .generated
            .iter()
            .map(|entry| &entry.directory)
            .chain(self.thumbnails.iter().map(|entry| &entry.path))
            .chain(self.dialogs.iter().flatten())
        {
            if !path.to_str().is_some_and(valid_preview_path) {
                return Err(format!(
                    "fixture systemFiles path is not canonical absolute: {path:?}"
                ));
            }
        }
        if self
            .generated
            .iter()
            .map(|entry| u64::from(entry.count))
            .sum::<u64>()
            > 100_000
        {
            return Err("fixture systemFiles generated entries exceed 100000".into());
        }
        for entry in &self.generated {
            if entry.prefix.contains(['/', '\0'])
                || entry.suffix.contains(['/', '\0'])
                || entry.prefix.len() + entry.suffix.len() > 240
            {
                return Err("fixture systemFiles generated names are invalid".into());
            }
        }
        if self
            .symlinks
            .values()
            .any(|target| target.is_empty() || target.contains('\0'))
        {
            return Err("fixture systemFiles symlink target is invalid".into());
        }
        Ok(())
    }

    pub(super) fn build(&self) -> Result<Arc<PreviewSystemFiles>, String> {
        let mut preview = PreviewSystemFiles::default();
        for dir in &self.dirs {
            preview.seed_directory(Path::new(dir));
        }
        for group in &self.generated {
            preview.seed_directory(&group.directory);
            for i in 1..=group.count {
                preview.seed_file(
                    &group
                        .directory
                        .join(format!("{}{i}{}", group.prefix, group.suffix)),
                    0,
                );
            }
        }
        for (path, value) in &self.files {
            preview.seed_file(Path::new(path), value.len() as u64);
        }
        for (path, target) in &self.symlinks {
            preview.seed_symlink(Path::new(path), target.into());
        }
        for (path, metadata) in &self.metadata {
            preview
                .seed_metadata(Path::new(path), metadata.mode, metadata.modified_ms)
                .map_err(|error| format!("fixture systemFiles metadata {path:?}: {error}"))?;
        }
        for entry in &self.thumbnails {
            let bytes = decode_base64(&entry.png)?;
            if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err("fixture thumbnail is not a PNG".into());
            }
            preview.seed_thumbnail(entry.path.clone(), entry.size, entry.mtime, bytes);
        }
        preview.trash_unavailable = self.trash_unavailable;
        preview.volumes.clone_from(&self.volumes);
        preview
            .dialogs
            .get_mut()
            .expect("private fixture initialization")
            .extend(self.dialogs.iter().cloned());
        Ok(Arc::new(preview))
    }
}
