//! Offline comparison of complete, reviewed native capture evidence.
mod pixels;
mod policy;

use std::{collections::{BTreeSet,HashSet},path::{Path,PathBuf}};
use serde_json::{Value,json};
use crate::bounded_fs;
use super::{Error,evidence::FrameEvidence,output::{Output,bounded_read_error,digest},saved::{SavedCapture,SavedRun}};
use crate::schema::object_struct;
use policy::{CaptureKey,CaptureSet,Policy};

struct Options { baseline:PathBuf, actual:PathBuf, policy:PathBuf, out:PathBuf }

fn options(args:Vec<String>)->Result<Options,Error> {
    let mut args=args.into_iter();
    let mut seen=HashSet::new();
    let(mut baseline,mut actual,mut policy,mut out)=(None,None,None,None);
    while let Some(key)=args.next() {
        if !seen.insert(key.clone()) { return Err(invalid(format!("duplicate option: {key}"))); }
        let value=args.next().ok_or_else(||invalid(format!("missing value for {key}")))?;
        match key.as_str() {
            "--baseline"=>baseline=Some(PathBuf::from(value)),
            "--actual"=>actual=Some(PathBuf::from(value)),
            "--policy"=>policy=Some(PathBuf::from(value)),
            "--out"=>out=Some(PathBuf::from(value)),
            _=>return Err(invalid(format!("unknown compare option: {key}"))),
        }
    }
    Ok(Options{
        baseline:baseline.ok_or_else(||invalid("--baseline is required"))?,
        actual:actual.ok_or_else(||invalid("--actual is required"))?,
        policy:policy.ok_or_else(||invalid("--policy is required"))?,
        out:out.ok_or_else(||invalid("--out is required"))?,
    })
}

object_struct! {
    #[serde(rename_all="camelCase",deny_unknown_fields)]
    struct Baseline {
        schema_version:u32,
        run:String,
        artifacts_digest:String,
        policy_digest:String,
        review:Review,
    }
}
object_struct! {
    #[serde(rename_all="camelCase",deny_unknown_fields)]
    struct Review {
        by:String,
        at:String,
        summary:String,
        build_id:String,
        captures:Vec<CaptureKey>,
    }
}

impl Baseline {
    fn load(root:&Path)->Result<Self,Error> {
        let canonical_root=root.canonicalize().map_err(io)?;
        let descriptor:Self=serde_json::from_slice(&bounded_fs::read_contained(&canonical_root,"baseline.json",64*1024).map_err(bounded_io_error)?)
            .map_err(|error|invalid(format!("invalid baseline review: {error}")))?;
        if descriptor.schema_version!=1 || descriptor.run!="run" || !sha256(&descriptor.artifacts_digest)
            || !sha256(&descriptor.policy_digest) || !sha256(&descriptor.review.build_id) {
            return Err(invalid("baseline requires schemaVersion 1, run directory 'run' and SHA-256 identities"));
        }
        for (text,maximum) in [(&descriptor.review.by,128),(&descriptor.review.at,128),(&descriptor.review.summary,2048)] {
            if text.trim().is_empty() || text.chars().count()>maximum { return Err(invalid("baseline review requires bounded nonempty by, at and summary fields")); }
        }
        let keys:BTreeSet<_>=descriptor.review.captures.iter().collect();
        if keys.is_empty() || keys.len()>4096 || keys.len()!=descriptor.review.captures.len()
            || keys.iter().any(|key|!super::scenario::safe_id(&key.capture_id)||key.surface!="main") {
            return Err(invalid("baseline review must name 1..4096 unique main-surface captures"));
        }
        let run=canonical_root.join(&descriptor.run);
        let metadata=std::fs::symlink_metadata(&run).map_err(io)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() { return Err(io("baseline run must be a real directory")); }
        if digest(&bounded_fs::read_contained(&canonical_root,"policy.json",64*1024).map_err(bounded_io_error)?)!=descriptor.policy_digest {
            return Err(io("reviewed policy no longer matches its baseline review"));
        }
        Ok(descriptor)
    }
}

struct Source {
    run:SavedRun,
    environment:Value,
    graphics:Value,
    profile:Value,
    fixture:Value,
    scenario:Value,
    keys:BTreeSet<CaptureKey>,
    verified_artifacts:usize,
}

