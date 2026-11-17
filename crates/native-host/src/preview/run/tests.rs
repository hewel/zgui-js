use super::*;
use std::sync::atomic::{AtomicU64,Ordering};

struct Directory(PathBuf);
impl Directory {
    fn new()->Self {
        static NEXT:AtomicU64=AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!("zgui-preview-recovery-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed))))
    }
}
impl Drop for Directory { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.0); } }

#[test]
fn successive_publication_failures_retain_the_application_cause() {
    let cause=json!({"code":"E_JS_EXCEPTION","message":"original mount failure","stage":"mount","exitCode":3});
    let worker=publication_failure(Error::new("E_OUTPUT_LIMIT",8,"diagnostic budget"),&json!({"firstError":cause}));
    let controller=publication_failure(Error::new("E_OUTPUT_IO",8,"missing worker inventory"),&worker);
    let final_result=publication_failure(Error::new("E_OUTPUT_LIMIT",8,"final result budget"),&controller);
    assert_eq!(final_result["originalError"],cause);
    assert_eq!(final_result["exitCode"],8);
}

#[test]
fn sealed_inventory_rejects_modified_worker_bytes() {
    let directory=Directory::new();
    let mut output=Output::create(&directory.0,1024*1024).unwrap();
    output.bytes("capture.bin","capture",b"original").unwrap();
    output.seal_worker().unwrap();
    output.restore_worker().unwrap();
    std::fs::write(directory.0.join("capture.bin"),b"tampered").unwrap();
    assert_eq!(output.restore_worker().unwrap_err().code,"E_OUTPUT_IO");
}

struct Scripted { executed:Vec<String>, failure_step:Option<String> }
impl ledger::StepDriver for Scripted {
    fn begin_step(&mut self,_action:u64,_step:&Step)->Value { json!({"timeUs":0}) }
    fn readiness(&self)->Value { json!({"timeUs":0}) }
    fn execute(&mut self,step:&Step)->Result<Value,Fault> {
        self.executed.push(step.id().to_owned());
        if self.failure_step.as_deref()==Some(step.id()) {
            return Err(Error::new("E_ASSERTION",4,"assertion failed").into());
        }
        Ok(json!({"ok":true}))
    }
}

fn settle_step(id:&str)->Step { Step::Settle{id:id.to_owned(),timeout_ms:None} }
fn capture_step(id:&str)->Step { Step::Capture{id:id.to_owned(),capture_id:id.to_owned(),required_frame_time_us:None,allowed_pending_assets:Vec::new()} }
fn assert_step(id:&str)->Step { Step::Assert{id:id.to_owned(),
    target:scenario::Target{surface:"main".into(),selector:scenario::Selector::TestId("x".into())},
    check:scenario::Check::Exists(true)} }

#[test]
fn ledger_stops_after_the_first_failed_step_and_keeps_monotonic_actions() {
    let directory=Directory::new();
    let output=Rc::new(RefCell::new(Output::create(&directory.0,1024*1024).unwrap()));
    let mut ledger=RunLedger::new(Instant::now());
    ledger.identity(json!({"buildId":"build-one"}),json!("scenario-one"),true);
    let mut driver=Scripted{executed:Vec::new(),failure_step:Some("b".into())};
    let steps=vec![settle_step("a"),assert_step("b"),capture_step("c")];
    let fault=ledger.execute(&steps,&mut driver,&output).unwrap_err();
    assert_eq!(fault.error.code,"E_ASSERTION");
    assert_eq!(driver.executed,vec!["a","b"]);
    assert!(ledger.has_failed());
    // A second dispatch is rejected without attempting another action.
    assert!(ledger.execute(&[settle_step("d")],&mut driver,&output).is_err());
    assert_eq!(driver.executed,vec!["a","b"]);
    let result=ledger.finish(Some(&output),None);
    assert_eq!(result["status"],"failed");
    assert_eq!(result["firstError"]["code"],"E_ASSERTION");
    assert_eq!(result["firstError"]["stage"],"assert");
    assert_eq!(result["firstError"]["step"],"b");
    assert_eq!(result["firstError"]["actionSequence"],2);
    assert_eq!(result["assertions"]["failed"],1);
    assert_eq!(result["captureCount"],0);
    assert!(directory.0.join("progress/result-000002.json").is_file());
    assert!(!directory.0.join("progress/result-000003.json").exists());
    assert!(directory.0.join("failure-cause.json").is_file());
}

