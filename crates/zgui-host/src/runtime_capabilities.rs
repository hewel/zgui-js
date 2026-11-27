//! Session-local path authority. Only live resolution consults host directories and environment.
use std::{borrow::Cow, cell::OnceCell, collections::BTreeMap, ffi::OsString, fs, io, path::{Path, PathBuf}};

use host_services::files::FileMode;

use crate::{
    assets::ContractFailure,
    package::Capabilities,
    runtime_paths::{self, DISPLAY, EMBEDDED_DISPLAY, Root},
};

const ENV_SOCKETS: &[(&str, &str)] = &[("niri", "NIRI_SOCKET")];

#[derive(Debug)]
pub(crate) struct ResolvedFileDirectory {
    pub(crate) declaration: String,
    pub(crate) path: PathBuf,
    pub(crate) mode: FileMode,
}

/// Immutable path authority resolved from one sealed manifest before evaluation.
#[derive(Debug, Default)]
pub(crate) struct ResolvedCapabilities {
    image_prefixes: Vec<String>,
    unix_sockets: Vec<String>,
    named_sockets: BTreeMap<String, Option<String>>,
    file_directories: Vec<ResolvedFileDirectory>,
}

impl ResolvedCapabilities {
    /// Resolves live declarations with explicit bindings taking precedence over the env allowlist.
    ///
    /// # Errors
    /// Fails if a required root, display, or resolved socket path is invalid.
    pub(crate) fn live(
        declarations: &Capabilities,
        bindings: &BTreeMap<String, String>,
        env: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Self, ContractFailure> {
        let resolved = Self::resolve(
            declarations,
            |root| live_root(root, &env),
            || env_text(&env, "WAYLAND_DISPLAY"),
            |name| {
                if let Some(path) = bindings.get(name) {
                    return Ok(Some(path.clone()));
                }
                let key = ENV_SOCKETS
                    .iter()
                    .find_map(|&(allowed, key)| (name == allowed).then_some(key));
                key.map_or(Ok(None), |key| env_text(&env, key))
            },
        )?;
        for path in resolved
            .unix_sockets
            .iter()
            .chain(resolved.named_sockets.values().flatten())
        {
            require_canonical_socket_path(Path::new(path))
                .map_err(|error| denied(format!("socket path {path:?}: {error}")))?;
        }
        Ok(resolved.with_file_directories(
            declarations,
            || absolute_env_path(&env, "HOME").map(Cow::Owned),
            || absolute_env_path(&env, "XDG_DATA_HOME").map(Cow::Owned),
            || absolute_env_path(&env, "XDG_DOCUMENTS_DIR").map(Cow::Owned),
            |home| live_documents_from_home(&env, home),
        ))
    }

    /// Resolves only the sealed declarations using the supplied path values.
    ///
    /// # Errors
    /// Fails if a required template variable or supplied binding is invalid.
    pub(crate) fn resolve(
        declarations: &Capabilities,
        mut root: impl FnMut(Root) -> Result<String, String>,
        mut display: impl FnMut() -> Result<Option<String>, String>,
        mut named: impl FnMut(&str) -> Result<Option<String>, String>,
    ) -> Result<Self, ContractFailure> {
        let mut roots = [None, None, None];
        let mut display_value = None;
        let mut path = |declaration: &str| -> Result<String, String> {
            if declaration.starts_with('/') {
                return Ok(declaration.to_owned());
            }
            let (kind, tail) =
                runtime_paths::template(declaration).ok_or("invalid capability path template")?;
            let cached = &mut roots[kind as usize];
            let base = match cached {
                Some(value) => value,
                None => cached.insert(root(kind)?),
            };
            if tail.is_empty() {
                return Ok(base.to_owned());
            }
            let base = base.trim_end_matches('/');
            let mut resolved = String::with_capacity(base.len() + 1 + tail.len());
            resolved.push_str(base);
            for segment in tail.split('/') {
                resolved.push('/');
                if !segment.contains('$') {
                    resolved.push_str(segment);
                    continue;
                }
                let display_value = match &mut display_value {
                    Some(value) => value,
                    None => display_value.insert(
                        display()?
                            .filter(|value| runtime_paths::safe_segment(value))
                            .ok_or("WAYLAND_DISPLAY must be a single safe filename component")?,
                    ),
                };
                if segment == DISPLAY {
                    resolved.push_str(display_value);
                } else {
                    let mut parts = segment.split(EMBEDDED_DISPLAY);
                    resolved.push_str(parts.next().unwrap_or_default());
                    for literal in parts {
                        resolved.push_str(display_value);
                        resolved.push_str(literal);
                    }
                }
            }
            Ok(resolved)
        };
        let mut paths = |entries: &[String]| {
            entries
                .iter()
                .map(|entry| path(entry))
                .collect::<Result<Vec<_>, _>>()
                .map_err(denied)
        };
        let image_prefixes = paths(&declarations.image_prefixes)?;
        let unix_sockets = paths(&declarations.unix_sockets)?;
        let named_sockets = declarations
            .named_sockets
            .iter()
            .map(|name| {
                let path = named(name).map_err(denied)?;
                if path
                    .as_deref()
                    .is_some_and(|path| !runtime_paths::canonical_absolute(path))
                {
                    return Err(denied(format!(
                        "named socket {name:?} must bind to a canonical absolute path"
                    )));
                }
                Ok((name.clone(), path))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            image_prefixes,
            unix_sockets,
            named_sockets,
            file_directories: Vec::new(),
        })
    }

    /// Resolved prefixes for the native image policy.
    #[must_use]
    pub(crate) fn image_prefixes(&self) -> &[String] {
        &self.image_prefixes
    }

    /// Resolved literal and templated declarations in `unixSockets` order.
    #[must_use]
    pub(crate) fn unix_sockets(&self) -> &[String] {
        &self.unix_sockets
    }

    /// Manifest-declared names, including optional endpoints with no binding.
    #[must_use]
    pub(crate) fn named_sockets(&self) -> &BTreeMap<String, Option<String>> {
        &self.named_sockets
    }

    /// Resolve only roots referenced by file grants; unresolved roots grant nothing.
    pub(crate) fn with_file_directories<'a>(
        mut self,
        declarations: &Capabilities,
        resolve_home: impl Fn() -> Option<Cow<'a, Path>>,
        resolve_data: impl Fn() -> Option<Cow<'a, Path>>,
        resolve_documents: impl Fn() -> Option<Cow<'a, Path>>,
        documents_from_home: impl Fn(&Path) -> PathBuf,
    ) -> Self {
        let home = OnceCell::new();
        let data = OnceCell::new();
        let documents = OnceCell::new();
        let home = || home.get_or_init(&resolve_home).as_deref();
        self.file_directories = declarations.files.directories.iter().filter_map(|grant| {
            let path = if grant.path.starts_with('$') {
                let (key, rest) = grant.path.split_once('/').unwrap_or((&grant.path, ""));
                let base = match key {
                    "$HOME" => home(),
                    "$XDG_DATA_HOME" => data.get_or_init(|| {
                        resolve_data().or_else(|| Some(Cow::Owned(home()?.join(".local/share"))))
                    }).as_deref(),
                    "$XDG_DOCUMENTS_DIR" => documents.get_or_init(|| {
                        resolve_documents().or_else(|| home().map(|home| Cow::Owned(documents_from_home(home))))
                    }).as_deref(),
                    _ => None,
                }?;
                if rest.is_empty() { base.to_path_buf() } else { base.join(rest) }
            } else {
                let path = Path::new(&grant.path);
                if !path.is_absolute() { return None; }
                path.to_path_buf()
            };
            Some(ResolvedFileDirectory { declaration: grant.path.clone(), path, mode: grant.mode })
        }).collect();
        self
    }