impl Source {
    fn load(root:&Path)->Result<Self,Error> {
        let run=SavedRun::open(root)?;
        let verified_artifacts=run.verify_complete()?;
        let environment=run.json("environment.json","environment")?;
        let graphics=run.json("graphics.json","graphics-environment")?;
        let profile=run.json("inputs/profile.json","resolved-profile")?;
        let fixture_bytes=run.verified("inputs/fixture.json","fixture")?;
        let scenario_bytes=run.verified("inputs/scenario.json","scenario")?;
        let fixture:Value=serde_json::from_slice(&fixture_bytes).map_err(io)?;
        let scenario:Value=serde_json::from_slice(&scenario_bytes).map_err(io)?;
        let bundle=run.json("inputs/bundle.json","bundle-identity")?;
        if run.result["schemaVersion"]!=1 || run.result["command"]!="run" || environment["schemaVersion"]!=1
            || !bundle.is_object() || environment["application"]!=bundle
            || bundle["buildId"]!=run.result["build"]["buildId"]
            || !bundle["buildId"].as_str().is_some_and(sha256)
            || fixture["schemaVersion"]!=1 || scenario["schemaVersion"]!=1 || profile["schemaVersion"]!=1
            || fixture["id"]!=environment["fixture"]["id"] || scenario["id"]!=environment["scenario"]["id"]
            || environment["fixture"]["digest"]!=digest(&fixture_bytes) || environment["scenario"]["digest"]!=digest(&scenario_bytes)
            || scenario["id"]!=run.result["scenarioId"] {
            return Err(io("run, environment and saved input identities disagree"));
        }
        let canonical=super::profile::canonical(json!({"profile":profile,"fontEvidence":environment["fonts"]}));
        let profile_digest=digest(&serde_json::to_vec(&canonical).map_err(io)?);
        if environment["profileDigest"]!=profile_digest.trim_start_matches("sha256:") {
            return Err(io("profile digest does not bind the saved profile and font evidence"));
        }
        let keys=run.capture_ids()?.into_iter().map(|capture_id|CaptureKey{capture_id,surface:"main".into()}).collect();
        Ok(Self{run,environment,graphics,profile,fixture,scenario,keys,verified_artifacts})
    }

    fn capture(&self,key:&CaptureKey,max_pixels:u64)->Result<(SavedCapture,Vec<u8>,pixels::Image),Error> {
        let capture=self.run.capture(&key.capture_id)?;
        let png=capture_png(self,&capture.frame,key)?;
        let image=pixels::decode(&png,max_pixels)?;
        if capture.frame.detail("physicalWidth").as_u64()!=Some(u64::from(image.width))
            || capture.frame.detail("physicalHeight").as_u64()!=Some(u64::from(image.height)) {
            return Err(io("PNG dimensions disagree with the owning native frame"));
        }
        Ok((capture,png,image))
    }

    fn provenance(&self)->Value {
        json!({"run":self.run.root(),"buildId":self.run.result["build"]["buildId"],
            "hostSourceTreeDigest":self.environment["host"]["sourceTreeDigest"],
            "profileDigest":self.environment["profileDigest"],"fixture":self.environment["fixture"],
            "scenario":self.environment["scenario"],"renderer":self.graphics,
            "verifiedArtifacts":self.verified_artifacts,"assertions":self.run.result["assertions"]})
    }
}

pub(super) fn command(args:Vec<String>)->Result<Value,Error> {
    let options=options(args)?;
    let policy_bytes=bounded_fs::read_input(&options.policy,64*1024).map_err(bounded_io_error)?;
    let policy=Policy::parse(&policy_bytes)?;
    let mut output=Output::create(&options.out,policy.max_output_bytes)?;
    let outcome=execute(&options,&policy,&policy_bytes,&mut output);
    let result=match outcome {
        Ok(result)=>result,
        Err(error)=>{
            let mut result=super::failure("compare",error);
            result["artifacts"]=json!({"manifest":"artifacts.json"});
            result
        }
    };
    Ok(output.finalize("compare",result))
}

