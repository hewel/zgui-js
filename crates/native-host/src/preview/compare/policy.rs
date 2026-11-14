//! Closed, bounded v1 policy input. Thresholds apply globally to every unmasked RGBA pixel.
use std::collections::BTreeSet;

use serde::Deserialize;

use crate::preview::{Error, schema::object_struct};

pub(super) const MAX_EDGE: u32 = 16_384;
pub(super) const MAX_PIXELS: u64 = MAX_EDGE as u64 * MAX_EDGE as u64;
const MAX_POLICY_BYTES: usize = 64 * 1024;
const MAX_MASKS: usize = 256;
const MAX_CAPTURES: usize = 4096;

object_struct! {
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct CaptureKey {
        pub capture_id: String,
        pub surface: String,
    }
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub(super) enum CaptureSet {
    All,
    Subset { captures: Vec<CaptureKey>, reason: String },
}

#[derive(Debug)]
pub(super) struct Policy {
    pub capture_set: CaptureSet,
    pub max_pixels: u64,
    pub max_output_bytes: u64,
    pub(super) scoring: Scoring,
    pub(super) masks: Vec<Mask>,
}

#[derive(Debug)]
pub(super) enum Scoring {
    Exact,
    Tolerant { thresholds: Thresholds, review_reason: String },
}

object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct PolicyInput {
        schema_version: u32,
        mode: String,
        alpha: String,
        capture_set: CaptureSetInput,
        masks: Vec<Mask>,
        #[serde(default, deserialize_with = "present")]
        thresholds: Option<Thresholds>,
        #[serde(default, deserialize_with = "present")]
        review_reason: Option<String>,
        #[serde(default = "default_limits")]
        limits: Limits,
    }
}

object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct CaptureSetInput {
        mode: String,
        #[serde(default, deserialize_with = "present")]
        captures: Option<Vec<CaptureKey>>,
        #[serde(default, deserialize_with = "present")]
        reason: Option<String>,
    }
}

object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct Thresholds {
        pub channel: u8,
        pub max_changed_pixels: u64,
        pub max_changed_ratio: f64,
    }
}

object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct Mask {
        pub capture_id: String,
        pub surface: String,
        pub rect: Rect,
        pub reason: String,
    }
}

object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct Rect {
        pub x: u32,
        pub y: u32,
        pub width: u32,
        pub height: u32,
    }
}

object_struct! {
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Limits {
        max_pixels: u64,
        max_output_bytes: u64,
    }
}

fn default_limits() -> Limits {
    Limits { max_pixels: 67_108_864, max_output_bytes: 268_435_456 }
}

// Omitted optional fields are allowed; explicit null is never a policy declaration.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(decoder: D) -> Result<Option<T>, D::Error> {
    T::deserialize(decoder).map(Some)
}

