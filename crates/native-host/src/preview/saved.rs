//! Bounded immutable run artifacts and saved-tree queries; never constructs a runtime or renderer.
use std::{collections::{HashMap,HashSet},io::Read,path::{Path,PathBuf}};
use serde_json::{Value,json};
use sha2::{Digest, Sha256};
use super::{Error,output::digest,scenario::{Selector,Target},selectors};

const FIELDS:&[&str]=&["token","parent","surface","generation","kind","ownership","testId","role","name","text","value","state","boundsLogical","boundsDevice","boundsKind","source","textTruncated","testIdTruncated","nameTruncated","valueTruncated","pointerReachabilityVerified"];

struct Options {
    root:PathBuf,capture:String,target:Option<Target>,fields:Vec<String>,max_nodes:usize,max_depth:usize,cursor:Option<String>,
}
fn options(args:Vec<String>)->Result<Options,Error> {
    let mut args=args.into_iter();
    let mut seen=HashSet::new();
    let(mut root,mut capture,mut test_id,mut role,mut name,mut cursor)=(None,None,None,None,None,None);
    let mut fields="token,parent,testId,role,name,text,state,boundsLogical".to_owned();
    let(mut max_nodes,mut max_depth)=(10,1);
    while let Some(key)=args.next() {
        if !seen.insert(key.clone()) { return Err(schema("duplicate inspect option")); }
        let value=args.next().ok_or_else(||schema("inspect option has no value"))?;
        match key.as_str() {
            "--run"=>root=Some(PathBuf::from(value)),"--capture"=>capture=Some(value),
            "--test-id"=>test_id=Some(value),"--role"=>role=Some(value),"--name"=>name=Some(value),
            "--fields"=>fields=value,"--cursor"=>cursor=Some(value),
            "--max-nodes"=>max_nodes=value.parse::<usize>().map_err(|_|schema("max-nodes must be an integer"))?,
            "--max-depth"=>max_depth=value.parse::<usize>().map_err(|_|schema("max-depth must be an integer"))?,
            _=>return Err(schema("unsupported inspect option")),
        }
    }
    let capture=capture.ok_or_else(||schema("--capture is required to select an immutable version"))?;
    if !super::scenario::safe_id(&capture) { return Err(schema("invalid capture ID")); }
    if max_nodes==0 || max_nodes>100 || max_depth>64 { return Err(schema("max-nodes must be 1..100 and max-depth 0..64")); }
    let fields:Vec<String>=fields.split(',').map(str::to_owned).collect();
    if fields.is_empty() || fields.iter().any(|field|!FIELDS.contains(&field.as_str())) || fields.iter().collect::<HashSet<_>>().len()!=fields.len() {
        return Err(schema("fields must be a nonempty, duplicate-free inspection allowlist"));
    }
    let target=match(test_id,role,name) {
        (None,None,None)=>None,
        (Some(id),None,None) if id.len()<=512=>Some(Target{surface:"main".into(),selector:Selector::TestId(id)}),
        (None,Some(role),Some(name)) if role.len()<=128 && name.len()<=4096=>Some(Target{surface:"main".into(),selector:Selector::Semantic{role,name}}),
        _=>return Err(schema("use test-id or role plus name, never both; selector strings must fit native metadata limits")),
    };
    Ok(Options{root:root.ok_or_else(||schema("--run is required"))?,capture,target,fields,max_nodes,max_depth,cursor})
}

pub fn command(args:Vec<String>)->Result<Value,Error> {
    let options=options(args)?;
    let run=SavedRun::open(&options.root)?;
    let capture=run.capture(&options.capture)?;
    query(&capture.tree,&capture.tree_digest,&run.root.join(format!("captures/{}.tree.json",options.capture)),
        run.result["status"].as_str().expect("validated terminal status"),&options)
}