fn execute(options:&Options,policy:&Policy,policy_bytes:&[u8],output:&mut Output)->Result<Value,Error> {
    let descriptor=Baseline::load(&options.baseline)?;
    let baseline=Source::load(&options.baseline.join(&descriptor.run))?;
    let actual=Source::load(&options.actual)?;
    let policy_digest=digest(policy_bytes);
    output.bytes("policy.json","comparison-policy",policy_bytes)?;
    output.json("baseline-review.json","baseline-review",&serde_json::to_value(&descriptor).map_err(io)?)?;
    let mut issues=compatibility(&baseline,&actual);
    if descriptor.policy_digest!=policy_digest {
        issues.push(json!({"field":"reviewedPolicy","baseline":descriptor.policy_digest,"actual":policy_digest}));
    }
    let reviewed:BTreeSet<_>=descriptor.review.captures.iter().cloned().collect();
    if reviewed!=baseline.keys || descriptor.review.build_id!=baseline.run.result["build"]["buildId"]
        || descriptor.artifacts_digest!=baseline.run.inventory_digest {
        return Err(io("baseline review does not bind the consumed inventory, build and capture set"));
    }
    let selected=match &policy.capture_set {
        CaptureSet::All=>{
            if baseline.keys!=actual.keys { issues.push(json!({"field":"captureSet","missing":baseline.keys.difference(&actual.keys).collect::<Vec<_>>(),"unexpected":actual.keys.difference(&baseline.keys).collect::<Vec<_>>()})); }
            baseline.keys.union(&actual.keys).cloned().collect::<BTreeSet<_>>()
        }
        CaptureSet::Subset{captures,..}=>captures.iter().cloned().collect(),
    };
    if selected.is_empty() { issues.push(json!({"field":"captureSet","reason":"empty comparisons cannot pass"})); }
    else { policy.validate_targets(&selected)?; }
    for key in &selected {
        if !baseline.keys.contains(key) || !actual.keys.contains(key) {
            issues.push(json!({"field":"requiredCapture","capture":key,"baselinePresent":baseline.keys.contains(key),"actualPresent":actual.keys.contains(key)}));
        }
    }
    // Subsets exclude scoring, never the validity of the producer's complete run.
    for (side,source) in [("baseline",&baseline),("actual",&actual)] {
        for key in source.keys.difference(&selected) {
            let (capture,_,_)=source.capture(key,policy.max_pixels)?;
            let mut details=Vec::new();
            capture.frame.readiness_issues(side,&mut details);
            if !details.is_empty() {
                issues.push(json!({"field":"excludedCapture","side":side,"capture":key,"details":details}));
            }
        }
    }
    let global_compatible=issues.is_empty();
    let(mut compared,mut different,mut mismatched,mut incompatible)=(0usize,0usize,0usize,0usize);
    let mut captures=Vec::new();
    for key in &selected {
        if !baseline.keys.contains(key) || !actual.keys.contains(key) {
            incompatible+=1;
            captures.push(json!({"capture":key,"status":"missing"}));
            continue;
        }
        let (left,left_png,left_image)=baseline.capture(key,policy.max_pixels)?;
        let (right,right_png,right_image)=actual.capture(key,policy.max_pixels)?;
        let capture_issues=capture_compatibility(&left.frame,&right.frame);
        let eligible=global_compatible && capture_issues.is_empty();
        let comparison=if eligible { compared+=1;Some(pixels::compare(key,&left_image,&right_image,policy)?) } else { incompatible+=1;None };
        if comparison.as_ref().is_some_and(|comparison|!comparison.exact_equal) { different+=1; }
        if comparison.as_ref().is_some_and(|comparison|!comparison.passed) { mismatched+=1; }
        let mut artifacts=json!({});
        if comparison.as_ref().is_none_or(|comparison|!comparison.exact_equal) {
            let prefix=format!("captures/{}-{}",key.surface,key.capture_id);
            let before=format!("{prefix}.baseline.png");
            let after=format!("{prefix}.actual.png");
            output.bytes(&before,"comparison-baseline-pixels",&left_png)?;
            output.bytes(&after,"comparison-actual-pixels",&right_png)?;
            artifacts["baseline"]=json!(before);
            artifacts["actual"]=json!(after);
            if let Some(diff)=comparison.as_ref().and_then(|comparison|comparison.diff_png.as_ref()) {
                let path=format!("{prefix}.diff.png");
                output.bytes(&path,"comparison-diff-pixels",diff)?;
                artifacts["diff"]=json!(path);
            }
        }
        captures.push(json!({"capture":key,"status":match &comparison {None=>"incompatible",Some(comparison) if comparison.passed=>"passed",Some(_)=>"different"},
            "compatibilityIssues":capture_issues,"exactEqual":comparison.as_ref().map(|comparison|comparison.exact_equal),
            "metrics":comparison.as_ref().map(|comparison|&comparison.metrics),"artifacts":artifacts,
            "baselineFrame":{"validation":left.frame.validation_frame,"presentation":left.frame.pixel_presentation,"captureTimeUs":left.frame.detail("readiness")["timeUs"]},
            "actualFrame":{"validation":right.frame.validation_frame,"presentation":right.frame.pixel_presentation,"captureTimeUs":right.frame.detail("readiness")["timeUs"]}}));
    }
    let error=if !global_compatible || incompatible>0 {
        Some(Error::new("E_BASELINE_INCOMPATIBLE",2,"baseline inputs, capture set or native frame environments are incompatible").with_details(json!({"report":"report.json"})))
    } else if mismatched>0 {
        Some(Error::new("E_VISUAL_DIFF",4,"native pixels exceed the reviewed comparison policy").with_details(json!({"report":"report.json"})))
    } else { None };
    let exit=error.as_ref().map_or(0,|error|error.exit_code);
    let status=if exit==0 {"passed"} else {"failed"};
    let excluded_baseline:Vec<_>=baseline.keys.difference(&selected).collect();
    let excluded_actual:Vec<_>=actual.keys.difference(&selected).collect();
    output.json("report.json","comparison-report",&json!({"schemaVersion":1,"comparisonVersion":"native-rgba8-v1","status":status,"exitCode":exit,
        "baseline":baseline.provenance(),"actual":actual.provenance(),"review":descriptor.review,
        "policy":{"digest":policy_digest,"artifact":"policy.json"},"captureSet":policy.capture_set,
        "excludedBaselineCaptures":excluded_baseline,"excludedActualCaptures":excluded_actual,
        "compatibilityIssues":issues,"captures":captures}))?;
    Ok(json!({"schemaVersion":1,"command":"compare","status":status,"exitCode":exit,"firstError":error,
        "scope":match policy.capture_set {CaptureSet::All=>"all",CaptureSet::Subset{..}=>"subset"},
        "policyDigest":policy_digest,"builds":{"baseline":baseline.run.result["build"]["buildId"],"actual":actual.run.result["build"]["buildId"]},
        "captures":{"compared":compared,"different":different,"failed":mismatched,"incompatible":incompatible,
            "excludedBaseline":excluded_baseline.len(),"excludedActual":excluded_actual.len()},
        "artifacts":{"manifest":"artifacts.json","report":"report.json"}}))
}