impl Policy {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_POLICY_BYTES {
            return Err(invalid("comparison policy exceeds 64 KiB"));
        }
        let input: PolicyInput = serde_json::from_slice(bytes)
            .map_err(|error| invalid(format!("invalid comparison policy: {error}")))?;
        if input.schema_version != 1 || input.alpha != "include" {
            return Err(invalid("comparison requires schemaVersion 1 and alpha include"));
        }
        if !(1..=MAX_PIXELS).contains(&input.limits.max_pixels)
            || !(1..=1_073_741_824).contains(&input.limits.max_output_bytes) {
            return Err(invalid("limits.maxPixels must be 1..=268435456 and maxOutputBytes 1..=1073741824"));
        }
        let scoring = match (input.mode.as_str(), input.thresholds, input.review_reason) {
            ("exact", None, None) => Scoring::Exact,
            ("tolerant", Some(thresholds), Some(review_reason)) => {
                validate_reason(&review_reason)?;
                if thresholds.max_changed_pixels > MAX_PIXELS
                    || !thresholds.max_changed_ratio.is_finite()
                    || !(0.0..=1.0).contains(&thresholds.max_changed_ratio) {
                    return Err(invalid("threshold budgets require maxChangedPixels <=268435456 and finite maxChangedRatio in 0..=1"));
                }
                Scoring::Tolerant { thresholds, review_reason }
            }
            _ => return Err(invalid("exact forbids thresholds/reviewReason; tolerant requires both explicit thresholds and reviewReason")),
        };
        let capture_set = match (input.capture_set.mode.as_str(), input.capture_set.captures, input.capture_set.reason) {
            ("all", None, None) => CaptureSet::All,
            ("subset", Some(captures), Some(reason)) => {
                validate_reason(&reason)?;
                if captures.is_empty() || captures.len() > MAX_CAPTURES {
                    return Err(invalid("capture subset must contain 1..=4096 unique keys"));
                }
                let mut seen = BTreeSet::new();
                for key in &captures {
                    validate_key(key)?;
                    if !seen.insert(key) { return Err(invalid("duplicate capture subset key")); }
                }
                CaptureSet::Subset { captures, reason }
            }
            _ => return Err(invalid("captureSet must be all without other fields, or subset with captures and reason")),
        };
        if input.masks.len() > MAX_MASKS { return Err(invalid("at most 256 masks are allowed")); }
        let mut masks = BTreeSet::new();
        for mask in &input.masks {
            if !safe_id(&mask.capture_id) || !safe_id(&mask.surface) {
                return Err(invalid("mask captureId and surface must be valid IDs"));
            }
            validate_reason(&mask.reason)?;
            let rect = &mask.rect;
            if rect.width == 0 || rect.height == 0
                || rect.x.checked_add(rect.width).is_none_or(|end| end > MAX_EDGE)
                || rect.y.checked_add(rect.height).is_none_or(|end| end > MAX_EDGE) {
                return Err(invalid("mask rectangles must be nonempty and within the 16384-pixel native extent"));
            }
            if !masks.insert((&mask.capture_id, &mask.surface, rect.x, rect.y, rect.width, rect.height)) {
                return Err(invalid("duplicate mask selector and rectangle"));
            }
        }
        let policy = Self {
            capture_set, scoring, masks: input.masks,
            max_pixels: input.limits.max_pixels, max_output_bytes: input.limits.max_output_bytes,
        };
        if let CaptureSet::Subset { captures, .. } = &policy.capture_set {
            policy.validate_targets(&captures.iter().cloned().collect())?;
        }
        Ok(policy)
    }

    pub fn validate_targets(&self, selected: &BTreeSet<CaptureKey>) -> Result<(), Error> {
        if selected.is_empty() { return Err(invalid("comparison requires a nonempty capture set")); }
        for key in selected { validate_key(key)?; }
        if let CaptureSet::Subset { captures, .. } = &self.capture_set
            && (captures.len() != selected.len() || captures.iter().any(|key| !selected.contains(key))) {
            return Err(invalid("selected captures do not match the explicit policy subset"));
        }
        for mask in &self.masks {
            if !selected.iter().any(|key| key.capture_id == mask.capture_id && key.surface == mask.surface) {
                return Err(invalid(format!("mask targets an unknown or unselected capture: {}/{}", mask.capture_id, mask.surface)));
            }
        }
        Ok(())
    }
}

fn safe_id(value: &str) -> bool { crate::preview::scenario::safe_id(value) }

fn validate_key(key: &CaptureKey) -> Result<(), Error> {
    if !safe_id(&key.capture_id) || !safe_id(&key.surface) {
        return Err(invalid("captureId and surface must match [a-z0-9][a-z0-9._-]* and contain at most 128 bytes"));
    }
    Ok(())
}

fn validate_reason(reason: &str) -> Result<(), Error> {
    if reason.trim().is_empty() || reason.chars().count() > 1024 {
        return Err(invalid("review/mask/subset reason must contain 1..=1024 characters of nonblank text"));
    }
    Ok(())
}