    /// Manifest-order file grants; unresolved live templates do not grant authority.
    pub(crate) fn file_directories(&self) -> &[ResolvedFileDirectory] {
        &self.file_directories
    }

    /// Authorizes only exact paths from either socket declaration list.
    #[must_use]
    pub(crate) fn allows_socket(&self, path: &str) -> bool {
        self.unix_sockets().iter().any(|allowed| allowed == path)
            || self
                .named_sockets()
                .values()
                .flatten()
                .any(|allowed| allowed == path)
    }
}

/// Parses one explicit live CLI binding without granting authority to a package.
///
/// # Errors
/// Fails for malformed or duplicate bindings.
pub(crate) fn add_socket_binding(
    bindings: &mut BTreeMap<String, String>,
    spec: &str,
) -> Result<(), String> {
    let (name, path) = spec
        .split_once('=')
        .ok_or("--bind-socket requires name=path")?;
    if !runtime_paths::socket_name(name) || !runtime_paths::canonical_absolute(path) {
        return Err("--bind-socket requires a valid name and canonical absolute path".into());
    }
    match bindings.entry(name.to_owned()) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(path.to_owned());
            Ok(())
        }
        std::collections::btree_map::Entry::Occupied(_) => {
            Err(format!("duplicate --bind-socket name {name:?}"))
        }
    }
}

