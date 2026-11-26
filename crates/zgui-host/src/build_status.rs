//! Strict wire contract for the builder's package publication status.
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

pub(crate) const STATUS_FILE: &str = ".zgui-build-status.json";
pub(crate) const MAX_STATUS_BYTES: u64 = 16 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    Building,
    Published,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "lowercase", deny_unknown_fields)]
pub(crate) enum Typecheck {
    Running { diagnostics: () },
    Passed { diagnostics: () },
    Failed { diagnostics: String },
}

// Without a default these fields must be present, even when their value is null.
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}

fn present_typecheck<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Typecheck>, D::Error> {
    crate::schema::deserialize_object(deserializer).map(Some)
}

crate::schema::object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(crate) struct BuildStatus {
        schema_version: u32,
        pub(crate) attempt_id: String,
        app: String,
        pub(crate) status: Status,
        #[serde(deserialize_with = "nullable")]
        pub(crate) previous_directory: Option<PathBuf>,
        #[serde(deserialize_with = "nullable")]
        pub(crate) publication_directory: Option<PathBuf>,
        #[serde(deserialize_with = "nullable")]
        build_id: Option<String>,
        #[serde(deserialize_with = "nullable")]
        pub(crate) error: Option<BuildError>,
        #[serde(default, deserialize_with = "present_typecheck", skip_serializing_if = "Option::is_none")]
        pub(crate) typecheck: Option<Typecheck>,
    }
}

crate::schema::object_struct! {
    #[serde(deny_unknown_fields)]
    pub(crate) struct BuildError { code: String, pub(crate) message: String }
}

impl BuildStatus {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let record: Self = serde_json::from_slice(bytes)?;
        let identity_valid = record.schema_version == 1
            && record.attempt_id.len() == 36
            && record.attempt_id.bytes().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
            && !record.app.is_empty()
            && record.app.len() <= 128
            && record
                .app
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && record
                .previous_directory
                .as_ref()
                .is_none_or(|path| path.is_absolute());
        let state_valid = match record.status {
            Status::Published => {
                record
                    .publication_directory
                    .as_ref()
                    .is_some_and(|path| path.is_absolute())
                    && record.build_id.as_deref().is_some_and(|id| {
                        id.strip_prefix("sha256:").is_some_and(|digest| {
                            digest.len() == 64
                                && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                    })
                    && record.error.is_none()
            }
            Status::Building => {
                record.publication_directory.is_none()
                    && record.build_id.is_none()
                    && record.error.is_none()
            }
            Status::Failed => {
                record.publication_directory.is_none()
                    && record.build_id.is_none()
                    && record.error.as_ref().is_some_and(|error| {
                        error.code == "E_BUILD" && error.message.chars().count() <= 2048
                    })
            }
        };
        if !identity_valid || !state_valid {
            return Err(serde_json::Error::custom(
                "invalid build status identity or state",
            ));
        }
        Ok(record)
    }
}