fn validate_identity(tree:&Value,frame:&Value,result:&Value,capture:&str,tree_digest:&str)->Result<(),Error> {
    if tree["schemaVersion"]!=1 || frame["schemaVersion"]!=1 || result["schemaVersion"]!=1 || result["command"]!="run"
        || frame["captureId"]!=capture || frame["treeDigest"]!=tree_digest || frame["layoutDocumentCoherent"]!=true
        || tree["buildId"].as_str().is_none_or(str::is_empty) || tree["buildId"]!=frame["buildId"] || tree["buildId"]!=result["build"]["buildId"]
        || tree["generation"].as_u64().is_none_or(|generation|generation==0 || result["generation"].as_u64().is_none_or(|current|generation>current))
        || tree["generation"]!=frame["generation"]
        || tree["surface"]!="main" || tree["surface"]!=frame["surface"]
        || tree["frameOrdinal"].as_u64().is_none() || tree["frameOrdinal"]!=frame["validationFrame"]["ordinal"]
        || tree["presentationSequence"].as_u64().is_none() || tree["presentationSequence"]!=frame["pixelPresentation"]["sequence"]
        || tree["metadataRevision"].as_u64().is_none() || tree["metadataRevision"]!=frame["metadataRevision"] {
        return Err(Error::new("E_OUTPUT_IO",8,"capture, tree and terminal run identities disagree"));
    }
    let native_resources=frame["resources"].get("requiredAssetsReady").is_some();
    for field in ["resourceRevision","surfaceConfiguration"] {
        if native_resources || tree.get(field).is_some() || frame.get(field).is_some() {
            if tree[field].as_u64().is_none() || tree[field]!=frame[field] {
                return Err(Error::new("E_OUTPUT_IO",8,"capture resource/surface configuration identities disagree"));
            }
        }
    }
    if frame.get("resourceRevision").is_some() && frame["resourceRevision"]!=frame["resources"]["resourceRevision"] {
        return Err(Error::new("E_OUTPUT_IO",8,"capture readiness belongs to another resource revision"));
    }
    Ok(())
}

