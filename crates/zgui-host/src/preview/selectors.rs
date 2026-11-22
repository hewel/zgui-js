use serde_json::{Value, json};
use super::{Error,evidence::{NodeEvidence,TreeEvidence},scenario::{Selector, Target}};

pub fn short(value: &Value) -> Value {
    match value.as_str() {
        Some(text) => short_string(text),
        _ => value.clone(),
    }
}
pub fn short_string(text: &str) -> Value {
    if text.len()>128 { json!({"prefix":&text[..text.floor_char_boundary(128)],"bytes":text.len(),"truncated":true}) }
    else { json!(text) }
}
fn small(value: Option<&str>) -> Value {
    value.map_or(Value::Null, |text| json!(&text[..text.floor_char_boundary(text.len().min(64))]))
}
pub fn summary(node: &NodeEvidence) -> Value {
    json!({"token":node.token,"testId":small(node.test_id.as_deref()),"role":small(node.role.as_deref()),"name":small(node.name.as_deref())})
}
pub fn requested(target: &Target) -> Value {
    match &target.selector {
        Selector::TestId(id)=>json!({"surface":target.surface,"testId":short_string(id)}),
        Selector::Semantic{role,name}=>json!({"surface":target.surface,"role":short_string(role),"name":short_string(name)}),
    }
}
pub fn matches(node: &NodeEvidence, selector: &Selector) -> bool {
    match selector {
        Selector::TestId(id)=>node.test_id.as_deref()==Some(id.as_str()),
        Selector::Semantic{role,name}=>!node.name_truncated && node.role.as_deref()==Some(role.as_str()) && node.name.as_deref()==Some(name.as_str()),
    }
}

/// Indices name this immutable snapshot only; never retain one across an action.
pub fn unique(tree: &TreeEvidence, target: &Target, allow_missing: bool) -> Result<Option<usize>,Error> {
    if tree.surface!=target.surface {
        return Err(Error::new("E_TARGET_STALE",4,"target and saved snapshot belong to different surfaces"));
    }
    if tree.detail("truncation")["nodes"]==true {
        return Err(Error::new("E_TARGET_INCOMPLETE",5,"a truncated topology cannot establish unique target identity"));
    }
    if matches!(target.selector,Selector::TestId(_)) && tree.detail("truncation")["metadata"]==true {
        return Err(Error::new("E_TARGET_INCOMPLETE",5,"truncated test IDs cannot establish unique target identity"));
    }
    if matches!(target.selector,Selector::Semantic{..}) && (tree.detail("semanticProjection")["available"]!=true || tree.nodes.iter().any(|node|node.name_truncated)) {
        return Err(Error::new("E_TARGET_INCOMPLETE",5,"complete native semantic names are unavailable"));
    }
    let mut first=None;
    let mut count=0;
    for (index,node) in tree.nodes.iter().enumerate() {
        if matches(node,&target.selector) { first.get_or_insert(index); count+=1; }
    }
    if count==1 || (count==0 && allow_missing) { return Ok(first); }
    let candidates:Vec<_>=tree.nodes.iter().filter(|node| {
        if count>1 { matches(node,&target.selector) } else { node.test_id.is_some() || node.role.as_deref()==Some("button") }
    }).take(3).map(summary).collect();
    Err(Error::new(if count==0 {"E_TARGET_MISSING"} else {"E_TARGET_AMBIGUOUS"},4,
        if count==0 {"no node matches the exact selector"} else {"selector matches multiple nodes"})
        .with_details(json!({"requested":requested(target),"matchCount":count,"candidates":candidates,"candidateLimit":3,
            "generation":tree.generation,"frameOrdinal":tree.frame_ordinal})))
}