#[test]
fn ledger_counts_assertions_and_captures_exactly_once() {
    let directory=Directory::new();
    let output=Rc::new(RefCell::new(Output::create(&directory.0,1024*1024).unwrap()));
    let mut ledger=RunLedger::new(Instant::now());
    let mut driver=Scripted{executed:Vec::new(),failure_step:Some("bad-capture".into())};
    let steps=vec![assert_step("one"),capture_step("good"),assert_step("two"),capture_step("bad-capture"),assert_step("three")];
    assert!(ledger.execute(&steps,&mut driver,&output).is_err());
    let result=ledger.finish(Some(&output),None);
    assert_eq!(result["assertions"],json!({"passed":2,"failed":0}));
    // The failed capture executed but was never published, so it is not counted.
    assert_eq!(result["captureCount"],1);
    // Artifact paths come from the step's capture ID, not driver payload fields.
    assert_eq!(result["artifacts"]["latestCapture"],"captures/good.png");
    assert_eq!(result["artifacts"]["latestTree"],"captures/good.tree.json");
}

#[test]
fn ledger_result_publication_collision_preserves_application_cause() {
    let directory=Directory::new();
    let output=Rc::new(RefCell::new(Output::create(&directory.0,1024*1024).unwrap()));
    // A row already occupying the result path collides with this action's write.
    output.borrow_mut().json("progress/result-000001.json","action-result",&json!({"earlier":true})).unwrap();
    let mut ledger=RunLedger::new(Instant::now());
    ledger.identity(json!({"buildId":"build-one"}),json!("scenario-one"),true);
    let mut driver=Scripted{executed:Vec::new(),failure_step:Some("b".into())};
    let fault=ledger.execute(&[assert_step("b")],&mut driver,&output).unwrap_err();
    assert_eq!(fault.error.code,"E_OUTPUT_IO");
    let cause=std::fs::read(directory.0.join("failure-cause.json")).unwrap();
    let result=ledger.finish(Some(&output),None);
    assert_eq!(result["exitCode"],8);
    assert_eq!(result["firstError"]["code"],"E_OUTPUT_IO");
    assert_eq!(result["originalError"]["code"],"E_ASSERTION");
    // The publication failure did not replace the persisted application cause.
    assert_eq!(std::fs::read(directory.0.join("failure-cause.json")).unwrap(),cause);
}

#[test]
fn ledger_successful_step_publication_collision_persists_cause_before_cleanup() {
    let directory=Directory::new();
    let output=Rc::new(RefCell::new(Output::create(&directory.0,1024*1024).unwrap()));
    output.borrow_mut().json("progress/result-000001.json","action-result",&json!({"earlier":true})).unwrap();
    let mut ledger=RunLedger::new(Instant::now());
    let mut driver=Scripted{executed:Vec::new(),failure_step:None};
    // The step succeeded, but its result row cannot be published.
    let fault=ledger.execute(&[settle_step("a"),settle_step("b")],&mut driver,&output).unwrap_err();
    assert_eq!(fault.error.code,"E_OUTPUT_IO");
    // The first publication cause is on disk before any cleanup can run.
    assert_eq!(driver.executed,vec!["a"]);
    let cause:Value=serde_json::from_slice(&std::fs::read(directory.0.join("failure-cause.json")).unwrap()).unwrap();
    assert_eq!(cause["summary"]["code"],"E_OUTPUT_IO");
    let before=std::fs::read(directory.0.join("failure-cause.json")).unwrap();
    ledger.cleanup(json!({"verified":false,"clean":false}),Some(Error::new("E_TEARDOWN",8,"teardown also failed").into()),Some(&output));
    assert_eq!(std::fs::read(directory.0.join("failure-cause.json")).unwrap(),before);
    let result=ledger.finish(Some(&output),None);
    assert_eq!(result["exitCode"],8);
    assert_eq!(result["firstError"]["code"],"E_OUTPUT_IO");
}