fn capture_png(source:&Source,frame:&FrameEvidence,key:&CaptureKey)->Result<Vec<u8>,Error> {
    let bytes=source.run.verified(&super::evidence::CapturePaths::new(&key.capture_id)?.png(),"capture-pixels")?;
    if frame.png_digest.as_deref()!=Some(digest(&bytes).as_str()) || frame.extra.get("renderer")!=Some(&source.graphics) {
        return Err(io("PNG or renderer identity disagrees with its native capture frame"));
    }
    Ok(bytes)
}

fn compatibility(baseline:&Source,actual:&Source)->Vec<Value> {
    let mut issues=Vec::new();
    for (name,source) in [("baseline",baseline),("actual",actual)] {
        if source.run.result["status"]!="passed" || source.run.result["exitCode"]!=0 || source.run.result["firstError"]!=Value::Null
            || source.run.result["assertions"]["failed"]!=0 || source.run.result["assertions"]["passed"].as_u64().is_none()
            || source.environment["timingDeterministic"]!=true || source.run.result["timingDeterministic"]!=true
            || source.environment["clock"]["mode"]!="virtual" || source.environment["fonts"]["mode"]!="shipped-only"
            || source.environment["fixture"]["providerPolicy"]!="fixture-only" {
            issues.push(json!({"field":"sourceRun","side":name,"reason":"comparison requires a successful deterministic native run with shipped fonts, fixture-only services and no failed assertions"}));
        }
    }
    for path in ["/application/id","/capabilityRevision","/hostRootStyleDigest","/clock","/random","/fonts","/profileDigest","/fixture","/scenario",
        "/resourcePublication","/inputPolicy","/dependencies/cargoLockDigest","/dependencies/pnpmLockDigest","/dependencies/quickJsEngineRevision",
        "/host/profile","/host/rustc","/host/target"] {
        equal_required(&mut issues,path,baseline.environment.pointer(path),actual.environment.pointer(path));
    }
    for (name,left,right) in [("profile",&baseline.profile,&actual.profile),("fixtureInput",&baseline.fixture,&actual.fixture),
        ("scenarioInput",&baseline.scenario,&actual.scenario),("renderer",&baseline.graphics,&actual.graphics),
        ("assertions",&baseline.run.result["assertions"],&actual.run.result["assertions"])] {
        equal_required(&mut issues,name,Some(left),Some(right));
    }
    for field in ["adapter","backend","alphaMode","driver","driverInfo","pngColor","sceneFormat","scratchFormat","targetFormat","storedColor","srgbTier","conversionRevision","vendor","device","deviceType","additionalTransferConversion"] {
        equal_required(&mut issues,&format!("renderer.{field}"),baseline.graphics.get(field),actual.graphics.get(field));
    }
    if baseline.graphics["pngColor"]!="straight-alpha-gamma-encoded" || actual.graphics["pngColor"]!="straight-alpha-gamma-encoded" {
        issues.push(json!({"field":"renderer.pngColor","reason":"unsupported native PNG color contract"}));
    }
    issues
}

