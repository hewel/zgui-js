mod ledger;

use std::{cell::RefCell, path::PathBuf, rc::Rc, time::{Duration, Instant}};
use serde_json::{Value, json};
use crate::package::Package;
use ledger::RunLedger;
use super::{Error, driver::{Driver, Fault, SharedOutput}, evidence, fixture, inventory, output::{self, Output}, profile, scenario::{self, Step}, supervisor};

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

pub(super) struct ValidatedRun {
    pub(super) options: Options,
    pub(super) profile: profile::Profile,
    pub(super) total_steps: usize,
}

pub(super) fn validate(args: &[String]) -> Result<ValidatedRun, Error> {
    let options=options(args.iter().cloned())?;
    let (_,profile)=profile::read_profile(&options.common.profile,&options.common.overrides)?;
    // Capability/schema errors cannot execute the application, allocate a GPU, or claim a run.
    let scenario = scenario::load(&options.scenario,super::RUN_CAPABILITIES,profile.clock.is_virtual())?;
    validate_steps(&scenario.scenario, &profile)?;
    fixture::load(&options.fixture)?;
    let total_steps = scenario.scenario.steps.len();
    Ok(ValidatedRun { options, profile, total_steps })
}

pub fn command(args: Vec<String>) -> Result<Value,Error> {
    controlled(args, None)
}

#[derive(Debug)]
struct WatchdogProgress {
    total_steps: usize,
    completed_steps: u64,
    last_completed: Option<Value>,
    in_flight: Option<Value>,
    in_flight_label: Option<String>,
    avg_completed_step_ms: Option<u128>,
}

impl WatchdogProgress {
    fn summary_suffix(&self) -> String {
        let in_flight = self.in_flight_label.as_deref().unwrap_or("none");
        format!(
            "completed {}/{} steps; in-flight: {in_flight}",
            self.completed_steps, self.total_steps
        )
    }

    fn to_details(&self) -> Value {
        json!({
            "totalSteps": self.total_steps,
            "completedSteps": self.completed_steps,
            "lastCompletedStep": self.last_completed,
            "inFlightStep": self.in_flight,
            "avgCompletedStepMs": self.avg_completed_step_ms,
        })
    }
}

fn watchdog_progress(root: &std::path::Path, total_steps: usize) -> Option<WatchdogProgress> {
    let progress_dir = root.join("progress");
    let entries = std::fs::read_dir(&progress_dir).ok()?;
    let mut max_result: Option<(u64, Value)> = None;
    let mut max_action: Option<(u64, PathBuf)> = None;
    let mut completed_steps: u64 = 0;
    let mut total_completed_wall_us: u128 = 0;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some(seq_str) = name.strip_prefix("result-").and_then(|s| s.strip_suffix(".json"))
            && let Ok(seq) = seq_str.parse::<u64>()
        {
            completed_steps += 1;
            if let Ok(bytes) = evidence::read_evidence(&entry.path())
                && let Ok(row) = serde_json::from_slice::<Value>(&bytes)
            {
                if let Some(dur) = row["wallDurationUs"].as_u64() {
                    total_completed_wall_us += u128::from(dur);
                }
                if max_result.as_ref().is_none_or(|(prev, _)| seq > *prev) {
                    max_result = Some((seq, row));
                }
            }
        } else if let Some(seq_str) = name.strip_prefix("action-").and_then(|s| s.strip_suffix(".json"))
            && let Ok(seq) = seq_str.parse::<u64>()
            && max_action.as_ref().is_none_or(|(prev, _)| seq > *prev)
        {
            max_action = Some((seq, entry.path()));
        }
    }

    let last_completed_seq = max_result.as_ref().map(|(seq, _)| *seq);
    let last_completed = max_result.map(|(seq, row)| {
        json!({
            "sequence": seq,
            "id": row["id"],
            "op": row["op"],
            "wallDurationUs": row["wallDurationUs"],
        })
    });
    let (in_flight, in_flight_label) = match max_action {
        Some((seq, path)) if !last_completed_seq.is_some_and(|done| seq <= done) => {
            evidence::read_evidence(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .map_or((None, None), |row| {
                    let label = row["step"]
                        .as_str()
                        .or_else(|| row["stage"].as_str())
                        .map(str::to_owned);
                    let value = json!({
                        "sequence": seq,
                        "step": row["step"],
                        "stage": row["stage"],
                    });
                    (Some(value), label)
                })
        }
        _ => (None, None),
    };
    if completed_steps == 0 && in_flight.is_none() {
        return None;
    }
    let avg_completed_step_ms = (completed_steps > 0)
        .then(|| (total_completed_wall_us / u128::from(completed_steps)) / 1000);
    Some(WatchdogProgress {
        total_steps,
        completed_steps,
        last_completed,
        in_flight,
        in_flight_label,
        avg_completed_step_ms,
    })
}