fn query(tree:&Value,tree_digest:&str,path:&Path,source_status:&str,options:&Options)->Result<Value,Error> {
    let nodes=tree["nodes"].as_array().ok_or_else(||Error::new("E_OUTPUT_IO",8,"saved tree has no node array"))?;
    if nodes.is_empty() || nodes.len()>4096 { return Err(Error::new("E_OUTPUT_LIMIT",8,"saved tree exceeds the bounded native snapshot contract")); }
    let selected=if let Some(target)=&options.target { selectors::unique(tree,target,false)?.expect("required target") } else { 0 };
    let mut by_token=HashMap::with_capacity(nodes.len());
    let mut depths=Vec::with_capacity(nodes.len());
    let mut parents=Vec::with_capacity(nodes.len());
    for (index,node) in nodes.iter().enumerate() {
        let token=node["token"].as_str().ok_or_else(||Error::new("E_OUTPUT_IO",8,"saved node has no token"))?;
        if token.len()>64 || by_token.contains_key(token) { return Err(Error::new("E_OUTPUT_IO",8,"saved tree has invalid or duplicate tokens")); }
        let parent=match node["parent"].as_str() {
            Some(parent)=>Some(*by_token.get(parent).ok_or_else(||Error::new("E_OUTPUT_IO",8,"saved tree is not parent-first"))?),
            None if node["parent"].is_null()=>None,
            _=>return Err(Error::new("E_OUTPUT_IO",8,"saved node parent is invalid")),
        };
        depths.push(parent.map_or(0_usize,|parent:usize|depths[parent]+1));
        parents.push(parent);
        by_token.insert(token,index);
    }
    let mut selected_nodes=vec![selected];
    if let Some(parent)=parents[selected] { selected_nodes.push(parent); }
    let mut depth_omitted=0;
    for index in selected+1..nodes.len() {
        if depths[index]<=depths[selected] { break; }
        if depths[index]-depths[selected]<=options.max_depth { selected_nodes.push(index); } else { depth_omitted+=1; }
    }
    let query_digest=digest(&serde_json::to_vec(&json!({"capture":options.capture,"target":options.target,"fields":options.fields,"maxDepth":options.max_depth})).expect("query serializes"));
    let prefix=format!("{}.{}.",tree_digest.trim_start_matches("sha256:"),query_digest.trim_start_matches("sha256:"));
    let offset=match &options.cursor {
        None=>0,
        Some(cursor)=>cursor.strip_prefix(&prefix).and_then(|offset|offset.parse::<usize>().ok()).filter(|offset|*offset<selected_nodes.len())
            .ok_or_else(||schema("cursor does not belong to this immutable snapshot and query"))?,
    };
    let mut result=json!({"schemaVersion":1,"command":"inspect","status":"passed","exitCode":0,
        "sourceRunStatus":source_status,"artifact":path,"treeDigest":tree_digest,"buildId":tree["buildId"],"generation":tree["generation"],
        "frameOrdinal":tree["frameOrdinal"],"presentationSequence":tree["presentationSequence"],"selectedToken":nodes[selected]["token"],
        "nodes":[],"nextCursor":null,"truncation":{"pageRemaining":0,"depthOmitted":depth_omitted,
            "sourceNodes":tree["truncation"]["nodes"].as_bool(),"sourceText":tree["truncation"]["text"].as_bool(),"fieldBytes":128}});
    let base=serde_json::to_vec(&result).expect("inspection serializes").len();
    let mut used=base+prefix.len()+24; // Reserve the longest cursor, including its offset.
    let mut projected=Vec::new();
    for index in selected_nodes.iter().skip(offset).take(options.max_nodes) {
        let node=project(&nodes[*index],&options.fields);
        let bytes=serde_json::to_vec(&node).expect("node serializes").len()+1;
        if used+bytes+1>super::MAX_STDOUT_BYTES { break; }
        used+=bytes;
        projected.push(node);
    }
    if projected.is_empty() { return Err(Error::new("E_OUTPUT_LIMIT",8,"selected inspection row cannot fit the 8 KiB response limit")); }
    let next=offset+projected.len();
    result["nodes"]=json!(projected);
    result["truncation"]["pageRemaining"]=json!(selected_nodes.len()-next);
    if next<selected_nodes.len() { result["nextCursor"]=json!(format!("{prefix}{next}")); }
    if serde_json::to_vec(&result).expect("inspection serializes").len()+1>super::MAX_STDOUT_BYTES {
        return Err(Error::new("E_OUTPUT_LIMIT",8,"inspection exceeds 8 KiB"));
    }
    Ok(result)
}

fn project(node:&Value,fields:&[String])->Value {
    let mut result=serde_json::Map::new();
    let mut truncated=Vec::new();
    for field in fields {
        let value=&node[field];
        let value=if let Some(text)=value.as_str() && text.len()>128 {
            truncated.push(field);
            json!(&text[..text.floor_char_boundary(128)])
        } else { value.clone() };
        result.insert(field.clone(),value);
    }
    result.insert("truncatedFields".into(),json!(truncated));
    Value::Object(result)
}
const MAX_FILE_BYTES:u64=64*1024*1024;
const MAX_RUN_BYTES:u64=256*1024*1024;
const MAX_ENTRIES:usize=65_536;

super::schema::object_struct! {
    #[serde(rename_all="camelCase")]
    struct Inventory {
        schema_version:u32,
        artifacts:Vec<InventoryEntry>,
        #[serde(rename="self",default)]
        self_path:Option<String>,
    }
}
super::schema::object_struct! {
    struct InventoryEntry {
        path:String,
        role:String,
        status:String,
        #[serde(default)]
        bytes:Option<u64>,
        #[serde(default)]
        digest:Option<String>,
    }
}

pub(super) struct SavedRun {
    root:PathBuf,
    entries:Vec<InventoryEntry>,
    pub inventory_digest:String,
    pub result:Value,
}

pub(super) struct SavedCapture {
    pub tree:Value,
    pub frame:Value,
    pub tree_digest:String,
}

