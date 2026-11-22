use super::{Error, output::digest};
use serde::Deserialize;
use serde_json::{Value, json};

const CARGO_LOCK: &str = include_str!("../../../../Cargo.lock");
const BUN_LOCK: &str = include_str!("../../../../bun.lock");

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
    let javascript = bun_package_keys(BUN_LOCK)?;
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
            "bunLockDigest": digest(BUN_LOCK.as_bytes()),
            "resolvedCargoPackages": cargo.package,
            "resolvedJavascriptPackages": javascript,
            "javascriptIdentityKind": "exact bun lockfile v2 package resolution keys",
            "quickJsEngineRevision": "6d46d07d04041b40f4f49eaa7fdebe44c314c699",
            "solidAuditedSourceRevision": "82f71fddb1340cab46167db119da5702038e65ab",
            "solidSourceRevisionMeaning": "specification audit reference, not an assertion that registry bytes are that git checkout"
        }
    }))
}

fn strip_jsonc_trailing_commas(input: &str) -> Result<String, Error> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            out.push(byte);
            if escaped { escaped = false; }
            else if byte == b'\\' { escaped = true; }
            else if byte == b'"' { in_string = false; }
            index += 1;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' { index += 1; }
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') { index += 1; }
            index = (index + 2).min(bytes.len());
            continue;
        }
        if byte == b'"' {
            in_string = true;
            out.push(byte);
            index += 1;
            continue;
        }
        if byte == b',' {
            let mut lookahead = index + 1;
            while lookahead < bytes.len() && bytes[lookahead].is_ascii_whitespace() {
                lookahead += 1;
            }
            if matches!(bytes.get(lookahead), Some(b'}' | b']')) {
                index += 1;
                continue;
            }
        }
        out.push(byte);
        index += 1;
    }
    String::from_utf8(out).map_err(|_| Error::new("E_HOST_INVARIANT", 9, "embedded bun lockfile is not valid UTF-8"))
}

// Indexes exact package resolution keys from the embedded bun.lock v2 JSONC document.
fn bun_package_keys(lock: &str) -> Result<Vec<String>, Error> {
    let invalid = || Error::new("E_HOST_INVARIANT", 9, "unsupported embedded bun lockfile representation");
    let stripped = strip_jsonc_trailing_commas(lock)?;
    let parsed: Value = serde_json::from_str(&stripped).map_err(|_| invalid())?;
    let object = parsed.as_object().ok_or_else(invalid)?;
    if object.get("lockfileVersion").and_then(Value::as_u64) != Some(2) {
        return Err(invalid());
    }
    let table = object.get("packages").and_then(Value::as_object).ok_or_else(invalid)?;
    let mut packages = Vec::with_capacity(table.len());
    for entry in table.values() {
        let resolved = entry.as_array()
            .and_then(|items| items.first())
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        if resolved.is_empty() || !resolved.is_ascii() || !resolved.contains('@') || resolved.contains(['\'', '"', '\\', ' ']) {
            return Err(invalid());
        }
        packages.push(resolved.to_owned());
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
    fn inventories_bun_lock_v2_packages_and_rejects_unknown_versions() {
        let lock = "{\n  // UTF-8 comment: 日本語\n  \"lockfileVersion\": 2,\n  \"packages\": {\n    \"@scope/tool\": [\"@scope/tool@1.2.3\", \"\", { \"note\": \"中文,}]\" }, \"sha512-x\"],\n    \"solid-js\": [\"solid-js@2.0.1\", \"\", {}, \"sha512-y\"],\n  },\n}\n";
        assert_eq!(bun_package_keys(lock).unwrap(), ["@scope/tool@1.2.3", "solid-js@2.0.1"]);
        assert!(bun_package_keys(&lock.replace("\"lockfileVersion\": 2", "\"lockfileVersion\": 1")).is_err());
    }
}
