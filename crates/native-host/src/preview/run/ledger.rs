//! Ordered record of one run's attempted actions, captures, assertions and
//! failures. The earliest cause stays identifiable when later evidence
//! publication or teardown also fails; publication failures preserve the
//! earliest originalError.
use std::time::Instant;
use serde_json::{Value,json};
use crate::{assets::fixture::FixtureRuntime, session::RuntimeTelemetry};
use super::super::{Error,driver::{Driver,Fault,SharedOutput},evidence,scenario::{PointerOperation,Step}};

/// Native side of a step: arm the deadline, report pre-action readiness, execute
/// exactly one operation. Tests substitute a deterministic driver.
pub(super) trait StepDriver {
    fn begin_step(&mut self,action:u64,step:&Step)->Value;
    fn execute(&mut self,step:&Step)->Result<Value,Fault>;
    fn readiness(&self)->Value;
}

impl StepDriver for Driver {
    fn begin_step(&mut self,action:u64,step:&Step)->Value {
        Driver::begin_step(self,action,match step { Step::Settle{timeout_ms,..}=>*timeout_ms,_=>None });
        self.readiness()
    }
    fn execute(&mut self,step:&Step)->Result<Value,Fault> {
        match step {
            Step::Settle{..}=>self.settle(false),
            Step::Capture{capture_id,surface,required_frame_time_us,allowed_pending_assets,..}=>
                self.capture(capture_id,surface,*required_frame_time_us,allowed_pending_assets),
            Step::PointerMove{location,pointer,..}=>self.pointer(location,pointer,PointerOperation::Move),
            Step::PointerClick{location,button,pointer,..}=>self.pointer(location,pointer,PointerOperation::Click(*button)),
            Step::PointerDown{location,button,pointer,..}=>self.pointer(location,pointer,PointerOperation::Down(*button)),
            Step::PointerUp{location,button,pointer,..}=>self.pointer(location,pointer,PointerOperation::Up(*button)),
            Step::PointerCancel{location,pointer,..}=>self.pointer(location,pointer,PointerOperation::Cancel),
            Step::Wheel{location,delta,phase,..}=>self.wheel(location,delta,*phase),
            Step::DragEnter{location,paths,..}=>self.drag_enter(location,paths),
            Step::DragMove{location,..}=>self.drag_move(location),
            Step::DragLeave{surface,..}=>self.drag_leave(surface),
            Step::DragDrop{location,paths,..}=>self.drag_drop(location,paths),
            Step::KeyPress{surface,key,modifiers,..}=>self.key(surface,key,modifiers).map(|mut trace| { trace["surface"]=json!(surface); trace }),
            Step::TextCommit{surface,text,..}=>self.text(surface,text).map(|mut trace| { trace["surface"]=json!(surface); trace }),
            Step::Assert{target,check,..}=>self.assert_node(target,check),
            Step::ClockAdvance{by_ms,..}=>self.advance(*by_ms),
            Step::ServiceEmit{channel,value,..}=>self.emit(channel,value),
            Step::AssertService{check,..}=>self.assert_service(check),
            Step::SurfaceResize{surface,width,height,..}=>self.resize(surface,*width,*height).map(|mut trace| { trace["surface"]=json!(surface); trace }),
            Step::SurfaceScale{surface,scale,..}=>self.scale(surface,*scale).map(|mut trace| { trace["surface"]=json!(surface); trace }),
            Step::SurfaceColorScheme{surface,color_scheme,..}=>self.color_scheme(surface,color_scheme).map(|mut trace| { trace["surface"]=json!(surface); trace }),
            Step::SurfaceRecreate{surface,..}=>self.recreate(surface).map(|mut trace| { trace["surface"]=json!(surface); trace }),
            Step::SurfaceCloseRequest{surface,..}=>self.close_request(surface),
            Step::MonitorsSet{monitors,..}=>self.set_monitors(monitors),
            Step::PopupDone{surface,..}=>self.popup_done(surface),
            Step::SetBackgroundBlurSupported{supported,..}=>self.set_background_blur_supported(*supported),
            Step::LifecycleReload{bundle,expect,..}=>self.lifecycle_reload(bundle.as_deref(),*expect),
        }
    }
    fn readiness(&self)->Value { Driver::readiness(self) }
}

