use serde_json::{Value, json};
use super::{Error, scenario::{Selector, Target}};

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
fn small(value: &Value) -> Value {
    value.as_str().map_or(Value::Null, |text| json!(&text[..text.floor_char_boundary(text.len().min(64))]))
}
pub fn summary(node: &Value) -> Value {
    json!({"token":node["token"],"testId":small(&node["testId"]),"role":small(&node["role"]),"name":small(&node["name"])})
}
pub fn requested(target: &Target) -> Value {
    match &target.selector {
        Selector::TestId(id)=>json!({"surface":target.surface,"testId":short_string(id)}),
        Selector::Semantic{role,name}=>json!({"surface":target.surface,"role":short_string(role),"name":short_string(name)}),
    }
}
pub fn matches(node: &Value, selector: &Selector) -> bool {
    match selector {
        Selector::TestId(id)=>node["testId"].as_str()==Some(id.as_str()),
        Selector::Semantic{role,name}=>node["nameTruncated"]!=true && node["role"].as_str()==Some(role.as_str()) && node["name"].as_str()==Some(name.as_str()),
    }
}

/// Indices name this immutable snapshot only; never retain one across an action.
pub fn unique(tree: &Value, target: &Target, allow_missing: bool) -> Result<Option<usize>,Error> {
    if tree["surface"].as_str()!=Some(target.surface.as_str()) {
        return Err(Error::new("E_TARGET_STALE",4,"target and saved snapshot belong to different surfaces"));
    }
    let nodes=tree["nodes"].as_array().ok_or_else(||Error::new("E_OUTPUT_IO",8,"tree has no node array"))?;
    if tree["truncation"]["nodes"]==true {
        return Err(Error::new("E_TARGET_INCOMPLETE",5,"a truncated topology cannot establish unique target identity"));
    }
    if matches!(target.selector,Selector::TestId(_)) && tree["truncation"]["metadata"]==true {
        return Err(Error::new("E_TARGET_INCOMPLETE",5,"truncated test IDs cannot establish unique target identity"));
    }
    if matches!(target.selector,Selector::Semantic{..}) && (tree["semanticProjection"]["available"]!=true || nodes.iter().any(|node|node["nameTruncated"]==true)) {
        return Err(Error::new("E_TARGET_INCOMPLETE",5,"complete native semantic names are unavailable"));
    }
    let mut first=None;
    let mut count=0;
    for (index,node) in nodes.iter().enumerate() {
        if matches(node,&target.selector) { first.get_or_insert(index); count+=1; }
    }
    if count==1 || (count==0 && allow_missing) { return Ok(first); }
    let candidates:Vec<_>=nodes.iter().filter(|node| {
        if count>1 { matches(node,&target.selector) } else { !node["testId"].is_null() || node["role"]=="button" }
    }).take(3).map(summary).collect();
    Err(Error::new(if count==0 {"E_TARGET_MISSING"} else {"E_TARGET_AMBIGUOUS"},4,
        if count==0 {"no node matches the exact selector"} else {"selector matches multiple nodes"})
        .with_details(json!({"requested":requested(target),"matchCount":count,"candidates":candidates,"candidateLimit":3,
            "generation":tree["generation"],"frameOrdinal":tree["frameOrdinal"]})))
}
