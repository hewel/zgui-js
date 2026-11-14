use std::{cell::RefCell, path::{Path, PathBuf}, rc::Rc, time::{Duration, Instant}};
use serde_json::{Value, json};
use crate::package::Package;
use super::{Error, driver::{Driver, Fault, SharedOutput}, fixture, inventory, output::Output, profile, scenario::{self, Step}, supervisor};

pub(super) struct Options {
    pub(super) bundle: PathBuf,
    pub(super) fixture: PathBuf,
    pub(super) scenario: PathBuf,
    pub(super) common: super::DoctorOptions,
}
fn options(args: impl Iterator<Item=String>) -> Result<Options,Error> {
    let mut args=args.peekable();
    let mut bundle=None;
    let mut fixture=None;
    let mut scenario=None;
    let mut common=Vec::new();
    let mut seen=std::collections::BTreeSet::new();
    while let Some(key)=args.next() {
        if !seen.insert(key.clone()) { return Err(Error::new("E_SCHEMA",2,format!("duplicate option: {key}"))); }
        let value=args.next().ok_or_else(||Error::new("E_SCHEMA",2,format!("missing value for {key}")))?;
        match key.as_str() {
            "--bundle" => bundle=Some(PathBuf::from(value)),
            "--fixture" => fixture=Some(PathBuf::from(value)),
            "--scenario" => scenario=Some(PathBuf::from(value)),
            _ => { common.push(key); common.push(value); }
        }
    }
    let common=super::doctor_options(common.into_iter())?;
    if common.out.is_none() { return Err(Error::new("E_SCHEMA",2,"--out is required")); }
    Ok(Options {
        bundle:bundle.ok_or_else(||Error::new("E_SCHEMA",2,"--bundle is required"))?,
        fixture:fixture.ok_or_else(||Error::new("E_SCHEMA",2,"--fixture is required"))?,
        scenario:scenario.ok_or_else(||Error::new("E_SCHEMA",2,"--scenario is required"))?,common,
    })
}

pub(super) fn validate(args: &[String]) -> Result<(Options, profile::Profile), Error> {
    let options=options(args.iter().cloned())?;
    let (_,profile)=profile::read_profile(&options.common.profile,&options.common.overrides)?;
    // Capability/schema errors cannot execute the application, allocate a GPU, or claim a run.
    let scenario = scenario::load(&options.scenario,super::RUN_CAPABILITIES,profile.clock.is_virtual())?;
    validate_steps(&scenario.scenario, &profile)?;
    fixture::load(&options.fixture)?;
    Ok((options, profile))
}

pub fn command(args: Vec<String>) -> Result<Value,Error> {
    controlled(args, None)
}

pub(super) fn controlled(args: Vec<String>, canceled: Option<&std::sync::atomic::AtomicBool>) -> Result<Value,Error> {
    let (options, profile)=validate(&args)?;
    let mut output=Output::create(options.common.out.as_deref().expect("required output"),profile.limits.max_output_bytes)?;
    let mut child=std::process::Command::new(std::env::current_exe().map_err(|error|Error::new("E_WORKER_CRASH",9,error.to_string()))?);
    child.args(["preview","_run-worker"]).args(args).env("ZGUI_PREVIEW_PARENT_PID",std::process::id().to_string()).env("TZ","UTC");
    let report=supervisor::controlled(&mut child,Duration::from_millis(profile.limits.max_run_wall_ms),canceled);
    let mut result=match report {
        Ok(report) => {
            if let Err(error)=output.restore_worker() {
                publication_failure(error, &report.result)
            } else { report.result }
        }
        Err(error) => {
            let mut failed=super::failure("run",error);
            failed["teardown"]=json!({"verified":false,"clean":null});
            let recovery=(|| {
                output.recover()?;
                if output.root().join("failure-cause.json").is_file() {
                    let cause=read_evidence(&output.root().join("failure-cause.json"))?;
                    failed["terminationError"]=failed["firstError"].clone();
                    failed["firstError"]=cause["summary"].clone();
                    failed["exitCode"]=cause["summary"]["exitCode"].clone();
                    failed["build"]=cause["build"].clone();
                }
                recover_failure(&mut output)
            })();
            match recovery {
                Ok(evidence) => failed["failureEvidence"]=evidence,
                Err(error) => failed=publication_failure(error, &failed),
            }
            failed
        }
    };
    if result["firstError"]["code"]=="E_RUN_CANCELED" || result["terminationError"]["code"]=="E_RUN_CANCELED" {
        result["cancellation"]=json!({"requested":true,"workerStopped":true,"cleanTeardownVerified":false});
        if result["firstError"]["code"]=="E_RUN_CANCELED" { result["status"]=json!("canceled"); }
    }
    if serde_json::to_vec(&result).expect("result serializes").len()+1>super::MAX_STDOUT_BYTES {
        result=publication_failure(Error::new("E_OUTPUT_LIMIT",8,"run summary exceeds 8 KiB"), &result);
    }
    if let Err(error)=output.finish(&result) {
        result=publication_failure(error, &result);
    }
    Ok(result)
}

