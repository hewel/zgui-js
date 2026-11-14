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

#[test]
fn recovery_completes_a_half_published_failure_without_overwriting_its_tree() {
    let directory=Directory::new();
    let mut output=Output::create(&directory.0,1024*1024).unwrap();
    let tree=json!({"generation":1,"frameOrdinal":1,"buildId":"build-one","nodes":[{"text":"last complete state"}]});
    let frame=json!({"generation":1,"validationFrame":{"ordinal":1},"buildId":"build-one","pixelPresentation":{"sequence":1}});
    output.json("evidence/generation-0000000001-frame-0000000001.tree.json","validation-tree",&tree).unwrap();
    output.json("evidence/generation-0000000001-frame-0000000001.frame.json","validation-frame",&frame).unwrap();
    output.json("failure/tree.json","failure-tree",&tree).unwrap();
    let before=std::fs::read(directory.0.join("failure/tree.json")).unwrap();
    // A recreated surface may restart its ordinal; it cannot replace the failure's selected generation.
    output.json("evidence/generation-0000000002-frame-0000000001.tree.json","validation-tree",&json!({"generation":2,"frameOrdinal":1,"buildId":"build-one"})).unwrap();
    output.json("evidence/generation-0000000002-frame-0000000001.frame.json","validation-frame",&json!({"generation":2,"validationFrame":{"ordinal":1},"buildId":"build-one"})).unwrap();
    let result=recover_failure(&mut output).unwrap();
    assert_eq!(result["available"],true);
    assert_eq!(std::fs::read(directory.0.join("failure/tree.json")).unwrap(),before);
    let saved=read_evidence(&directory.0.join("failure/frame.json")).unwrap();
    assert_eq!(saved["validationFrame"]["ordinal"],1);
    assert_eq!(saved["generation"],1);
    assert_eq!(saved["historical"],true);
    assert!(saved["satisfiesFailureRevision"].is_null());
    assert_eq!(recover_failure(&mut output).unwrap(),result);
}