pub(super) fn invalid(message: impl Into<String>) -> Error { Error::new("E_SCHEMA", 2, message) }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn input() -> Value {
        json!({"schemaVersion":1,"mode":"exact","alpha":"include","captureSet":{"mode":"all"},"masks":[]})
    }
    fn parse(value: &Value) -> Result<Policy, Error> { Policy::parse(&serde_json::to_vec(value).unwrap()) }
    fn key(id: &str) -> CaptureKey { CaptureKey { capture_id:id.into(), surface:"main".into() } }

    #[test]
    fn closed_objects_reject_arrays_duplicates_unknown_fields_and_null() {
        for bytes in [
            r#"{"schemaVersion":1,"mode":"exact","mode":"exact","alpha":"include","captureSet":{"mode":"all"},"masks":[]}"#,
            r#"{"schemaVersion":1,"mode":"exact","alpha":"include","captureSet":{"mode":"all","mode":"all"},"masks":[]}"#,
            r#"{"schemaVersion":1,"mode":"exact","alpha":"include","captureSet":{"mode":"all"},"masks":[{"captureId":"shot","surface":"main","rect":{"x":0,"x":0,"y":0,"width":1,"height":1},"reason":"clock"}]}"#,
            r#"[1,"exact","include",{"mode":"all"},[]]"#,
        ] { assert_eq!(Policy::parse(bytes.as_bytes()).unwrap_err().code, "E_SCHEMA"); }
        for (field, value) in [
            ("captureSet", json!(["all"])), ("captureSet", json!({"mode":"all","extra":true})),
            ("thresholds", Value::Null), ("reviewReason", Value::Null), ("limits", json!([1,1])),
            ("limits", json!({"maxPixels":1,"maxOutputBytes":1,"extra":1})), ("surprise", json!(true)),
            ("masks", json!([["shot","main",{"x":0,"y":0,"width":1,"height":1},"reviewed"]])),
            ("masks", json!([{"captureId":"shot","surface":"main","rect":[0,0,1,1],"reason":"reviewed"}])),
        ] {
            let mut declaration = input(); declaration[field] = value;
            assert!(parse(&declaration).is_err(), "accepted {declaration}");
        }
    }

    #[test]
    fn nested_tolerant_and_subset_objects_are_closed_and_reasons_are_required() {
        let tolerant = json!({
            "schemaVersion":1,"mode":"tolerant","alpha":"include",
            "captureSet":{"mode":"subset","captures":[{"captureId":"shot","surface":"main"}],"reason":"selected scene"},
            "masks":[],"thresholds":{"channel":2,"maxChangedPixels":1,"maxChangedRatio":0.1},
            "reviewReason":"reviewed rasterizer variation"
        });
        for bad in [json!([2,1,0.1]), json!({"channel":2,"maxChangedPixels":1,"maxChangedRatio":0.1,"alpha":false})] {
            let mut value = tolerant.clone(); value["thresholds"] = bad; assert!(parse(&value).is_err());
        }
        for bad in [json!([["shot","main"]]), json!([{"captureId":"shot","surface":"main","extra":1}])] {
            let mut value = tolerant.clone(); value["captureSet"]["captures"] = bad; assert!(parse(&value).is_err());
        }
        for field in ["reviewReason", "alpha"] {
            let mut value = tolerant.clone(); value[field] = json!("  "); assert!(parse(&value).is_err());
            let mut value = tolerant.clone(); value.as_object_mut().unwrap().remove(field); assert!(parse(&value).is_err());
        }
        let mut value = tolerant; value["captureSet"]["reason"] = json!(" \n"); assert!(parse(&value).is_err());
    }

    #[test]
    fn subset_and_mask_targets_are_explicit_and_unique() {
        let mut value = input();
        value["captureSet"] = json!({"mode":"subset","captures":[{"captureId":"shot","surface":"main"}],"reason":"review only counter"});
        let policy = parse(&value).unwrap();
        policy.validate_targets(&BTreeSet::from([key("shot")])).unwrap();
        assert!(policy.validate_targets(&BTreeSet::from([key("other")])).is_err());
        value["captureSet"]["captures"] = json!([{"captureId":"shot","surface":"main"},{"captureId":"shot","surface":"main"}]);
        assert!(parse(&value).is_err());
        value["captureSet"]["captures"] = json!([]);
        assert!(parse(&value).is_err());
        value["captureSet"]["captures"] = json!([{"captureId":"../escape","surface":"main"}]);
        assert!(parse(&value).is_err());
        value = input();
        value["masks"] = json!([{"captureId":"other","surface":"main","rect":{"x":0,"y":0,"width":1,"height":1},"reason":"dynamic clock"}]);
        assert!(parse(&value).unwrap().validate_targets(&BTreeSet::from([key("shot")])).is_err());
        value["captureSet"] = json!({"mode":"subset","captures":[{"captureId":"shot","surface":"main"}],"reason":"reviewed subset"});
        assert!(parse(&value).is_err());
    }

    #[test]
    fn tolerant_requires_review_and_finite_budgets_and_limits_are_bounded() {
        let mut value = input(); value["mode"] = json!("tolerant");
        assert!(parse(&value).is_err());
        value["thresholds"] = json!({"channel":2,"maxChangedPixels":1,"maxChangedRatio":0.1});
        value["reviewReason"] = json!("reviewed antialiasing variation");
        parse(&value).unwrap();
        for bad in [json!(-0.1), json!(1.1), Value::Null] {
            value["thresholds"]["maxChangedRatio"] = bad; assert!(parse(&value).is_err());
        }
        value = input(); value["thresholds"] = json!({"channel":0,"maxChangedPixels":0,"maxChangedRatio":0});
        assert!(parse(&value).is_err());
        for limit in [0, MAX_PIXELS + 1] {
            value = input(); value["limits"] = json!({"maxPixels":limit,"maxOutputBytes":1});
            assert!(parse(&value).is_err());
        }
        assert!(Policy::parse(&vec![b' '; MAX_POLICY_BYTES + 1]).is_err());
    }

    #[test]
    fn masks_refuse_degenerate_overflowing_duplicate_and_excessive_geometry() {
        let mask = json!({"captureId":"shot","surface":"main","rect":{"x":0,"y":0,"width":1,"height":1},"reason":"clock"});
        for rect in [json!({"x":0,"y":0,"width":0,"height":1}), json!({"x":4294967295_u32,"y":0,"width":2,"height":1})] {
            let mut value = input(); let mut invalid_mask = mask.clone(); invalid_mask["rect"] = rect;
            value["masks"] = json!([invalid_mask]); assert!(parse(&value).is_err());
        }
        let mut value = input(); value["masks"] = json!([mask.clone(),mask.clone()]); assert!(parse(&value).is_err());
        value["masks"] = json!(vec![mask; MAX_MASKS + 1]); assert!(parse(&value).is_err());
    }
}