pub fn worker(args: impl Iterator<Item=String>) -> supervisor::Report {
    let started=Instant::now();
    let mut output:Option<SharedOutput>=None;
    let mut driver=None;
    let mut stage="preflight";
    let mut step_id=None::<String>;
    let mut action=0_u64;
    let mut build=Value::Null;
    let mut scenario_id=Value::Null;
    let mut steps=Vec::<Value>::new();
    let mut diagnostics=Vec::<Value>::new();
    let mut original_error=None::<Value>;
    let mut cause_saved=false;
    let mut captures=0;
    let mut latest=Value::Null;
    let mut assertions_passed=0;
    let mut assertions_failed=0;
    let mut timing_deterministic=false;
    let mut fixture_runtime=None;
    let outcome=(|| -> Result<(),Fault> {
        let options=options(args)?;
        let (_,declared)=profile::read_profile(&options.common.profile,&options.common.overrides)?;
        let parent=std::env::var("ZGUI_PREVIEW_PARENT_PID").ok().and_then(|value|value.parse().ok())
            .ok_or_else(||Error::new("E_HOST_INVARIANT",9,"worker has no controller identity"))?;
        let sink=Rc::new(RefCell::new(Output::attach_worker(options.common.out.as_deref().expect("required output"),declared.limits.max_output_bytes,parent)?));
        output=Some(sink.clone());
        let profile=profile::load(&options.common.profile,&options.common.overrides)?;
        let fixture=fixture::load(&options.fixture)?;
        let scenario=scenario::load(&options.scenario,super::RUN_CAPABILITIES,profile.profile.clock.is_virtual())?;
        validate_steps(&scenario.scenario, &profile.profile)?;
        timing_deterministic=profile.profile.clock.is_virtual();
        scenario_id=json!(scenario.scenario.id);
        sink.borrow_mut().json("inputs/profile.json","resolved-profile",&profile.resolved_json)?;
        sink.borrow_mut().bytes("inputs/fixture.json","fixture",&fixture.original)?;
        sink.borrow_mut().bytes("inputs/scenario.json","scenario",&scenario.original)?;
        let package=Rc::new(Package::load(&options.bundle).map_err(|error|Error::new("E_BUNDLE",3,error.to_string()))?);
        build=json!({"id":package.manifest.id,"buildId":package.manifest.build_id,"sourceMap":package.manifest.source_map,
            "sourceMapDigest":package.source_map_digest,"entryDigest":super::output::digest(package.source.as_bytes())});
        sink.borrow_mut().json("inputs/bundle.json","bundle-identity",&build)?;
        let root_styles=super::driver::stylesheet(&profile);
        let mut environment=inventory::snapshot()?;
        environment["profileDigest"]=json!(profile.digest);
        environment["fonts"]=profile.font_evidence.clone();
        environment["fixture"]=json!({"id":fixture.id,"digest":fixture.digest,"providerPolicy":"fixture-only"});
        environment["scenario"]=json!({"id":scenario.scenario.id,"digest":scenario.digest,"requiredCapabilities":scenario.scenario.required_capabilities});
        environment["application"]=build.clone();
        environment["capabilities"]=super::capabilities(true);
        environment["capabilityRevision"]=json!("native-preview-v1-p5");
        environment["hostRootStyleDigest"]=json!(super::output::digest(root_styles.as_bytes()));
        environment["inputPolicy"]=json!({"surfaceFocus":"main","dispatch":"one native deliver_all burst per action; no frame between low-level events","modifiers":"key-local; restored to none after key-up"});
        environment["timingDeterministic"]=json!(timing_deterministic);
        environment["clock"]=super::clock::Environment::configuration(
            match profile.profile.clock { profile::Clock::Virtual{epoch_ms,..}=>Some(epoch_ms),profile::Clock::Wall{}=>None },
            profile.profile.random.seed);
        environment["random"]=environment["clock"]["random"].clone();
        environment["resourcePublication"]=json!("complete-batch-launch-order-v1");
        environment["graphicsEvidence"]=json!("graphics.json");
        sink.borrow_mut().json("environment.json","environment",&environment)?;
        stage="mount";
        sink.borrow_mut().json("progress/action-000000.json","action-progress",&json!({"stage":stage,"actionSequence":0,"build":build}))?;
        driver=Some(Driver::new(&profile,package,fixture.definition,sink.clone(),started+Duration::from_millis(profile.profile.limits.max_run_wall_ms),root_styles)?);
        let active=driver.as_mut().expect("driver installed");
        fixture_runtime=Some(active.fixtures.clone());
        sink.borrow_mut().json("graphics.json","graphics-environment",&active.graphics())?;
        active.check()?;
        active.settle(false)?;
        for (index,step) in scenario.scenario.steps.iter().enumerate() {
            action=index as u64+1;
            step_id=Some(step.id().to_owned());
            stage=step.op();
            active.begin_step(action,match step { Step::Settle{timeout_ms,..}=>*timeout_ms,_=>None });
            sink.borrow_mut().json(&format!("progress/action-{action:06}.json"),"action-progress",&json!({"stage":stage,"step":step.id(),"actionSequence":action,"build":build,"readiness":active.readiness()}))?;
            let step_started=Instant::now();
            let result=match step {
                Step::Settle{..}=>active.settle(false),
                Step::Capture{capture_id,required_frame_time_us,allowed_pending_assets,..}=> {
                    active.capture(capture_id,*required_frame_time_us,allowed_pending_assets).inspect(|capture| { captures+=1; latest=capture.clone(); })
                }
                Step::PointerMove{location,..}=>active.pointer(location,None),
                Step::PointerClick{location,button,..}=>active.pointer(location,Some(*button)),
                Step::KeyPress{surface,key,modifiers,..}=>active.key(*key,modifiers).map(|mut trace| { trace["surface"]=json!(surface); trace }),
                Step::TextCommit{surface,text,..}=>active.text(text).map(|mut trace| { trace["surface"]=json!(surface); trace }),
                Step::Assert{target,check,..}=> {
                    let result=active.assert_node(target,check);
                    if result.is_ok() { assertions_passed+=1; } else { assertions_failed+=1; }
                    result
                }
                Step::ClockAdvance{by_ms,..}=>active.advance(*by_ms),
                Step::ServiceEmit{channel,value,..}=>active.emit(channel,value),
                Step::AssertService{check,..}=> {
                    let result=active.assert_service(check);
                    if result.is_ok() { assertions_passed+=1; } else { assertions_failed+=1; }
                    result
                }
                Step::SurfaceResize{surface,width,height,..}=>active.resize(*width,*height).map(|mut trace| { trace["surface"]=json!(surface); trace }),
                Step::SurfaceScale{surface,scale,..}=>active.scale(*scale).map(|mut trace| { trace["surface"]=json!(surface); trace }),
                Step::SurfaceRecreate{surface,..}=>active.recreate().map(|mut trace| { trace["surface"]=json!(surface); trace }),
            };
            if let Err(fault)=&result {
                let summary=cause_summary(fault,stage,step_id.as_deref(),action);
                original_error=Some(summary.clone());
                let cause=json!({"summary":summary,"build":compact_build(&build),"diagnostic":fault.diagnostic});
                diagnostics.push(cause.clone());
                sink.borrow_mut().json("failure-cause.json","causal-error",&cause)?;
                cause_saved=true;
            }
            let row=json!({"sequence":action,"id":step.id(),"op":stage,"status":if result.is_ok(){"passed"}else{"failed"},
                "wallDurationUs":step_started.elapsed().as_micros(),"readiness":active.readiness(),
                "result":result.as_ref().ok(),"error":result.as_ref().err().map(|fault|&fault.error)});
            sink.borrow_mut().json(&format!("progress/result-{action:06}.json"),"action-result",&row)?;
            steps.push(row);
            result?;
        }
        stage="fixture-finalize";
        step_id=None;
        active.fixtures.finalize()?;
        Ok(())
    })();
    let mut failure=outcome.err();
    if original_error.is_none() && let Some(fault)=failure.as_ref() {
        let summary=cause_summary(fault,stage,step_id.as_deref(),action);
        original_error=Some(summary.clone());
        if let Some(output)=&output {
            let publication=output.borrow_mut().json("failure-cause.json","causal-error",
                &json!({"summary":summary,"build":compact_build(&build),"diagnostic":fault.diagnostic}));
            match publication { Ok(())=>cause_saved=true, Err(error)=>failure=Some(error.into()) }
        }
    }
    let mut failure_evidence=json!({"available":false});
    let mut teardown=json!({"verified":true,"clean":true,"sessionCreated":false});
    let frames=driver.as_ref().map(Driver::frames).unwrap_or(0);
    let generation=driver.as_ref().map(Driver::generation).unwrap_or(1);
    if let Some(active)=driver {
        if failure.is_some() {
            match active.failure_evidence() {
                Ok(evidence)=>failure_evidence=evidence,
                Err(error)=>{ diagnostics.push(json!({"stage":"failure-evidence","error":error})); failure=Some(error.into()); }
            }
        }
        let (cleanup,cleanup_fault)=active.finish();
        teardown=cleanup;
        if let Some(fault)=cleanup_fault {
            diagnostics.push(json!({"stage":"teardown","error":fault.error,"diagnostic":fault.diagnostic}));
            if failure.is_none() { stage="teardown"; failure=Some(fault); }
        }
    }
    if failure.is_some() && failure_evidence["available"]!=true && frames>0 && let Some(output)=&output {
        match recover_failure(&mut output.borrow_mut()) {
            Ok(evidence)=>failure_evidence=evidence,
            Err(error)=> {
                if original_error.is_none() { original_error=failure.as_ref().map(|fault|cause_summary(fault,stage,step_id.as_deref(),action)); }
                diagnostics.push(json!({"stage":"failure-evidence","error":error}));
                failure=Some(error.into());
            }
        }
    }
    if let Some(fault)=failure.as_ref() {
        diagnostics.insert(0,json!({"stage":stage,"step":step_id,"actionSequence":action,"build":build,
            "generation":generation,"error":fault.error,"diagnostic":fault.diagnostic}));
        if !cause_saved && original_error.is_none() && let Some(output)=&output {
            let summary=cause_summary(fault,stage,step_id.as_deref(),action);
            original_error=Some(summary.clone());
            if let Err(error)=output.borrow_mut().json("failure-cause.json","causal-error",
                &json!({"summary":summary,"build":compact_build(&build),"diagnostic":fault.diagnostic})) {
                failure=Some(error.into());
            }
        }
    }
    let mut result=json!({"schemaVersion":1,"command":"run","runId":format!("run-{}",std::process::id()),
        "scenarioId":scenario_id,"generation":generation,"status":if failure.is_none(){"passed"}else{"failed"},
        "exitCode":failure.as_ref().map_or(0,|fault|fault.error.exit_code),"assertions":{"passed":assertions_passed,"failed":assertions_failed},
        "captureCount":captures,"build":compact_build(&build),"timingDeterministic":timing_deterministic,"teardown":teardown,
        "firstError":failure.as_ref().map(|fault|cause_summary(fault,stage,step_id.as_deref(),action)),
        "failureEvidence":failure_evidence,"artifacts":{"manifest":"artifacts.json","environment":"environment.json",
            "diagnostics":"diagnostics.jsonl","steps":"steps.jsonl","metrics":"metrics.json","services":"services.json",
            "latestCapture":latest["png"],"latestTree":latest["tree"]}});
    if let Some(original)=original_error && original!=result["firstError"] { result["originalError"]=original; }
    if let Some(output)=output {
        let publication=(|| -> Result<(),Error> {
            let mut output=output.borrow_mut();
            output.bytes("steps.jsonl","steps",&json_lines(&steps))?;
            output.bytes("diagnostics.jsonl","diagnostics",&json_lines(&diagnostics))?;
            if let Some(fixtures)=&fixture_runtime { output.json("services.json","fixture-services",&fixtures.trace())?; }
            output.json("metrics.json","metrics",&json!({"wallDurationUs":started.elapsed().as_micros(),"nativeFrames":frames,
                "captures":captures,"availability":{"wallDuration":true,"nativeFrames":true,"jsHeap":false,"mutationCost":false,"rss":false}}))?;
            output.seal_worker()
        })();
        if let Err(error)=publication {
            result=publication_failure(error, &result);
        }
    }
    if serde_json::to_vec(&result).expect("result serializes").len()+1>super::MAX_STDOUT_BYTES {
        result=publication_failure(Error::new("E_OUTPUT_LIMIT",8,"worker summary exceeds 8 KiB"), &result);
    }
    supervisor::Report { result,output_state:None }
}