pub(super) fn controlled(args: Vec<String>, canceled: Option<&std::sync::atomic::AtomicBool>) -> Result<Value,Error> {
    let ValidatedRun { options, profile, total_steps } = validate(&args)?;
    let mut output=Output::create(options.common.out.as_deref().expect("required output"),profile.limits.max_output_bytes)?;
    let mut child=std::process::Command::new(std::env::current_exe().map_err(|error|Error::new("E_WORKER_CRASH",9,error.to_string()))?);
    child.args(["preview","_run-worker"]).args(args).env("ZGUI_PREVIEW_PARENT_PID",std::process::id().to_string()).env("TZ","UTC");
    let report=supervisor::controlled(&mut child,Duration::from_millis(profile.limits.max_run_wall_ms),canceled);
    let mut result=match report {
        Ok(report) => {
            if let Err(error)=output.restore_worker() {
                output::publication_failure("run", error, &report.result)
            } else { report.result }
        }
        Err(error) => {
            let is_watchdog = error.code == "E_WATCHDOG";
            let mut failed=super::failure("run",error);
            failed["teardown"]=json!({"verified":false,"clean":null});
            let recovery=(|| {
                output.recover()?;
                if output.root().join("failure-cause.json").is_file() {
                    let cause:Value=serde_json::from_slice(&evidence::read_evidence(&output.root().join("failure-cause.json"))?)
                        .map_err(|error|Error::new("E_OUTPUT_IO",8,error.to_string()))?;
                    failed["terminationError"]=failed["firstError"].clone();
                    failed["firstError"]=cause["summary"].clone();
                    failed["exitCode"]=cause["summary"]["exitCode"].clone();
                    failed["build"]=cause["build"].clone();
                } else if is_watchdog
                    && let Some(progress) = watchdog_progress(output.root(), total_steps)
                {
                    if let Some(msg) = failed["firstError"]["message"].as_str().map(str::to_owned) {
                        failed["firstError"]["message"] = json!(format!(
                            "{msg} ({})",
                            progress.summary_suffix()
                        ));
                    }
                    failed["firstError"]["details"] = progress.to_details();
                }
                evidence::recover(&mut output)
            })();
            match recovery {
                Ok(evidence) => failed["failureEvidence"]=evidence,
                Err(error) => failed=output::publication_failure("run", error, &failed),
            }
            failed
        }
    };
    if result["firstError"]["code"]=="E_RUN_CANCELED" || result["terminationError"]["code"]=="E_RUN_CANCELED" {
        result["cancellation"]=json!({"requested":true,"workerStopped":true,"cleanTeardownVerified":false});
        if result["firstError"]["code"]=="E_RUN_CANCELED" { result["status"]=json!("canceled"); }
    }
    Ok(output.finalize("run", result))
}