impl SavedRun {
    pub fn open(path:&Path)->Result<Self,Error> {
        let root=path.canonicalize().map_err(io)?;
        let inventory_bytes=read_file(&root,"artifacts.json",MAX_FILE_BYTES)?;
        let mut inventory:Inventory=serde_json::from_slice(&inventory_bytes).map_err(io)?;
        if inventory.schema_version!=1 || inventory.self_path.as_deref().is_some_and(|path|path!="artifacts.json") {
            return Err(schema("unsupported artifact inventory version or self identity"));
        }
        if inventory.artifacts.len()>MAX_ENTRIES { return Err(Error::new("E_OUTPUT_LIMIT",8,"saved inventory exceeds 65536 entries")); }
        inventory.artifacts.sort_unstable_by(|left,right|left.path.cmp(&right.path));
        for entry in &inventory.artifacts { super::output::relative_path(&entry.path)?; }
        if inventory.artifacts.windows(2).any(|pair|pair[0].path==pair[1].path) {
            return Err(io("saved inventory contains duplicate paths"));
        }
        let mut run=Self{root,entries:inventory.artifacts,inventory_digest:digest(&inventory_bytes),result:Value::Null};
        run.result=run.json("result.json","result")?;
        if !matches!(run.result["status"].as_str(),Some("passed"|"failed")) {
            return Err(io("saved run has no terminal status"));
        }
        Ok(run)
    }

    pub fn root(&self)->&Path { &self.root }

    fn entry(&self,path:&str)->Result<&InventoryEntry,Error> {
        let index=self.entries.binary_search_by(|entry|entry.path.as_str().cmp(path))
            .map_err(|_|io(format!("artifact is absent from saved inventory: {path}")))?;
        let entry=&self.entries[index];
        if entry.status!="complete" { return Err(io(format!("saved artifact is partial: {path}"))); }
        Ok(entry)
    }

    pub fn verified(&self,path:&str,role:&str)->Result<Vec<u8>,Error> {
        let entry=self.entry(path)?;
        if entry.role!=role { return Err(io(format!("saved artifact has the wrong role: {path}"))); }
        let bytes=read_file(&self.root,path,MAX_FILE_BYTES)?;
        if entry.bytes!=Some(bytes.len() as u64) || entry.digest.as_deref()!=Some(digest(&bytes).as_str()) {
            return Err(io(format!("saved artifact failed its inventory digest: {path}")));
        }
        Ok(bytes)
    }

    pub fn json(&self,path:&str,role:&str)->Result<Value,Error> {
        serde_json::from_slice(&self.verified(path,role)?).map_err(io)
    }

    pub fn capture(&self,id:&str)->Result<SavedCapture,Error> {
        if !super::scenario::safe_id(id) { return Err(schema("invalid capture ID")); }
        let tree_bytes=self.verified(&format!("captures/{id}.tree.json"),"capture-tree")?;
        let tree=serde_json::from_slice(&tree_bytes).map_err(io)?;
        let frame=self.json(&format!("captures/{id}.frame.json"),"capture-frame")?;
        let tree_digest=digest(&tree_bytes);
        validate_identity(&tree,&frame,&self.result,id,&tree_digest)?;
        Ok(SavedCapture{tree,frame,tree_digest})
    }

    pub fn capture_ids(&self)->Result<Vec<String>,Error> {
        let mut ids=Vec::new();
        for entry in &self.entries {
            if entry.role=="capture-frame" {
                let id=entry.path.strip_prefix("captures/").and_then(|path|path.strip_suffix(".frame.json"))
                    .filter(|id|super::scenario::safe_id(id)).ok_or_else(||io("invalid capture frame path"))?;
                ids.push(id.to_owned());
                for (suffix,role) in [(".tree.json","capture-tree"),(".png","capture-pixels")] {
                    if self.entry(&format!("captures/{id}{suffix}"))?.role!=role { return Err(io("capture is missing a matching tree or PNG")); }
                }
            }
        }
        ids.sort_unstable();
        for entry in &self.entries {
            let suffix=match entry.role.as_str() { "capture-tree"=>".tree.json","capture-pixels"=>".png",_=>continue };
            let id=entry.path.strip_prefix("captures/").and_then(|path|path.strip_suffix(suffix))
                .ok_or_else(||io("invalid capture artifact path"))?;
            if ids.binary_search_by(|known|known.as_str().cmp(id)).is_err() { return Err(io("orphan capture image or tree")); }
        }
        if self.result["captureCount"].as_u64()!=Some(ids.len() as u64) { return Err(io("capture inventory disagrees with terminal capture count")); }
        Ok(ids)
    }

