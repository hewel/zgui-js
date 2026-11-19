//! Session-local path authority. Only live resolution consults host directories and environment.
use std::{collections::BTreeMap, ffi::OsString, fs, io, path::Path};

use crate::{
    assets::ContractFailure,
    package::Capabilities,
    runtime_paths::{self, DISPLAY, EMBEDDED_DISPLAY, Root},
};

const ENV_SOCKETS: &[(&str, &str)] = &[("niri", "NIRI_SOCKET")];

/// Immutable path authority resolved from one sealed manifest before evaluation.
#[derive(Debug, Default)]
pub(crate) struct ResolvedCapabilities {
    image_prefixes: Vec<String>,
    unix_sockets: Vec<String>,
    named_sockets: BTreeMap<String, Option<String>>,
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
        Ok(resolved)
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