pub fn worker(args: impl Iterator<Item=String>) -> supervisor::Report {
    let started=Instant::now();
    let mut ledger=RunLedger::new(started);
    let mut output:Option<SharedOutput>=None;
    let mut resolved_environment = None;
    let mut driver=None;
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
        ledger.identity(Value::Null,json!(scenario.scenario.id),profile.profile.clock.is_virtual());
        sink.borrow_mut().json("inputs/profile.json","resolved-profile",&profile.resolved_json)?;
        sink.borrow_mut().bytes("inputs/fixture.json","fixture",&fixture.original)?;
        sink.borrow_mut().bytes("inputs/scenario.json","scenario",&scenario.original)?;
        let package=Rc::new(Package::load(&options.bundle).map_err(|error|Error::new("E_BUNDLE",3,error.to_string()))?);
        let mut build=json!({"id":package.manifest.id,"buildId":package.manifest.build_id,"sourceMap":package.manifest.source_map,
            "sourceMapDigest":package.source_map_digest,"entryDigest":package.entry_digest,
            "styleSystem":package.manifest.style_system,"styles":package.manifest.styles,
            "fonts":package.manifest.fonts,"artifactDigests":package.manifest.artifact_digests});
        if !package.manifest.capabilities.is_empty() {
            build["capabilities"] = serde_json::to_value(&package.manifest.capabilities)
                .map_err(|error| Error::new("E_BUNDLE", 3, error.to_string()))?;
        }
        ledger.identity(build.clone(),json!(scenario.scenario.id),profile.profile.clock.is_virtual());
        sink.borrow_mut().json("inputs/bundle.json","bundle-identity",&build)?;
        let build_info = package.read_verified_json("build-info.json")
            .map_err(|error| Error::new("E_BUNDLE", 3, error.to_string()))?;
        sink.borrow_mut().json("inputs/build-info.json","verified-build-info",&build_info)?;
        let root_styles=super::driver::stylesheet(&profile);
        let mut environment=inventory::snapshot()?;
        environment["profileDigest"]=json!(profile.digest);
        environment["fonts"]=profile.font_evidence.clone();
        environment["fixture"]=json!({"id":fixture.id,"digest":fixture.digest,"providerPolicy":"fixture-only"});
        environment["scenario"]=json!({"id":scenario.scenario.id,"digest":scenario.digest,"requiredCapabilities":scenario.scenario.required_capabilities});
        environment["application"]=build.clone();
        environment["capabilityRevision"]=json!("native-preview-v1-p5");
        environment["hostRootStyleDigest"]=json!(output::digest(root_styles.as_bytes()));
        environment["inputPolicy"]=json!({"surfaceFocus":"main","dispatch":"one native deliver_all burst per action; no frame between low-level events","modifiers":"key-local; restored to none after key-up"});
        if package.manifest.capabilities.shell_surfaces {
            environment["inputPolicy"]["surfaceFocus"] = json!("selected-surface");
        }
        environment["timingDeterministic"]=json!(profile.profile.clock.is_virtual());
        environment["clock"]=crate::environment::Environment::configuration(
            match profile.profile.clock { profile::Clock::Virtual{epoch_ms,..}=>Some(epoch_ms),profile::Clock::Wall{}=>None },
            profile.profile.random.seed);
        environment["random"]=environment["clock"]["random"].clone();
        environment["resourcePublication"]=json!("complete-batch-launch-order-v1");
        environment["graphicsEvidence"]=json!("graphics.json");
        resolved_environment = Some(environment);
        ledger.enter("mount");
        sink.borrow_mut().json("progress/action-000000.json","action-progress",&json!({"stage":"mount","actionSequence":0,"build":build}))?;
        let reload = super::driver::ReloadConfig {
            dev_state: scenario.scenario.required_capabilities.iter().any(|cap| cap == "lifecycle.reload"),
            bundle_dir: options.bundle.clone(),
            scenario_dir: options.scenario.parent().map(std::path::Path::to_path_buf).unwrap_or_default(),
        };
        driver=Some(Driver::new(&profile,package,fixture.definition,sink.clone(),started+Duration::from_millis(profile.profile.limits.max_run_wall_ms),root_styles,reload)?);
        let active=driver.as_mut().expect("driver installed");
        fixture_runtime=Some(active.fixtures.clone());
        // A startup mount fault wins over any later graphics publication failure.
        active.check()?;
        if !profile.profile.pure_shell {
            sink.borrow_mut().json("graphics.json","graphics-environment",&active.graphics())?;
        }
        let execution = active.settle(false).and_then(|_| ledger.execute(&scenario.scenario.steps, active, &sink));
        if profile.profile.pure_shell {
            let graphics = sink.borrow_mut().json("graphics.json","graphics-environment",&active.graphics());
            if let Err(error) = graphics {
                if execution.is_ok() { return Err(error.into()); }
                ledger.publication_failed("graphics", error, Some(&sink));
            }
        }
        execution?;
        ledger.enter("fixture-finalize");
        active.fixtures.finalize()?;
        Ok(())
    })();
    let graphics_unavailable = outcome.as_ref().is_err_and(|fault| fault.error.code == "E_GRAPHICS_UNAVAILABLE");
    if let Err(fault)=outcome { ledger.fail(fault,output.as_ref()); }
    // Publish once, after renderer creation (including recreation) can no longer fail.
    if let (Some(mut environment), Some(output)) = (resolved_environment, output.as_ref()) {
        environment["capabilities"] = super::capabilities(!graphics_unavailable);
        let published = output.borrow_mut().json("environment.json", "environment", &environment);
        if let Err(error) = published {
            ledger.publication_failed("environment", error, Some(output));
        }
    }
    if let Some(active)=driver {
        let mut found=json!({"available":false});
        if ledger.has_failed() {
            match active.failure_evidence() {
                Ok(evidence)=>found=evidence,
                Err(error)=>ledger.publication_failed("failure-evidence",error,output.as_ref()),
            }
        }
        let generation=active.generation();
        let frames=active.frames();
        let (teardown,cleanup_fault,telemetry)=active.finish();
        ledger.observations(generation,frames,telemetry,found);
        ledger.cleanup(teardown,cleanup_fault,output.as_ref());
    }
    if ledger.has_failed() && ledger.evidence_missing() && ledger.frames()>0 && let Some(output)=&output {
        let recovered=evidence::recover(&mut output.borrow_mut());
        match recovered {
            Ok(found)=>ledger.record_evidence(found),
            Err(error)=>ledger.publication_failed("failure-evidence",error,Some(output)),
        }
    }
    let result=ledger.finish(output.as_ref(),fixture_runtime.as_deref());
    supervisor::Report { result }
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
            Step::SurfaceResize{surface:name,width,height,..} if name=="main"=> { surface.width = *width; surface.height = *height; profile::dimensions(&surface)?; }
            Step::SurfaceScale{surface:name,scale,..} if name=="main"=> { surface.scale = *scale; profile::dimensions(&surface)?; }
            _=>{}
        }
    }
    Ok(())
}


#[cfg(test)]
mod tests;