    /// Hash every declared artifact without retaining unrelated payloads. Consumers
    /// rehash the bytes they actually use so concurrent replacement cannot swap them.
    pub fn verify_complete(&self)->Result<usize,Error> {
        let mut total=0u64;
        let mut buffer=[0u8;16*1024];
        for entry in &self.entries {
            let expected=entry.bytes.ok_or_else(||io("artifact has no byte length"))?;
            if entry.status!="complete" || entry.role.is_empty() { return Err(io("run contains partial or untyped artifacts")); }
            total=total.checked_add(expected).ok_or_else(||io("artifact byte count overflow"))?;
            if expected>MAX_FILE_BYTES || total>MAX_RUN_BYTES {
                return Err(Error::new("E_OUTPUT_LIMIT",8,"comparison input exceeds 64 MiB per artifact or 256 MiB per run"));
            }
            let mut file=open_file(&self.root,&entry.path,expected)?.take(expected+1);
            let mut hash=Sha256::new();
            let mut count=0;
            loop {
                let length=file.read(&mut buffer).map_err(io)?;
                if length==0 { break; }
                count+=length as u64;
                hash.update(&buffer[..length]);
            }
            if count!=expected || entry.digest.as_deref()!=Some(format!("sha256:{:x}",hash.finalize()).as_str()) {
                return Err(io(format!("saved artifact failed its inventory digest: {}",entry.path)));
            }
        }
        let mut visited=0;
        self.verify_directory(&self.root,0,&mut visited)?;
        Ok(self.entries.len())
    }

    fn verify_directory(&self,path:&Path,depth:usize,visited:&mut usize)->Result<(),Error> {
        if depth>32 { return Err(Error::new("E_OUTPUT_LIMIT",8,"saved artifact directory exceeds depth 32")); }
        for child in std::fs::read_dir(path).map_err(io)? {
            let child=child.map_err(io)?;
            *visited+=1;
            if *visited>MAX_ENTRIES*2 { return Err(Error::new("E_OUTPUT_LIMIT",8,"saved artifact directory entry limit exceeded")); }
            let kind=child.file_type().map_err(io)?;
            if kind.is_symlink() { return Err(io("saved artifacts cannot contain symlinks")); }
            if kind.is_dir() { self.verify_directory(&child.path(),depth+1,visited)?; }
            else if kind.is_file() {
                let path=child.path();
                let relative=path.strip_prefix(&self.root).map_err(io)?.to_str().ok_or_else(||io("artifact path is not UTF-8"))?;
                if relative!="artifacts.json" { self.entry(relative)?; }
            } else { return Err(io("saved artifacts must be regular files")); }
        }
        Ok(())
    }
}

pub(super) fn read_path(path:&Path,limit:u64)->Result<Vec<u8>,Error> {
    let parent=path.parent().filter(|parent|!parent.as_os_str().is_empty()).unwrap_or(Path::new(".")).canonicalize().map_err(io)?;
    let name=path.file_name().and_then(|name|name.to_str()).ok_or_else(||io("expected a UTF-8 file name"))?;
    read_file(&parent,name,limit)
}