fn compact_build(build:&Value)->Value {
    json!({"buildId":build["buildId"],"sourceMapDigest":build["sourceMapDigest"],"identityArtifact":"inputs/bundle.json"})
}
fn cause_summary(fault:&Fault,stage:&str,step:Option<&str>,action:u64)->Value {
    json!({"code":fault.error.code,"message":fault.error.message.chars().take(768).collect::<String>(),
        "stage":stage,"step":step,"actionSequence":action,"exitCode":fault.error.exit_code,"details":fault.error.details})
}
fn publication_failure(error:Error,previous:&Value)->Value {
    let mut failed=super::failure("run",error);
    failed["originalError"]=if previous["originalError"].is_null() { previous["firstError"].clone() } else { previous["originalError"].clone() };
    failed
}
fn validate_steps(scenario:&scenario::Scenario,profile:&profile::Profile)->Result<(),Error> {
    let mut surface=profile.surface.clone();
    let mut advanced_ms=0_u64;
    for step in &scenario.steps {
        match step {
            Step::Settle{timeout_ms:Some(timeout),..} if *timeout>profile.limits.max_step_wall_ms=>
                return Err(Error::new("E_SCHEMA",2,"settle timeoutMs exceeds the profile's maxStepWallMs")),
            Step::ClockAdvance{by_ms,..}=> {
                advanced_ms=advanced_ms.checked_add(*by_ms).filter(|total|*total<=profile.limits.max_virtual_duration_ms)
                    .ok_or_else(||Error::new("E_SCHEMA",2,"scenario advances beyond maxVirtualDurationMs"))?;
            }
            Step::SurfaceResize{width,height,..}=> { surface.width=*width;surface.height=*height;profile::dimensions(&surface)?; }
            Step::SurfaceScale{scale,..}=> { surface.scale=*scale;profile::dimensions(&surface)?; }
            _=>{}
        }
    }
    Ok(())
}