fn capture_compatibility(left:&FrameEvidence,right:&FrameEvidence)->Vec<Value> {
    let mut issues=Vec::new();
    equal_required(&mut issues,"/surface",Some(&json!(left.surface)),Some(&json!(right.surface)));
    for field in ["physicalWidth","physicalHeight","scale","timingDeterministic","renderer","allowedPendingAssets"] {
        equal_required(&mut issues,&format!("/{field}"),left.extra.get(field),right.extra.get(field));
    }
    equal_required(&mut issues,"/readiness/timeUs",
        left.extra.get("readiness").and_then(|value|value.get("timeUs")),
        right.extra.get("readiness").and_then(|value|value.get("timeUs")));
    equal_required(&mut issues,"/validationFrame/timeUs",
        Some(&json!(left.validation_frame.time_us)),Some(&json!(right.validation_frame.time_us)));
    equal_required(&mut issues,"/pixelPresentation/frame/timeUs",
        Some(&json!(left.pixel_presentation.frame.time_us)),Some(&json!(right.pixel_presentation.frame.time_us)));
    left.readiness_issues("baseline",&mut issues);
    right.readiness_issues("actual",&mut issues);
    issues
}

fn equal_required(issues:&mut Vec<Value>,field:&str,left:Option<&Value>,right:Option<&Value>) {
    if left.is_none_or(Value::is_null) || right.is_none_or(Value::is_null) || left!=right {
        issues.push(json!({"field":field,"baseline":left,"actual":right}));
    }
}
fn sha256(value:&str)->bool { value.strip_prefix("sha256:").is_some_and(|hash|hash.len()==64&&hash.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte))) }
fn invalid(message:impl Into<String>)->Error { Error::new("E_SCHEMA",2,message) }
fn io(error:impl std::fmt::Display)->Error { Error::new("E_OUTPUT_IO",8,error.to_string()) }
fn bounded_io_error(error:bounded_fs::BoundedReadError)->Error {
    bounded_read_error(error,"comparison input is not a bounded regular file")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_load_rejects_symlinked_review_file_while_policy_allows_symlink() {
        let dir = super::super::tests::TempDir::create("compare");
        let target = dir.0.join("real-policy.json");
        std::fs::write(&target, br#"{"schemaVersion":1,"id":"p","captureSet":"all","maxOutputBytes":65536,"maxPixels":1000,"thresholds":{"maxDiffPixels":0,"maxChannelDelta":0,"meanDeltaLimit":0.0}}"#).unwrap();

        #[cfg(unix)]
        {
            let policy_link = dir.0.join("policy-link.json");
            std::os::unix::fs::symlink(&target, &policy_link).unwrap();
            assert!(!bounded_fs::read_input(&policy_link, 64 * 1024).unwrap().is_empty());

            let baseline_dir = dir.0.join("baseline");
            std::fs::create_dir(&baseline_dir).unwrap();
            std::os::unix::fs::symlink(&target, baseline_dir.join("baseline.json")).unwrap();
            let err = Baseline::load(&baseline_dir).err().unwrap();
            assert_eq!(err.code, "E_OUTPUT_IO");
        }
    }
}

