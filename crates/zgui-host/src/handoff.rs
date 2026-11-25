//! Bounded in-memory JSON handoff channel (`<= 64 KiB`) preserved only across normal generation transitions.
use std::collections::BTreeSet;

use rquickjs::{Ctx, Exception, Function, Persistent, Value};

pub(crate) const MAX_HANDOFF_BYTES: usize = 64 * 1024;
const MAX_HANDOFF_DEPTH: usize = 32;

/// Validated plain-JSON snapshot passed in memory between settled generations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionHandoffSnapshot {
    pub(crate) json: String,
}

pub(crate) struct SessionHandoffStore {
    incoming: Option<String>,
    captures: Vec<(u64, Persistent<Function<'static>>)>,
    next_id: u64,
}

impl SessionHandoffStore {
    pub(crate) fn new(incoming: Option<SessionHandoffSnapshot>) -> Self {
        Self {
            incoming: incoming.map(|s| s.json),
            captures: Vec::new(),
            next_id: 1,
        }
    }

    pub(crate) fn register_capture<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        callback: Function<'js>,
    ) -> rquickjs::Result<u64> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| Exception::throw_range(ctx, "handoff registrations exhausted"))?;
        self.captures.push((id, Persistent::save(ctx, callback)));
        Ok(id)
    }

    pub(crate) fn unregister_capture(&mut self, id: u64) {
        self.captures.retain(|(entry_id, _)| *entry_id != id);
    }

    pub(crate) fn set_incoming(&mut self, snapshot: Option<SessionHandoffSnapshot>) {
        self.incoming = snapshot.map(|s| s.json);
    }

    pub(crate) fn take_incoming<'js>(&mut self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let Some(json) = self.incoming.take() else {
            return Ok(Value::new_null(ctx.clone()));
        };
        ctx.json_parse(json)
    }

    /// Invokes the latest registered handoff serializer on a normal generation handoff.
    /// Rejects non-JSON, circular structures, depth > 32, or UTF-8 size > 64 KiB.
    pub(crate) fn capture(&self, ctx: &Ctx<'_>) -> Option<SessionHandoffSnapshot> {
        let (_, callback) = self.captures.last()?;
        let func = callback.clone().restore(ctx).ok()?;
        let value: Value<'_> = match func.call(()) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("session-handoff-capture-error: {err}");
                return None;
            }
        };
        if value.is_null() || value.is_undefined() {
            return None;
        }
        let json_str = match ctx.json_stringify(value) {
            Ok(Some(js_str)) => match js_str.to_string() {
                Ok(s) => s,
                Err(err) => {
                    eprintln!("session-handoff-utf8-error: {err}");
                    return None;
                }
            },
            Ok(None) => return None,
            Err(err) => {
                eprintln!("session-handoff-stringify-error: {err}");
                return None;
            }
        };
        validate_handoff_json(json_str).map_or_else(
            |reason| {
                eprintln!("session-handoff-rejected: {reason}");
                None
            },
            Some,
        )
    }

    /// Releases every JS Persistent root before QuickJS drops.
    pub(crate) fn clear_roots(&mut self) {
        self.captures.clear();
        self.incoming = None;
    }

    pub(crate) fn clear_all(&mut self) {
        self.clear_roots();
    }
}

pub(crate) fn validate_handoff_json(json: String) -> Result<SessionHandoffSnapshot, &'static str> {
    if json.len() > MAX_HANDOFF_BYTES {
        return Err("session handoff exceeds 64 KiB limit");
    }
    let parsed: serde_json::Value =
        serde_json::from_str(&json).map_err(|_| "session handoff must be valid JSON")?;
    if !check_json_depth(&parsed, MAX_HANDOFF_DEPTH) {
        return Err("session handoff exceeds maximum JSON depth");
    }
    let _ = parsed;
    Ok(SessionHandoffSnapshot { json })
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

/// Validates whether an extension set is permitted under `rollback_allowed_extensions`.
pub(crate) fn rollback_permitted(
    extension_ids: &[String],
    rollback_allowed_extensions: &[String],
) -> bool {
    let allowed: BTreeSet<&str> = rollback_allowed_extensions.iter().map(String::as_str).collect();
    extension_ids.iter().all(|id| allowed.contains(id.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_64_kib_budget_and_depth_cap() {
        assert!(validate_handoff_json(r#"{"tab":"wifi","count":3}"#.into()).is_ok());

        let exact = format!("\"{}\"", "a".repeat(MAX_HANDOFF_BYTES - 2));
        assert_eq!(exact.len(), MAX_HANDOFF_BYTES);
        assert!(validate_handoff_json(exact).is_ok());

        let oversized = format!("\"{}\"", "a".repeat(MAX_HANDOFF_BYTES - 1));
        assert_eq!(oversized.len(), MAX_HANDOFF_BYTES + 1);
        assert!(validate_handoff_json(oversized).is_err());

        let too_deep = format!("{}{}", "[".repeat(MAX_HANDOFF_DEPTH + 2), "]".repeat(MAX_HANDOFF_DEPTH + 2));
        assert!(validate_handoff_json(too_deep).is_err());
    }
}