fn compact_build(build:&Value)->Value {
    json!({"buildId":build["buildId"],"sourceMapDigest":build["sourceMapDigest"],"identityArtifact":"inputs/bundle.json"})
}

fn json_lines(rows:&[Value])->Vec<u8> {
    let mut output=Vec::new();
    for row in rows { serde_json::to_writer(&mut output,row).expect("JSON value serializes"); output.push(b'\n'); }
    output
}

pub(super) struct RunLedger {
    start:Instant,
    stage:&'static str,
    step:Option<String>,
    action:u64,
    build:Value,
    scenario_id:Value,
    timing_deterministic:bool,
    steps:Vec<Value>,
    diagnostics:Vec<Value>,
    captures:u64,
    latest:Option<evidence::CapturePaths>,
    assertions_passed:u64,
    assertions_failed:u64,
    cause_write_failed:bool,
    failure:Option<Fault>,
    original_error:Option<Value>,
    failure_evidence:Value,
    teardown:Value,
    frames:u64,
    generation:u64,
    telemetry:Option<RuntimeTelemetry>,
}

impl RunLedger {
    pub fn new(start:Instant)->Self {
        Self { start,stage:"preflight",step:None,action:0,build:Value::Null,scenario_id:Value::Null,
            timing_deterministic:false,steps:Vec::new(),diagnostics:Vec::new(),captures:0,latest:None,
            assertions_passed:0,assertions_failed:0,cause_write_failed:false,
            failure:None,original_error:None,failure_evidence:json!({"available":false}),
            teardown:json!({"verified":true,"clean":true,"sessionCreated":false}),frames:0,generation:1,telemetry:None }
    }

    /// Build identity is tracked as soon as it is loaded, even when a later
    /// artifact publication fails before the run can finish.
    pub fn identity(&mut self,build:Value,scenario_id:Value,timing_deterministic:bool) {
        self.build=build;
        self.scenario_id=scenario_id;
        self.timing_deterministic=timing_deterministic;
    }

