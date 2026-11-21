//! Dev-only, bounded, versioned plain-data snapshot channel preserved across
//! staged generation reloads (`SessionHost::mount_module_inner` / `mount_shell`).
//!
//! Snapshots carry strictly validated plain JSON data — never closures,
//! promises, native handles, file/image capabilities, or service subscriptions.
//! Production and non-dev live sessions never install this channel.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::Rc,
};

use rquickjs::{
    Ctx, Exception, Object, Value,
    function::{Func, Opt},
    object::Property,
};
use sha2::{Digest, Sha256};

use crate::session::SessionModule;

pub(crate) const DEV_STATE_SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_DEV_STATE_KEYS: usize = 128;
pub(crate) const MAX_DEV_STATE_KEY_BYTES: usize = 128;
pub(crate) const MAX_DEV_STATE_VALUE_BYTES: usize = 16 * 1024;
pub(crate) const MAX_DEV_STATE_TOTAL_BYTES: usize = 128 * 1024;
const MAX_JSON_DEPTH: usize = 32;

/// Isolation key for a dev reload snapshot: app identity + manifest schema &
/// capability digest. Any change forces a clean cold mount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevStateScope {
    pub(crate) app_id: String,
    pub(crate) capability_digest: String,
}

impl DevStateScope {
    pub(crate) fn for_module(module: &SessionModule) -> Self {
        match module.as_package() {
            Some(package) => {
                let canonical = serde_json::to_vec(&(
                    DEV_STATE_SCHEMA_VERSION,
                    package.manifest.schema_version,
                    package.manifest.host_api_version,
                    &package.manifest.capabilities,
                ))
                .expect("manifest capabilities serialize cleanly");
                Self {
                    app_id: package.manifest.id.clone(),
                    capability_digest: format!("sha256:{:x}", Sha256::digest(&canonical)),
                }
            }
            None => Self {
                app_id: module.name().to_owned(),
                capability_digest: format!("inline:v{DEV_STATE_SCHEMA_VERSION}"),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevStateEntry {
    pub(crate) version: i64,
    pub(crate) json: String,
}

/// Pure plain-data snapshot captured from a settled generation and handed to
/// its staged successor candidate.
#[derive(Clone, Debug)]
pub(crate) struct DevStateSnapshot {
    pub(crate) schema_version: u32,
    pub(crate) scope: DevStateScope,
    pub(crate) entries: BTreeMap<String, DevStateEntry>,
}

/// Session-local bounded plain-data store installed on `globalThis.__zguiDevState`
/// strictly in dev mode (`SessionPolicy::dev_state == true`).
pub(crate) struct DevStateStore {
    hydrated: BTreeMap<String, DevStateEntry>,
    current: BTreeMap<String, DevStateEntry>,
    total_bytes: usize,
    active: bool,
}

impl DevStateStore {
    pub(crate) fn new(hydrated: Option<DevStateSnapshot>, expected_scope: &DevStateScope) -> Self {
        let hydrated_map = match hydrated {
            Some(snapshot)
                if snapshot.schema_version == DEV_STATE_SCHEMA_VERSION
                    && snapshot.scope == *expected_scope =>
            {
                snapshot.entries
            }
            _ => BTreeMap::new(),
        };
        Self {
            hydrated: hydrated_map,
            current: BTreeMap::new(),
            total_bytes: 0,
            active: true,
        }
    }

    pub(crate) fn snapshot(&self, scope: DevStateScope) -> Option<DevStateSnapshot> {
        if !self.active || self.current.is_empty() {
            return None;
        }
        Some(DevStateSnapshot {
            schema_version: DEV_STATE_SCHEMA_VERSION,
            scope,
            entries: self.current.clone(),
        })
    }

    pub(crate) fn clear(&mut self) {
        self.active = false;
        self.hydrated.clear();
        self.current.clear();
        self.total_bytes = 0;
    }

    fn read(&self, key: &str, expected_version: Option<i64>) -> Option<String> {
        if !self.active {
            return None;
        }
        let entry = self.hydrated.get(key)?;
        if let Some(expected) = expected_version
            && entry.version != expected
        {
            return None;
        }
        Some(entry.json.clone())
    }

    fn write(&mut self, key: String, json: String, version: i64) -> Result<(), &'static str> {
        if !self.active {
            return Ok(());
        }
        if key.is_empty() || key.len() > MAX_DEV_STATE_KEY_BYTES {
            return Err("dev state key length out of bounds");
        }
        if json.len() > MAX_DEV_STATE_VALUE_BYTES {
            return Err("dev state value exceeds maximum byte budget");
        }
        let parsed: serde_json::Value =
            serde_json::from_str(&json).map_err(|_| "dev state value must be valid plain JSON")?;
        if !check_json_depth(&parsed, MAX_JSON_DEPTH) {
            return Err("dev state JSON nesting exceeds maximum depth");
        }
        let entry_bytes = key.len() + json.len();
        let previous_bytes = self
            .current
            .get(&key)
            .map_or(0, |existing| key.len() + existing.json.len());
        if !self.current.contains_key(&key) && self.current.len() >= MAX_DEV_STATE_KEYS {
            return Err("dev state key count exceeds maximum budget");
        }
        let next_total = self.total_bytes - previous_bytes + entry_bytes;
        if next_total > MAX_DEV_STATE_TOTAL_BYTES {
            return Err("dev state total byte budget exceeded");
        }
        self.total_bytes = next_total;
        self.current.insert(key, DevStateEntry { version, json });
        Ok(())
    }

    fn remove(&mut self, key: &str) {
        if let Some(removed) = self.current.remove(key) {
            self.total_bytes = self.total_bytes.saturating_sub(key.len() + removed.json.len());
        }
    }

    /// Install the frozen host-owned `__zguiDevState` object into QuickJS.
    pub(crate) fn install(store: &Rc<RefCell<Self>>, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        let obj = Object::new(ctx.clone())?;

        let read_store = Rc::downgrade(store);
        obj.prop(
            "read",
            Property::from(Func::from(
                move |ctx: Ctx<'_>, key: String, expected_version: Opt<Value<'_>>| {
                    if key.is_empty() || key.len() > MAX_DEV_STATE_KEY_BYTES {
                        return Err(Exception::throw_range(
                            &ctx,
                            "dev state key must be a non-empty string within 128 bytes",
                        ));
                    }
                    let version = match expected_version.0 {
                        None => None,
                        Some(v) if v.is_undefined() || v.is_null() => None,
                        Some(v) => Some(parse_version(&ctx, v)?),
                    };
                    let Some(store) = read_store.upgrade() else {
                        return Ok(None);
                    };
                    Ok(store.borrow().read(&key, version))
                },
            ))
            .enumerable(),
        )?;

        let write_store = Rc::downgrade(store);
        obj.prop(
            "write",
            Property::from(Func::from(
                move |ctx: Ctx<'_>, key: String, json: String, version: Opt<Value<'_>>| {
                    let version = match version.0 {
                        None => 1,
                        Some(v) if v.is_undefined() || v.is_null() => 1,
                        Some(v) => parse_version(&ctx, v)?,
                    };
                    let Some(store) = write_store.upgrade() else {
                        return Ok(());
                    };
                    store
                        .borrow_mut()
                        .write(key, json, version)
                        .map_err(|msg| Exception::throw_range(&ctx, msg))
                },
            ))
            .enumerable(),
        )?;

        let remove_store = Rc::downgrade(store);
        obj.prop(
            "remove",
            Property::from(Func::from(move |_ctx: Ctx<'_>, key: String| {
                if let Some(store) = remove_store.upgrade() {
                    store.borrow_mut().remove(&key);
                }
            }))
            .enumerable(),
        )?;

        // Freeze to prevent app code from mutating the host channel.
        if let Ok(object_ctor) = ctx.globals().get::<_, Object<'_>>("Object")
            && let Ok(freeze) = object_ctor.get::<_, rquickjs::Function<'_>>("freeze")
        {
            let _: Value<'_> = freeze.call((obj.clone(),))?;
        }
        ctx.globals().set("__zguiDevState", obj)?;
        Ok(())
    }
}

fn parse_version(ctx: &Ctx<'_>, value: Value<'_>) -> rquickjs::Result<i64> {
    let Some(num) = value.as_number() else {
        return Err(Exception::throw_type(
            ctx,
            "dev state version must be a safe integer",
        ));
    };
    if !num.is_finite() || num.fract() != 0.0 || num.abs() > 9_007_199_254_740_991.0 {
        return Err(Exception::throw_range(
            ctx,
            "dev state version must be a safe integer",
        ));
    }
    Ok(num as i64)
}

fn check_json_depth(value: &serde_json::Value, remaining: usize) -> bool {
    if remaining == 0 {
        return false;
    }
    match value {
        serde_json::Value::Array(items) => items.iter().all(|v| check_json_depth(v, remaining - 1)),
        serde_json::Value::Object(map) => map.values().all(|v| check_json_depth(v, remaining - 1)),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(id: &str, cap: &str) -> DevStateScope {
        DevStateScope {
            app_id: id.to_owned(),
            capability_digest: cap.to_owned(),
        }
    }

    #[test]
    fn snapshot_hydrates_only_matching_scope_and_version() {
        let s1 = scope("app-a", "sha256:aaa");
        let s2 = scope("app-a", "sha256:bbb");
        let mut store = DevStateStore::new(None, &s1);
        store.write("count".into(), "42".into(), 2).unwrap();
        let snap = store.snapshot(s1.clone()).expect("non-empty snapshot");

        // Matching scope hydrates matching version, ignores mismatched version.
        let matched = DevStateStore::new(Some(snap.clone()), &s1);
        assert_eq!(matched.read("count", Some(2)).as_deref(), Some("42"));
        assert_eq!(matched.read("count", Some(3)), None);

        // Mismatched capability digest forces clean cold start.
        let cold = DevStateStore::new(Some(snap), &s2);
        assert_eq!(cold.read("count", Some(2)), None);
    }

    #[test]
    fn store_enforces_size_key_count_and_depth_bounds() {
        let s = scope("app-a", "sha256:aaa");
        let mut store = DevStateStore::new(None, &s);
        assert!(store.write(String::new(), "1".into(), 1).is_err());
        assert!(store.write("k".repeat(MAX_DEV_STATE_KEY_BYTES + 1), "1".into(), 1).is_err());
        assert!(store.write("bad".into(), "{not json".into(), 1).is_err());
        assert!(store.write("big".into(), format!("\"{}\"", "a".repeat(MAX_DEV_STATE_VALUE_BYTES)), 1).is_err());

        // Deeply nested JSON rejected.
        let deep = format!("{}{}", "[".repeat(40), "]".repeat(40));
        assert!(store.write("deep".into(), deep, 1).is_err());

        // Key count capped at MAX_DEV_STATE_KEYS.
        for i in 0..MAX_DEV_STATE_KEYS {
            store.write(format!("k{i}"), "1".into(), 1).unwrap();
        }
        assert!(store.write("overflow".into(), "1".into(), 1).is_err());

        // Clear frees everything cleanly.
        store.clear();
        assert!(store.snapshot(s).is_none());
    }
}