fn denied(message: String) -> ContractFailure {
    ContractFailure {
        code: "E_CAPABILITY",
        message,
    }
}

fn require_canonical_socket_path(path: &Path) -> io::Result<()> {
    // A daemon may not have bound its socket yet, but existing ancestors must not be aliases.
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
            Ok(_) => {
                if fs::canonicalize(ancestor)?.as_os_str() != ancestor.as_os_str() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "path must be canonical and not contain symlinks",
                    ));
                }
                return Ok(());
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "socket path has no existing ancestor",
    ))
}

fn env_text(env: &impl Fn(&str) -> Option<OsString>, key: &str) -> Result<Option<String>, String> {
    env(key)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| format!("{key} must be valid UTF-8"))
        })
        .transpose()
}

fn absolute_env_path(env: &impl Fn(&str) -> Option<OsString>, key: &str) -> Option<PathBuf> {
    env_text(env, key).ok().flatten().map(PathBuf::from).filter(|path| path.is_absolute())
}

fn live_documents_from_home(env: &impl Fn(&str) -> Option<OsString>, home: &Path) -> PathBuf {
    let config = absolute_env_path(env, "XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
    if let Ok(text) = fs::read_to_string(config.join("user-dirs.dirs")) {
        for line in text.lines() {
            if let Some(value) = line.trim().strip_prefix("XDG_DOCUMENTS_DIR=") {
                let value = value.trim().trim_matches('"');
                if let Some(relative) = value.strip_prefix("$HOME/") {
                    return home.join(relative);
                }
                if Path::new(value).is_absolute() {
                    return value.into();
                }
            }
        }
    }
    home.join("Documents")
}

fn live_root(root: Root, env: &impl Fn(&str) -> Option<OsString>) -> Result<String, String> {
    let key = root.key();
    let value = env_text(env, key)?;
    if matches!(root, Root::Data) && value.is_none() {
        let home = live_root(Root::Home, env)?;
        let data = runtime_paths::default_data_home(&home);
        // The standard default may not exist yet; existing ancestors must be canonical.
        for ancestor in Path::new(&data).ancestors() {
            match fs::symlink_metadata(ancestor) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("{key} default: {error}")),
                Ok(_) => {
                    runtime_paths::open_canonical_dir(ancestor)
                        .map_err(|error| format!("{key} default: {error}"))?;
                    return Ok(data);
                }
            }
        }
        return Err(format!("{key} default has no existing directory ancestor"));
    }
    let value = value
        .filter(|path| runtime_paths::canonical_absolute(path))
        .ok_or_else(|| format!("{key} must be set to a canonical absolute directory"))?;
    let directory = runtime_paths::open_canonical_dir(Path::new(&value))
        .map_err(|error| format!("{key}: {error}"))?;
    if matches!(root, Root::Runtime) {
        runtime_paths::require_private(&directory, 0o7777)
            .map_err(|error| format!("{key}: {error}"))?;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_package::Fixture;
    use std::os::unix::{fs::symlink, net::UnixListener};

    #[test]
    fn live_file_directories_keep_declarations_modes_and_skip_missing_roots() {
        use crate::package::{FileDirectoryGrant, FilesCapabilities};
        use host_services::files::FileMode;

        let declarations = Capabilities {
            files: FilesCapabilities {
                directories: [
                    ("/literal", FileMode::Read),
                    ("$HOME/work", FileMode::ReadWrite),
                    ("$XDG_DATA_HOME/cache", FileMode::Read),
                    ("$XDG_DOCUMENTS_DIR/notes", FileMode::Read),
                ].into_iter().map(|(path, mode)| FileDirectoryGrant { path: path.into(), mode }).collect(),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolve = |home, data, docs| ResolvedCapabilities::live(&declarations, &BTreeMap::new(), |key| {
            match key {
                "HOME" => home,
                "XDG_DATA_HOME" => data,
                "XDG_DOCUMENTS_DIR" => docs,
                "XDG_CONFIG_HOME" => Some("/dev/null"),
                _ => None,
            }.map(OsString::from)
        }).unwrap();
        let all = resolve(Some("/test-home"), Some("/test-data"), Some("/test-documents"));
        let paths = |caps: &ResolvedCapabilities| caps.file_directories().iter()
            .map(|grant| grant.path.to_str().unwrap().to_owned()).collect::<Vec<_>>();
        assert_eq!(paths(&all), ["/literal", "/test-home/work", "/test-data/cache", "/test-documents/notes"]);
        assert_eq!(all.file_directories()[1].declaration, "$HOME/work");
        assert_eq!(all.file_directories()[1].mode, FileMode::ReadWrite);
        assert_eq!(paths(&resolve(None, Some("relative"), None)), ["/literal"]);
        assert_eq!(paths(&resolve(Some("/test-home"), None, None)),
            ["/literal", "/test-home/work", "/test-home/.local/share/cache", "/test-home/Documents/notes"]);
    }

    #[test]
    fn live_resolution_rejects_socket_symlinks_before_publishing_authority() {
        let fixture = Fixture::new("", "");
        let root = &fixture.package().directory;
        let target = root.join("real.sock");
        let _listener = UnixListener::bind(&target).expect("bind the real socket");
        let socket_link = root.join("linked.sock");
        symlink(&target, &socket_link).expect("link to the real socket");
        let parent_link = root.join("linked-parent");
        symlink(root, &parent_link).expect("link to the socket directory");
        let dangling = root.join("dangling.sock");
        symlink(root.join("absent.sock"), &dangling).expect("link to a missing socket");

        let declarations = Capabilities {
            named_sockets: vec!["niri".into()],
            ..Default::default()
        };
        let mut bindings = BTreeMap::new();
        add_socket_binding(&mut bindings, &format!("niri={}", socket_link.display()))
            .expect("a symlink path can still pass lexical CLI validation");
        let failure = ResolvedCapabilities::live(&declarations, &bindings, |_| None)
            .expect_err("an existing symlink cannot be published as a named endpoint");
        assert_eq!(failure.code, "E_CAPABILITY", "{failure}");

        for path in [parent_link.join("not-created.sock"), dangling] {
            let failure = ResolvedCapabilities::live(&declarations, &BTreeMap::new(), |key| {
                (key == "NIRI_SOCKET").then(|| path.as_os_str().to_owned())
            })
            .expect_err("a symlink cannot grant authority before its target exists");
            assert_eq!(failure.code, "E_CAPABILITY", "{path:?}: {failure}");
        }

        let literal = parent_link.join("real.sock");
        let declarations = Capabilities {
            unix_sockets: vec![literal.to_str().expect("UTF-8 test socket path").to_owned()],
            ..Default::default()
        };
        let failure = ResolvedCapabilities::live(&declarations, &BTreeMap::new(), |_| None)
            .expect_err("a literal socket path cannot have a symlinked parent");
        assert_eq!(failure.code, "E_CAPABILITY", "{failure}");
    }
}
