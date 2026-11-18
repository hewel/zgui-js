use super::{Error, output::digest};
use serde::Deserialize;
use serde_json::{Value, json};

const CARGO_LOCK: &str = include_str!("../../../../Cargo.lock");
const PNPM_LOCK: &str = include_str!("../../../../pnpm-lock.yaml");

#[derive(Deserialize)]
struct CargoLock { package: Vec<LockedPackage> }
#[derive(Deserialize, serde::Serialize)]
struct LockedPackage {
    name: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checksum: Option<String>,
}

pub(crate) fn snapshot() -> Result<Value, Error> {
    let cargo: CargoLock = toml::from_str(CARGO_LOCK)
        .map_err(|error| Error::new("E_HOST_INVARIANT", 9, format!("embedded Cargo lockfile: {error}")))?;
    let javascript = pnpm_package_keys(PNPM_LOCK)?;
    Ok(json!({
        "schemaVersion": 1,
        "host": {
            "version": env!("CARGO_PKG_VERSION"),
            "rustc": env!("ZGUI_BUILD_RUSTC"),
            "target": env!("ZGUI_BUILD_TARGET"),
            "profile": env!("ZGUI_BUILD_PROFILE"),
            "sourceTreeDigest": format!("sha256:{}", env!("ZGUI_BUILD_SOURCE_SHA256")),
            "sourceTreeScope": "workspace Rust sources, manifests, lockfiles and local native patches; includes test sources"
        },
        "dependencies": {
            "cargoLockDigest": digest(CARGO_LOCK.as_bytes()),
            "pnpmLockDigest": digest(PNPM_LOCK.as_bytes()),
            "resolvedCargoPackages": cargo.package,
            "resolvedJavascriptPackages": javascript,
            "javascriptIdentityKind": "exact pnpm v9 package resolution keys from every lockfile document",
            "quickJsEngineRevision": "6d46d07d04041b40f4f49eaa7fdebe44c314c699",
            "solidAuditedSourceRevision": "82f71fddb1340cab46167db119da5702038e65ab",
            "solidSourceRevisionMeaning": "specification audit reference, not an assertion that registry bytes are that git checkout"
        }
    }))
}

// This indexes the exact package keys of the embedded, pnpm-generated v9 lockfile,
// not arbitrary YAML or the dependency graph. Preserve peer-qualified identities
// rather than guessing which @ separates a package name from its version.
fn pnpm_package_keys(lock: &str) -> Result<Vec<String>, Error> {
    let invalid = || Error::new("E_HOST_INVARIANT", 9, "unsupported embedded pnpm lockfile representation");
    let mut packages = Vec::new();
    let mut in_packages = false;
    let mut version_seen = false;
    for line in lock.lines() {
        if line == "---" { in_packages = false; version_seen = false; continue; }
        if line.starts_with("lockfileVersion:") {
            if line != "lockfileVersion: '9.0'" { return Err(invalid()); }
            version_seen = true;
        }
        if line == "packages:" {
            if !version_seen { return Err(invalid()); }
            in_packages = true;
            continue;
        }
        if !line.is_empty() && !line.starts_with(' ') { in_packages = false; }
        if !in_packages || !line.starts_with("  ") || line.starts_with("   ") { continue; }
        let key = line.strip_prefix("  ").and_then(|line| line.strip_suffix(':')).ok_or_else(invalid)?;
        let key = if key.starts_with('\'') {
            key.strip_prefix('\'').and_then(|key| key.strip_suffix('\'')).ok_or_else(invalid)?
        } else { key };
        if key.is_empty() || !key.is_ascii() || !key.contains('@') || key.contains(['\'', '"', '\\', ' ']) {
            return Err(invalid());
        }
        packages.push(key.to_owned());
    }
    if packages.is_empty() { return Err(invalid()); }
    packages.sort_unstable();
    packages.dedup();
    Ok(packages)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventories_all_lock_documents_without_confusing_snapshot_entries() {
        let lock = "---\nlockfileVersion: '9.0'\npackages:\n  '@scope/tool@1.2.3':\n    resolution: {}\nsnapshots:\n  ignored@1.0.0: {}\n---\nlockfileVersion: '9.0'\npackages:\n  solid-js@1.9.19:\n    resolution: {}\nsnapshots:\n  ignored@2.0.0: {}\n";
        assert_eq!(pnpm_package_keys(lock).unwrap(), ["@scope/tool@1.2.3", "solid-js@1.9.19"]);
        assert!(pnpm_package_keys(&lock.replace("'9.0'", "'10.0'")).is_err());
    }
}
