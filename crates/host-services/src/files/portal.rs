//! Linux XDG Desktop Portal `org.freedesktop.portal.FileChooser` backend over `zbus`.
#![forbid(unsafe_code)]

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

use crate::{
    ServiceError,
    files::{DialogFilter, DialogOptions},
};

const PORTAL_BUS: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER_IFACE: &str = "org.freedesktop.portal.FileChooser";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

static PORTAL_TOKEN_SEQ: AtomicU64 = AtomicU64::new(1);

/// File chooser dialog mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalDialogKind {
    OpenFile,
    SaveFile,
    OpenDirectory,
}

/// Runs an XDG FileChooser Portal request synchronously (call from a worker thread!).
///
/// Returns `Ok(None)` when the user cancels the dialog, or `Ok(Some(path))` on selection.
pub fn run_file_chooser(
    kind: PortalDialogKind,
    options: &DialogOptions,
) -> Result<Option<PathBuf>, ServiceError> {
    let conn = Connection::session().map_err(|err| {
        ServiceError::new(
            "PortalUnavailable",
            format!("cannot connect to D-Bus session bus: {err}"),
        )
    })?;
    let unique = conn
        .unique_name()
        .ok_or_else(|| ServiceError::new("PortalUnavailable", "D-Bus connection has no unique name"))?;
    let sender_segment = unique.as_str().trim_start_matches(':').replace('.', "_");
    let seq = PORTAL_TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
    let handle_token = format!("zgui_fc_{}_{seq}", std::process::id());
    let expected_path = format!("/org/freedesktop/portal/desktop/request/{sender_segment}/{handle_token}");

    let request_proxy = Proxy::new(&conn, PORTAL_BUS, expected_path.as_str(), REQUEST_IFACE)
        .map_err(|err| ServiceError::new("PortalUnavailable", err.to_string()))?;
    let mut signals = request_proxy
        .receive_signal("Response")
        .map_err(|err| ServiceError::new("PortalUnavailable", err.to_string()))?;

    let chooser_proxy = Proxy::new(&conn, PORTAL_BUS, PORTAL_PATH, FILE_CHOOSER_IFACE)
        .map_err(|err| ServiceError::new("PortalUnavailable", err.to_string()))?;

    let mut portal_opts: HashMap<&str, Value<'_>> = HashMap::new();
    portal_opts.insert("handle_token", Value::from(handle_token.as_str()));
    portal_opts.insert("modal", Value::from(true));

    let method = match kind {
        PortalDialogKind::OpenFile => {
            portal_opts.insert("multiple", Value::from(false));
            portal_opts.insert("directory", Value::from(false));
            "OpenFile"
        }
        PortalDialogKind::OpenDirectory => {
            portal_opts.insert("multiple", Value::from(false));
            portal_opts.insert("directory", Value::from(true));
            "OpenFile"
        }
        PortalDialogKind::SaveFile => {
            if let Some(suggested) = &options.suggested_name {
                portal_opts.insert("current_name", Value::from(suggested.as_str()));
            }
            "SaveFile"
        }
    };

    let encoded_filters = encode_portal_filters(&options.filters);
    if !encoded_filters.is_empty() {
        portal_opts.insert("filters", Value::from(encoded_filters));
    }

    let title = options.title.as_deref().unwrap_or(match kind {
        PortalDialogKind::OpenFile => "Open File",
        PortalDialogKind::SaveFile => "Save File",
        PortalDialogKind::OpenDirectory => "Open Directory",
    });

    let _returned_path: OwnedObjectPath = chooser_proxy
        .call(method, &("", title, portal_opts))
        .map_err(|err| ServiceError::new("PortalUnavailable", format!("FileChooser.{method} failed: {err}")))?;

    let Some(signal) = signals.next() else {
        return Err(ServiceError::new(
            "PortalUnavailable",
            "FileChooser portal closed before responding",
        ));
    };
    let (response_code, results): (u32, HashMap<String, OwnedValue>) = signal
        .body()
        .deserialize()
        .map_err(|err| ServiceError::new("PortalUnavailable", format!("invalid portal response: {err}")))?;

    match response_code {
        0 => {
            let Some(uris_val) = results.get("uris") else {
                return Ok(None);
            };
            let uris: Vec<String> = uris_val
                .try_clone()
                .ok()
                .and_then(|v| Vec::<String>::try_from(v).ok())
                .unwrap_or_default();
            let Some(first_uri) = uris.first() else {
                return Ok(None);
            };
            let path = decode_file_uri(first_uri)?;
            Ok(Some(path))
        }
        1 => Ok(None), // User cancelled.
        other => Err(ServiceError::new(
            "PortalError",
            format!("XDG FileChooser portal returned error response {other}"),
        )),
    }
}

/// Encodes platform-neutral extension filters into XDG FileChooser `a(sa(us))` tuples.
fn encode_portal_filters(filters: &[DialogFilter]) -> Vec<(String, Vec<(u32, String)>)> {
    filters
        .iter()
        .filter(|f| !f.extensions.is_empty())
        .map(|f| {
            let patterns = f
                .extensions
                .iter()
                .map(|ext| {
                    let clean = ext.trim_start_matches('.').trim_start_matches("*.");
                    (0_u32, format!("*.{clean}"))
                })
                .collect();
            (f.name.clone(), patterns)
        })
        .collect()
}

/// Decodes a `file:///...` URI returned by the XDG Portal into an absolute `PathBuf`.
pub fn decode_file_uri(uri: &str) -> Result<PathBuf, ServiceError> {
    let raw = uri
        .strip_prefix("file://localhost")
        .or_else(|| uri.strip_prefix("file://"))
        .ok_or_else(|| ServiceError::new("InvalidPath", format!("unsupported URI scheme: {uri:?}")))?;
    if !raw.starts_with('/') {
        return Err(ServiceError::new(
            "InvalidPath",
            format!("file URI must be absolute: {uri:?}"),
        ));
    }
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        if bytes[idx] == b'%' && idx + 2 < bytes.len() {
            let hi = hex_val(bytes[idx + 1]);
            let lo = hex_val(bytes[idx + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                let b = (hi << 4) | lo;
                if b == 0 {
                    return Err(ServiceError::new("InvalidPath", "file URI contains NUL byte"));
                }
                decoded.push(b);
                idx += 3;
                continue;
            }
        }
        decoded.push(bytes[idx]);
        idx += 1;
    }
    let text = String::from_utf8(decoded)
        .map_err(|_| ServiceError::new("InvalidPath", "file URI is not valid UTF-8"))?;
    Ok(PathBuf::from(text))
}

const fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uri_decoding_handles_percent_encoding_and_rejects_bad_schemes() {
        assert_eq!(
            decode_file_uri("file:///home/user/My%20Notes/hello%231.md").unwrap(),
            PathBuf::from("/home/user/My Notes/hello#1.md")
        );
        assert_eq!(
            decode_file_uri("file://localhost/tmp/a.txt").unwrap(),
            PathBuf::from("/tmp/a.txt")
        );
        assert!(decode_file_uri("https://example.com/a.md").is_err());
        assert!(decode_file_uri("file:///tmp/bad%00.txt").is_err());
    }
}