#[test]
fn ledger_teardown_cause_survives_later_evidence_publication_failure() {
    let directory=Directory::new();
    let output=Rc::new(RefCell::new(Output::create(&directory.0,1024*1024).unwrap()));
    let mut ledger=RunLedger::new(Instant::now());
    let mut driver=Scripted{executed:Vec::new(),failure_step:None};
    assert!(ledger.execute(&[assert_step("a")],&mut driver,&output).is_ok());
    ledger.enter("fixture-finalize");
    ledger.cleanup(json!({"verified":false,"clean":false}),Some(Error::new("E_TEARDOWN",8,"teardown failed").into()),Some(&output));
    assert!(directory.0.join("failure-cause.json").is_file());
    ledger.publication_failed("failure-evidence",Error::new("E_OUTPUT_IO",8,"cannot read evidence"),Some(&output));
    let result=ledger.finish(Some(&output),None);
    assert_eq!(result["exitCode"],8);
    assert_eq!(result["firstError"]["code"],"E_OUTPUT_IO");
    assert_eq!(result["originalError"]["code"],"E_TEARDOWN");
    assert_eq!(result["originalError"]["stage"],"teardown");
    assert!(result["originalError"]["step"].is_null());
}

#[test]
fn ledger_teardown_fault_cannot_replace_an_application_cause() {
    let directory=Directory::new();
    let output=Rc::new(RefCell::new(Output::create(&directory.0,1024*1024).unwrap()));
    let mut ledger=RunLedger::new(Instant::now());
    let mut driver=Scripted{executed:Vec::new(),failure_step:Some("b".into())};
    assert!(ledger.execute(&[assert_step("b")],&mut driver,&output).is_err());
    ledger.cleanup(json!({"verified":false,"clean":false}),Some(Error::new("E_TEARDOWN",8,"teardown also failed").into()),Some(&output));
    let result=ledger.finish(Some(&output),None);
    assert_eq!(result["firstError"]["code"],"E_ASSERTION");
    assert!(result["originalError"].is_null());
}

#[test]
fn ledger_exhausted_output_keeps_the_earliest_cause_and_exit_8() {
    let directory=Directory::new();
    // The budget admits the ownership claim but fails every later publication.
    let output=Rc::new(RefCell::new(Output::create(&directory.0,16).unwrap()));
    let mut ledger=RunLedger::new(Instant::now());
    ledger.identity(json!({"buildId":"build-one"}),json!("scenario-one"),true);
    let mut driver=Scripted{executed:Vec::new(),failure_step:Some("b".into())};
    assert!(ledger.execute(&[settle_step("a"),assert_step("b")],&mut driver,&output).is_err());
    // The first progress publication already failed; no step executed.
    assert!(driver.executed.is_empty());
    assert!(ledger.has_failed());
    // A mount sentinel arriving while output is exhausted still survives as
    // the original cause behind the exit-8 publication failure.
    let mut mount=RunLedger::new(Instant::now());
    mount.fail(Error::new("E_JS_EXCEPTION",3,"mount sentinel").into(),Some(&output));
    let result=mount.finish(Some(&output),None);
    assert_eq!(result["status"],"failed");
    assert_eq!(result["exitCode"],8);
    assert_eq!(result["originalError"]["code"],"E_JS_EXCEPTION");
}