    pub fn enter(&mut self,stage:&'static str) { self.stage=stage; self.step=None; }
    pub fn has_failed(&self)->bool { self.failure.is_some() }
    pub fn frames(&self)->u64 { self.frames }
    pub fn evidence_missing(&self)->bool { self.failure_evidence["available"]!=true }

    fn cause_summary(&self,fault:&Fault)->Value {
        json!({"code":fault.error.code,"message":fault.error.message.chars().take(768).collect::<String>(),
            "stage":self.stage,"step":self.step,"actionSequence":self.action,"exitCode":fault.error.exit_code,"details":fault.error.details})
    }

    /// Dispatch one scenario's steps. The first fault is frozen before result
    /// publication so steps.jsonl, progress files and failure-cause.json never
    /// disagree; the run stops after the first failed operation. Actions are
    /// strictly monotonic across calls; a terminal ledger rejects more steps.
    pub fn execute(&mut self,steps:&[Step],driver:&mut impl StepDriver,output:&SharedOutput)->Result<(),Fault> {
        if let Some(fault)=&self.failure { return Err(fault.clone()); }
        for step in steps {
            self.action+=1;
            self.step=Some(step.id().to_owned());
            self.stage=step.op();
            let progress=driver.begin_step(self.action,step);
            let published={
                let mut held=output.borrow_mut();
                held.json(&format!("progress/action-{:06}.json",self.action),"action-progress",
                    &json!({"stage":self.stage,"step":step.id(),"actionSequence":self.action,"build":self.build,"readiness":progress}))
            };
            if let Err(error)=published {
                self.fail(error.into(),Some(output));
                return Err(self.failure.clone().expect("recorded"));
            }
            let step_started=Instant::now();
            let result=driver.execute(step);
            // Attempted assertions and successful captures are counted from the
            // step variant and actual result only, exactly once per action.
            match step {
                Step::Assert{..}|Step::AssertService{..} => {
                    if result.is_ok() { self.assertions_passed+=1; } else { self.assertions_failed+=1; }
                }
                Step::Capture{capture_id,surface,..} if result.is_ok() => {
                    self.captures+=1; self.latest=evidence::CapturePaths::new(capture_id,surface).ok();
                }
                _=>{}
            }
            if let Err(fault)=&result {
                self.fail(fault.clone(),Some(output));
                // The cause could not be persisted: the publication fault is
                // terminal and no completed action result exists for this step.
                if self.cause_write_failed { return Err(self.failure.clone().expect("publication fault recorded")); }
            }
            let row=json!({"sequence":self.action,"id":step.id(),"op":self.stage,"status":if result.is_ok(){"passed"}else{"failed"},
                "wallDurationUs":step_started.elapsed().as_micros(),"readiness":driver.readiness(),
                "result":result.as_ref().ok(),"error":result.as_ref().err().map(|fault|&fault.error)});
            let published={
                let mut held=output.borrow_mut();
                held.json(&format!("progress/result-{:06}.json",self.action),"action-result",&row)
            };
            if let Err(error)=published {
                // The application cause was already persisted; the publication
                // failure becomes terminal without erasing originalError.
                self.publication_failed("action-result",error,Some(output));
                return Err(self.failure.clone().expect("recorded"));
            }
            self.steps.push(row);
            result?;
        }
        Ok(())
    }

    /// Record the primary cause once and attempt failure-cause.json before any
    /// cleanup or readback. A failed cause publication becomes terminal while
    /// the original cause stays in originalError; later calls cannot overwrite.
    pub fn fail(&mut self,fault:Fault,output:Option<&SharedOutput>) {
        if self.original_error.is_none() {
            let summary=self.cause_summary(&fault);
            self.original_error=Some(summary.clone());
            let cause=json!({"summary":summary,"build":compact_build(&self.build),"diagnostic":fault.diagnostic});
            self.diagnostics.push(cause.clone());
            if let Some(output)=output
                && let Err(error)=output.borrow_mut().json("failure-cause.json","causal-error",&cause) {
                self.cause_write_failed=true;
                self.failure=Some(error.into());
                return;
            }
        }
        if self.failure.is_none() { self.failure=Some(fault); }
    }

    /// A later evidence or output publication failure. It may override the
    /// terminal fault but never replaces the earliest originalError; when no
    /// cause was recorded yet, the publication error itself becomes it.
    pub fn publication_failed(&mut self,stage:&'static str,error:Error,output:Option<&SharedOutput>) {
        self.fail(error.clone().into(),output);
        self.diagnostics.push(json!({"stage":stage,"error":error}));
        self.failure=Some(error.into());
    }

    /// Teardown contributes the first failure only when nothing failed before;
    /// it never replaces a recorded application cause.
    pub fn cleanup(&mut self,teardown:Value,fault:Option<Fault>,output:Option<&SharedOutput>) {
        self.teardown=teardown;
        if let Some(fault)=fault {
            self.diagnostics.push(json!({"stage":"teardown","error":fault.error,"diagnostic":fault.diagnostic}));
            if self.failure.is_none() { self.enter("teardown"); self.fail(fault,output); }
        }
    }

    pub fn record_evidence(&mut self,failure_evidence:Value) { self.failure_evidence=failure_evidence; }

    pub fn observations(&mut self,generation:u64,frames:u64,telemetry:Option<RuntimeTelemetry>,failure_evidence:Value) {
        self.generation=generation;
        self.frames=frames;
        self.telemetry=telemetry;
        if !failure_evidence.is_null() { self.failure_evidence=failure_evidence; }
    }

    /// Final summary, bounded diagnostics/services/metrics writes and worker
    /// sealing. Any publication failure preserves the earliest originalError.
    pub fn finish(mut self,output:Option<&SharedOutput>,fixtures:Option<&FixtureRuntime>)->Value {
        if let Some(fault)=self.failure.clone() {
            self.diagnostics.insert(0,json!({"stage":self.stage,"step":self.step,"actionSequence":self.action,
                "build":self.build,"generation":self.generation,"error":fault.error,"diagnostic":fault.diagnostic}));
        }
        let failure=self.failure.clone();
        let mut result=json!({"schemaVersion":1,"command":"run","runId":format!("run-{}",std::process::id()),
            "scenarioId":self.scenario_id,"generation":self.generation,"status":if failure.is_none(){"passed"}else{"failed"},
            "exitCode":failure.as_ref().map_or(0,|fault|fault.error.exit_code),
            "assertions":{"passed":self.assertions_passed,"failed":self.assertions_failed},
            "captureCount":self.captures,"build":compact_build(&self.build),"timingDeterministic":self.timing_deterministic,
            "teardown":self.teardown,
            "firstError":failure.as_ref().map(|fault|self.cause_summary(fault)),
            "failureEvidence":self.failure_evidence,
            "artifacts":{"manifest":"artifacts.json","environment":"environment.json",
                "diagnostics":"diagnostics.jsonl","steps":"steps.jsonl","metrics":"metrics.json","services":"services.json",
                "latestCapture":self.latest.as_ref().map(|paths|paths.png()),
                "latestTree":self.latest.as_ref().map(|paths|paths.tree())}});
        if let Some(original)=&self.original_error && *original!=result["firstError"] { result["originalError"]=original.clone(); }
        if let Some(output)=output {
            let diagnostics=self.diagnostics.clone();
            let steps=self.steps.clone();
            let started=self.start;
            let frames=self.frames;
            let captures=self.captures;
            let telemetry=self.telemetry;
            let publication=(|| -> Result<(),Error> {
                let mut held=output.borrow_mut();
                held.bytes("steps.jsonl","steps",&json_lines(&steps))?;
                held.bytes("diagnostics.jsonl","diagnostics",&json_lines(&diagnostics))?;
                if let Some(fixtures)=fixtures { held.json("services.json","fixture-services",&fixtures.trace())?; }
                let process_memory = telemetry.and_then(|item| item.process_memory);
                held.json("metrics.json","metrics",&json!({
                    "wallDurationUs":started.elapsed().as_micros(),
                    "nativeFrames":frames,
                    "captures":captures,
                    "maxJsEntryDurationUs":telemetry.map(|item|item.max_entry_duration.as_micros()),
                    "jsHeapBytes":telemetry.map(|item|item.js_heap_bytes),
                    "jsHeapLiveBytes":telemetry.map(|item|item.js_heap_live_bytes),
                    "jsHeapPostDisposeBytes":telemetry.map(|item|item.js_heap_post_dispose_bytes),
                    "jsStackLimitBytes":telemetry.map(|item|item.js_stack_limit_bytes),
                    "rssBytes":process_memory.map(|item|item.rss_bytes),
                    "pssBytes":process_memory.map(|item|item.pss_bytes),
                    "pssAnonBytes":process_memory.map(|item|item.pss_anon_bytes),
                    "pssFileBytes":process_memory.map(|item|item.pss_file_bytes),
                    "privateDirtyBytes":process_memory.map(|item|item.private_dirty_bytes),
                    "availability":{
                        "wallDuration":true,
                        "nativeFrames":true,
                        "jsHeap":telemetry.is_some(),
                        "jsEntryDuration":telemetry.is_some(),
                        "mutationCost":false,
                        "rss":process_memory.is_some()
                    }
                }))?;
                held.seal_worker()
            })();
            if let Err(error)=publication {
                result=super::super::output::publication_failure("run",error,&result);
            }
        }
        super::super::output::enforce_stdout_limit("run",result)
    }
}