fn open_file(root:&Path,relative:&str,limit:u64)->Result<std::fs::File,Error> {
    let relative=super::output::relative_path(relative)?;
    let mut path=root.to_owned();
    let mut last_metadata=None;
    for component in relative.components() {
        path.push(component);
        let metadata=std::fs::symlink_metadata(&path).map_err(io)?;
        if metadata.file_type().is_symlink() { return Err(io("saved artifacts cannot be symlinks")); }
        last_metadata=Some(metadata);
    }
    let metadata=last_metadata.expect("validated relative path has components");
    if !metadata.is_file() || metadata.len()>limit { return Err(Error::new("E_OUTPUT_LIMIT",8,"saved artifact is not a bounded regular file")); }
    let mut options=std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK);
    }
    let file=options.open(path).map_err(io)?;
    let metadata=file.metadata().map_err(io)?;
    if !metadata.is_file() || metadata.len()>limit { return Err(Error::new("E_OUTPUT_LIMIT",8,"saved artifact changed to an unbounded or non-regular file")); }
    Ok(file)
}

fn read_file(root:&Path,relative:&str,limit:u64)->Result<Vec<u8>,Error> {
    let file=open_file(root,relative,limit)?;
    let mut bytes=Vec::with_capacity(file.metadata().map_err(io)?.len() as usize);
    file.take(limit+1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64>limit { return Err(Error::new("E_OUTPUT_LIMIT",8,"saved artifact grew beyond its read limit")); }
    Ok(bytes)
}
fn schema(message:&str)->Error { Error::new("E_SCHEMA",2,message) }
fn io(error:impl std::fmt::Display)->Error { Error::new("E_OUTPUT_IO",8,error.to_string()) }

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture()->Value {
        json!({"schemaVersion":1,"surface":"main","generation":1,"buildId":"example","frameOrdinal":3,
            "presentationSequence":2,"metadataRevision":1,"truncation":{"nodes":false,"text":false},
            "nodes":[
                {"token":"node-0","parent":null,"testId":null},
                {"token":"node-1","parent":"node-0","testId":"selected"},
                {"token":"node-2","parent":"node-1","testId":"child"},
                {"token":"node-3","parent":"node-0","testId":"unrelated"}]})
    }
    fn selection()->Options {
        options(["--run","unused","--capture","initial","--test-id","selected","--max-nodes","1","--fields","token,parent"].map(str::to_owned).to_vec()).unwrap()
    }
    #[test]
    fn cursor_cannot_cross_snapshot_or_query_and_context_stays_in_selected_subtree() {
        let tree=fixture();
        let hash=digest(&serde_json::to_vec(&tree).unwrap());
        let mut options=selection();
        let first=query(&tree,&hash,Path::new("saved.tree.json"),"failed",&options).unwrap();
        assert_eq!(first["nodes"][0]["token"],"node-1");
        options.cursor=Some(first["nextCursor"].as_str().unwrap().to_owned());
        assert!(query(&tree,"sha256:different",Path::new("saved.tree.json"),"failed",&options).is_err());
        options.max_depth=0;
        assert!(query(&tree,&hash,Path::new("saved.tree.json"),"failed",&options).is_err());
        options.max_depth=1;
        let parent=query(&tree,&hash,Path::new("saved.tree.json"),"failed",&options).unwrap();
        assert_eq!(parent["nodes"][0]["token"],"node-0");
        options.cursor=Some(parent["nextCursor"].as_str().unwrap().to_owned());
        let child=query(&tree,&hash,Path::new("saved.tree.json"),"failed",&options).unwrap();
        assert_eq!(child["nodes"][0]["token"],"node-2");
        assert!(child["nextCursor"].is_null());
        assert_eq!(child["sourceRunStatus"],"failed");
    }
    #[test]
    fn byte_pagination_preserves_all_nodes_and_marks_utf8_prefixes() {
        let mut tree=fixture();
        tree["nodes"]=json!((0..80).map(|index|json!({
            "token":format!("node-{index}"),"parent":(index>0).then_some("node-0"),
            "name":"界".repeat(512),"text":"🙂".repeat(512)
        })).collect::<Vec<_>>());
        let hash=digest(&serde_json::to_vec(&tree).unwrap());
        let mut options=options(["--run","unused","--capture","initial","--max-nodes","100","--fields","token,name,text"].map(str::to_owned).to_vec()).unwrap();
        let mut tokens=HashSet::new();
        let mut pages=0;
        loop {
            let page=query(&tree,&hash,Path::new("saved.tree.json"),"passed",&options).unwrap();
            assert!(serde_json::to_vec(&page).unwrap().len()+1<=super::super::MAX_STDOUT_BYTES);
            for node in page["nodes"].as_array().unwrap() {
                assert!(tokens.insert(node["token"].as_str().unwrap().to_owned()));
                assert_eq!(node["truncatedFields"],json!(["name","text"]));
                assert!(node["name"].as_str().unwrap().len()<=128);
                assert!(node["text"].as_str().unwrap().len()<=128);
            }
            pages+=1;
            options.cursor=page["nextCursor"].as_str().map(str::to_owned);
            if options.cursor.is_none() { break; }
            assert!(pages<80);
        }
        assert_eq!(tokens.len(),80);
        assert!(pages>1);
    }
    #[test]
    fn internally_consistent_capture_cannot_belong_to_another_capture_or_run() {
        let tree=fixture();
        let frame=json!({"schemaVersion":1,"captureId":"initial","treeDigest":"hash","layoutDocumentCoherent":true,
            "buildId":"example","generation":1,"surface":"main","metadataRevision":1,"validationFrame":{"ordinal":3},"pixelPresentation":{"sequence":2}});
        let result=json!({"schemaVersion":1,"command":"run","build":{"buildId":"example"},"generation":1});
        validate_identity(&tree,&frame,&result,"initial","hash").unwrap();
        let mut later_result=result.clone();
        later_result["generation"]=json!(2);
        validate_identity(&tree,&frame,&later_result,"initial","hash").unwrap();
        let mut mismatched_frame=frame.clone();
        mismatched_frame["generation"]=json!(2);
        assert!(validate_identity(&tree,&mismatched_frame,&later_result,"initial","hash").is_err());
        assert!(validate_identity(&tree,&frame,&result,"after","hash").is_err());
        for (field,value) in [("buildId",json!("other-build")),("generation",json!(2))] {
            let mut tree=tree.clone();
            let mut frame=frame.clone();
            tree[field]=value.clone();
            frame[field]=value;
            assert!(validate_identity(&tree,&frame,&result,"initial","hash").is_err());
        }
        for revision in [None,Some(Value::Null),Some(json!("1")),Some(json!(-1))] {
            let mut tree=tree.clone();
            let mut frame=frame.clone();
            tree.as_object_mut().unwrap().remove("metadataRevision");
            frame.as_object_mut().unwrap().remove("metadataRevision");
            if let Some(value)=revision { tree["metadataRevision"]=value.clone(); frame["metadataRevision"]=value; }
            assert!(validate_identity(&tree,&frame,&result,"initial","hash").is_err());
        }
        for field in ["resourceRevision","surfaceConfiguration"] {
            let mut tree=tree.clone();
            let mut frame=frame.clone();
            for identity in ["resourceRevision","surfaceConfiguration"] { tree[identity]=json!(1);frame[identity]=json!(1); }
            frame["resources"]=json!({"requiredAssetsReady":true,"resourceRevision":1});
            tree[field]=json!(1);frame[field]=json!(2);
            assert!(validate_identity(&tree,&frame,&result,"initial","hash").is_err());
            frame[field]=json!(1);
            validate_identity(&tree,&frame,&result,"initial","hash").unwrap();
            tree.as_object_mut().unwrap().remove(field);frame.as_object_mut().unwrap().remove(field);
            assert!(validate_identity(&tree,&frame,&result,"initial","hash").is_err());
            tree[field]=Value::Null;frame[field]=Value::Null;
            assert!(validate_identity(&tree,&frame,&result,"initial","hash").is_err());
        }
        let mut frame=frame;
        frame["layoutDocumentCoherent"]=json!(false);
        assert!(validate_identity(&tree,&frame,&result,"initial","hash").is_err());
    }
}