fn json_lines(rows:&[Value])->Vec<u8> {
    let mut output=Vec::new();
    for row in rows { serde_json::to_writer(&mut output,row).expect("JSON value serializes"); output.push(b'\n'); }
    output
}

/// Called only after the owned worker has stopped. It never loads the application.
fn recover_failure(output:&mut Output)->Result<Value,Error> {
    let directory=output.root().join("evidence");
    let tree_path=output.root().join("failure/tree.json");
    let frame_path=output.root().join("failure/frame.json");
    let existing_tree=tree_path.is_file().then(||read_evidence(&tree_path)).transpose()?;
    let source=if frame_path.is_file() {
        frame_path.clone()
    } else if let Some(tree)=&existing_tree {
        let ordinal=tree["frameOrdinal"].as_u64().ok_or_else(||Error::new("E_OUTPUT_IO",8,"failure tree has no frame identity"))?;
        let generation=tree["generation"].as_u64().ok_or_else(||Error::new("E_OUTPUT_IO",8,"failure tree has no generation"))?;
        directory.join(format!("generation-{generation:010}-frame-{ordinal:010}.frame.json"))
    } else {
        let mut frames=match std::fs::read_dir(&directory) {
            Ok(entries)=>entries.collect::<Result<Vec<_>,_>>().map_err(|error|Error::new("E_OUTPUT_IO",8,error.to_string()))?
                .into_iter().map(|entry|entry.path())
                .filter(|path|path.file_name().and_then(|name|name.to_str()).is_some_and(|name|name.starts_with("generation-")&&name.ends_with(".frame.json"))).collect::<Vec<_>>(),
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(json!({"available":false})),
            Err(error)=>return Err(Error::new("E_OUTPUT_IO",8,error.to_string())),
        };
        frames.sort();
        let Some(path)=frames.pop() else { return Ok(json!({"available":false})); };
        path
    };
    let mut frame=read_evidence(&source)?;
    let tree=match existing_tree {
        Some(tree)=>tree,
        None=>read_evidence(&source.with_file_name(source.file_name().expect("frame file").to_string_lossy().replace(".frame.json",".tree.json")))?,
    };
    if frame["generation"]!=tree["generation"] || frame["validationFrame"]["ordinal"]!=tree["frameOrdinal"] || frame["buildId"]!=tree["buildId"] {
        return Err(Error::new("E_OUTPUT_IO",8,"recovered frame and tree identities disagree"));
    }
    let mut pixels=output.root().join("failure/last-valid.png").is_file();
    if !pixels && !frame_path.is_file() {
        match std::fs::read_dir(output.root().join("captures")) {
            Ok(entries)=>for entry in entries {
                let path=entry.map_err(|error|Error::new("E_OUTPUT_IO",8,error.to_string()))?.path();
                if !path.file_name().and_then(|name|name.to_str()).is_some_and(|name|name.ends_with(".frame.json")) { continue; }
                let capture=read_evidence(&path)?;
                if capture["generation"]!=frame["generation"] || capture["buildId"]!=frame["buildId"] || capture["pixelPresentation"]!=frame["pixelPresentation"] { continue; }
                let png=path.with_file_name(path.file_name().expect("capture file").to_string_lossy().replace(".frame.json",".png"));
                let bytes=std::fs::read(png).map_err(|error|Error::new("E_OUTPUT_IO",8,error.to_string()))?;
                if capture["pngDigest"]!=super::output::digest(&bytes) { return Err(Error::new("E_OUTPUT_IO",8,"historical pixel identity mismatch")); }
                output.bytes("failure/last-valid.png","historical-pixels",&bytes)?;
                pixels=true;
                break;
            },
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
            Err(error)=>return Err(Error::new("E_OUTPUT_IO",8,error.to_string())),
        }
    }
    frame["historical"]=json!(true);
    frame["satisfiesFailureRevision"]=Value::Null;
    frame["pixelsAvailable"]=json!(pixels);
    if !tree_path.is_file() { output.json("failure/tree.json","historical-tree",&tree)?; }
    if !frame_path.is_file() { output.json("failure/frame.json","historical-frame",&frame)?; }
    Ok(json!({"available":true,"historical":true,"tree":"failure/tree.json","frame":"failure/frame.json",
        "pixels":pixels.then_some("failure/last-valid.png")}))
}
fn read_evidence(path:&Path)->Result<Value,Error> {
    let metadata=std::fs::metadata(path).map_err(|error|Error::new("E_OUTPUT_IO",8,error.to_string()))?;
    if metadata.len()>8*1024*1024 { return Err(Error::new("E_OUTPUT_LIMIT",8,"recovered evidence exceeds 8 MiB")); }
    let bytes=std::fs::read(path).map_err(|error|Error::new("E_OUTPUT_IO",8,error.to_string()))?;
    serde_json::from_slice(&bytes).map_err(|error|Error::new("E_OUTPUT_IO",8,error.to_string()))
}

#[cfg(test)]
mod tests;
