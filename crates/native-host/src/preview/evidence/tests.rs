use super::*;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64,Ordering};

struct Directory(PathBuf);
impl Directory {
    fn new()->Self {
        static NEXT:AtomicU64=AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!("zgui-evidence-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed))))
    }
}
impl Drop for Directory { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.0); } }

fn tree(generation:u64,ordinal:u64)->TreeEvidence {
    serde_json::from_value(json!({"schemaVersion":1,"buildId":"build-one","generation":generation,"surface":"main",
        "frameOrdinal":ordinal,"metadataRevision":2,"presentationSequence":3,
        "resourceRevision":4,"surfaceConfiguration":5,"styleRevision":6,
        "nodes":[{"token":"n0","parent":null,"surface":"main","generation":generation,"kind":"view","ownership":"app",
            "text":"","textTruncated":false,"source":null,"value":null,
            "state":{"enabled":null,"visible":null,"focused":false},
            "boundsLogical":null,"boundsDevice":null,"boundsKind":"none","pointerReachabilityVerified":false}],
        "truncation":{"nodes":0,"metadata":0},"semanticProjection":{"available":true}})).unwrap()
}
fn frame(generation:u64,ordinal:u64)->FrameEvidence {
    serde_json::from_value(json!({"schemaVersion":1,"buildId":"build-one","generation":generation,"surface":"main",
        "metadataRevision":2,"resourceRevision":4,"surfaceConfiguration":5,"styleRevision":6,
        "validationFrame":{"ordinal":ordinal,"timeUs":10,"drawId":7,"outcome":"presented"},
        "pixelPresentation":{"frame":{"ordinal":ordinal,"timeUs":10,"drawId":7,"outcome":"presented"},
            "sequence":3,"configurationId":5,"drawId":7},
        "layoutDocumentCoherent":true,"captureId":"cap-one",
        "readiness":{"appReady":true,"currentWorkSettled":true,"frameCurrent":true,"nativeAssetReadinessAvailable":true,"timeUs":10},
        "resources":{"requiredAssetsReady":true,"pendingIds":[],"allowedPendingAssets":[],"resourceRevision":4},
        "requiredStateRevision":4,"validatedStateRevision":4,"timingDeterministic":true,"scale":1.0,
        "physicalWidth":640,"physicalHeight":480,"renderer":{"kind":"software"},"allowedPendingAssets":[]})).unwrap()
}

#[test]
fn frame_and_tree_identities_must_agree() {
    let tree=tree(3,9);
    assert!(tree.validate_frame(&frame(3,9)).is_ok(), "matching frame and tree identities must validate");
    assert!(tree.validate_frame(&frame(4,9)).is_err(), "a frame from another generation must not validate");
    assert!(tree.validate_frame(&frame(3,8)).is_err(), "a frame with another ordinal must not validate");
    let mut older_build=frame(3,9);
    older_build.build_id="build-two".into();
    assert!(tree.validate_frame(&older_build).is_err(), "a frame from another build must not validate");
    let mut missing_revision=frame(3,9);
    missing_revision.resource_revision=None;
    assert!(tree.validate_frame(&missing_revision).is_err(), "a frame missing the tree's resource revision must not validate");
}

#[test]
fn named_capture_paths_cannot_collide_with_main_or_be_forged() {
    let main = CapturePaths::new("cap-one", "main").unwrap();
    let named = CapturePaths::new("cap-one", "cap-one.png").unwrap();
    assert_eq!(main.png(), "captures/cap-one.png", "main retains the flat capture path");
    assert_eq!(named.png(), "captures/surfaces/cap-one.png/cap-one.png", "named captures must be scoped to their surface");
    let decoded = CapturePaths::from_artifact(&named.frame(), ".frame.json").unwrap();
    assert_eq!((decoded.id(), decoded.surface()), ("cap-one", "cap-one.png"), "artifact decoding preserves both capture and surface identity");
    for path in [
        "captures/surfaces/main/cap-one.png",
        "captures/surfaces/../cap-one.png",
        "captures/surfaces/cap-one.png/part/cap-one.png",
    ] {
        assert!(CapturePaths::from_artifact(path, ".png").is_err(), "accepted forged capture path {path}");
    }

    let mut tree = tree(3, 9);
    let mut frame = frame(3, 9);
    tree.surface = named.surface().to_owned();
    frame.surface = tree.surface.clone();
    tree.nodes[0].surface = tree.surface.clone();
    tree.extra.insert("checkpointSequence".into(), json!(11));
    frame.extra.insert("checkpointSequence".into(), json!(11));
    let digest = output::digest(&serde_json::to_vec(&tree).unwrap());
    frame.tree_digest = Some(digest.clone());
    frame.png_digest = Some("sha256:aa".into());
    let terminal = json!({"schemaVersion":1,"command":"run","build":{"buildId":"build-one"},"generation":3});
    assert!(frame.validate_capture(&tree, &named, &digest, &terminal).is_ok(), "named evidence validates under its own path");
    assert!(frame.validate_capture(&tree, &main, &digest, &terminal).is_err(), "named evidence must not validate under main's path");
}

#[test]
fn capture_validation_rejects_wrong_tree_or_terminal_identity() {
    let tree=tree(3,9);
    let mut frame=frame(3,9);
    let paths=CapturePaths::new("cap-one","main").unwrap();
    let tree_bytes=serde_json::to_vec(&tree).unwrap();
    let digest=output::digest(&tree_bytes);
    frame.tree_digest=Some(digest.clone());
    frame.png_digest=Some("sha256:aa".into());
    let terminal=json!({"schemaVersion":1,"command":"run","build":{"buildId":"build-one"},"generation":3});
    assert!(frame.validate_capture(&tree,&paths,&digest,&terminal).is_ok(), "matching terminal and capture identities must validate");
    // A later terminal generation accepts an earlier capture generation.
    let later=json!({"schemaVersion":1,"command":"run","build":{"buildId":"build-one"},"generation":4});
    assert!(frame.validate_capture(&tree,&paths,&digest,&later).is_ok(), "older capture remains valid after a later generation");
    // An earlier terminal generation does not.
    let earlier=json!({"schemaVersion":1,"command":"run","build":{"buildId":"build-one"},"generation":2});
    assert!(frame.validate_capture(&tree,&paths,&digest,&earlier).is_err(), "terminal evidence cannot reference a future capture generation");
    assert!(frame.validate_capture(&tree,&paths,"sha256:other",&terminal).is_err(), "capture must reject an unrelated tree digest");
    let mut wrong_id=frame.clone();
    wrong_id.capture_id=Some("cap-two".into());
    assert!(wrong_id.validate_capture(&tree,&paths,&digest,&terminal).is_err(), "frame capture ID must match the artifact path");
    let mut incoherent=frame.clone();
    incoherent.layout_document_coherent=false;
    assert!(incoherent.validate_capture(&tree,&paths,&digest,&terminal).is_err(), "incoherent layout must not be accepted as capture evidence");
}

#[test]
fn topology_rejects_cycles_dangling_parents_and_duplicates() {
    let mut tree=tree(1,1);
    assert_eq!(tree.validate_topology().unwrap(),vec![None], "the only node is the parentless root");
    let mut node=tree.nodes[0].clone();
    node.token="n1".into();
    node.parent=Some("n0".into());
    tree.nodes.push(node.clone());
    assert_eq!(tree.validate_topology().unwrap(),vec![None,Some(0)], "child refers to the first node");
    let mut duplicate=tree.clone();
    duplicate.nodes[1].parent=None;
    duplicate.nodes[1].token="n0".into();
    assert!(duplicate.validate_topology().is_err(), "duplicate node tokens must be rejected");
    let mut dangling=tree.clone();
    dangling.nodes[1].parent=Some("absent".into());
    assert!(dangling.validate_topology().is_err(), "parent token must identify a node in the tree");
    let mut child_first=tree.clone();
    child_first.nodes.swap(0,1);
    assert!(child_first.validate_topology().is_err(), "node records must follow parent-first order");
    // A node recorded for another surface or generation cannot belong to this tree.
    let mut foreign=tree.clone();
    foreign.nodes[1].generation=2;
    assert!(foreign.validate_topology().is_err(), "a node from a different generation must not belong to the tree");
}

#[test]
fn malformed_identity_fields_never_default_to_matching_values() {
    // Absent or mistyped identities fail to parse instead of comparing equal.
    assert!(serde_json::from_value::<TreeEvidence>(json!({"schemaVersion":1,"generation":3})).is_err(), "missing identity fields must not default");
    let mut value=serde_json::to_value(tree(3,9)).unwrap();
    value["buildId"]=Value::Null;
    assert!(serde_json::from_value::<TreeEvidence>(value.clone()).is_err(), "a null build identity must not parse");
    value["buildId"]=json!("build-one");
    value["generation"]=json!("3");
    assert!(serde_json::from_value::<TreeEvidence>(value).is_err(), "a string generation must not parse");
}

#[test]
fn readiness_issues_move_verbatim() {
    let complete=frame(3,9);
    let mut issues=Vec::new();
    complete.readiness_issues("baseline",&mut issues);
    assert!(issues.is_empty(), "complete baseline reported readiness issues: {issues:?}");
    let mut incomplete=frame(3,9);
    incomplete.extra.get_mut("readiness").unwrap()["appReady"]=json!(false);
    incomplete.readiness_issues("actual",&mut issues);
    assert_eq!(issues.len(),1, "only appReady was false in the actual frame");
    assert_eq!(issues[0]["side"],"actual", "the issue belongs to the incomplete side");
}

#[test]
fn optional_revision_fields_reject_present_null_and_roundtrip_absent() {
    for field in ["resourceRevision","surfaceConfiguration","styleRevision"] {
        // Legacy evidence without the revision parses and reserializes without a null.
        let mut legacy=serde_json::to_value(tree(3,9)).unwrap();
        legacy.as_object_mut().unwrap().remove(field);
        let parsed:TreeEvidence=serde_json::from_value(legacy).unwrap();
        assert!(!serde_json::to_value(&parsed).unwrap().as_object().unwrap().contains_key(field), "legacy tree must omit absent {field}");
        for malformed in [Value::Null,json!("4"),json!(-1)] {
            let mut value=serde_json::to_value(tree(3,9)).unwrap();
            value[field]=malformed.clone();
            assert!(serde_json::from_value::<TreeEvidence>(value).is_err(),"{field} accepted malformed revision");
            let mut value=serde_json::to_value(frame(3,9)).unwrap();
            value[field]=malformed.clone();
            assert!(serde_json::from_value::<FrameEvidence>(value).is_err(),"{field} accepted malformed revision");
        }
    }
    // Native resource readiness makes revisions mandatory, never defaulted.
    let mut native=tree(3,9);
    native.resource_revision=None;
    assert!(native.validate_frame(&frame(3,9)).is_err(), "native readiness requires a resource revision");
    native.surface_configuration=None;
    native.resource_revision=Some(4);
    assert!(native.validate_frame(&frame(3,9)).is_err(), "native readiness requires a surface configuration revision");
    // Metadata and surface-configuration mismatches are identity failures.
    let mut metadata=frame(3,9);
    metadata.metadata_revision=99;
    assert!(tree(3,9).validate_frame(&metadata).is_err(), "a different metadata revision must fail identity validation");
    let mut configuration=frame(3,9);
    configuration.surface_configuration=Some(99);
    assert!(tree(3,9).validate_frame(&configuration).is_err(), "a different surface configuration must fail identity validation");
    // Style absence on both sides remains a valid legacy pair with native resources.
    let mut legacy_tree=tree(3,9);
    legacy_tree.style_revision=None;
    let mut legacy_frame=frame(3,9);
    legacy_frame.style_revision=None;
    assert!(legacy_tree.validate_frame(&legacy_frame).is_ok(), "legacy evidence can omit style revisions on both sides");
}

#[test]
fn deleted_extension_keys_fail_structured_not_panic() {
    for key in ["readiness","resources","allowedPendingAssets","timingDeterministic","requiredStateRevision",
        "validatedStateRevision","scale","physicalWidth","physicalHeight","renderer","captureId"] {
        let mut value=serde_json::to_value(frame(3,9)).unwrap();
        value.as_object_mut().unwrap().remove(key);
        let pruned:FrameEvidence=serde_json::from_value(value).unwrap();
        assert!(pruned.detail(key).is_null(),"{key} still readable after removal");
        match key {
            "captureId" => {
                assert!(pruned.validate_capture(&tree(3,9),&CapturePaths::new("cap-one","main").unwrap(),"sha256:x",
                    &json!({"schemaVersion":1,"command":"run","build":{"buildId":"build-one"},"generation":3})).is_err(), "capture validation requires its missing ID");
            }
            // Compare flags a missing pixel extent as a structured mismatch.
            "physicalWidth" => assert!(pruned.detail("physicalWidth").as_u64()!=Some(640), "missing width must not default to 640"),
            "physicalHeight" => assert!(pruned.detail("physicalHeight").as_u64()!=Some(480), "missing height must not default to 480"),
            // Readiness completeness keys surface as incomplete issues, never a panic.
            "renderer" => { let mut issues=Vec::new(); pruned.readiness_issues("baseline",&mut issues); }
            _ => {
                let mut issues=Vec::new();
                pruned.readiness_issues("baseline",&mut issues);
                assert!(!issues.is_empty(),"{key} removal produced no readiness issue");
            }
        }
    }
    for key in ["truncation","semanticProjection"] {
        let mut value=serde_json::to_value(tree(3,9)).unwrap();
        value.as_object_mut().unwrap().remove(key);
        let pruned:TreeEvidence=serde_json::from_value(value).unwrap();
        assert!(pruned.detail(key).is_null(),"{key} still readable after removal");
        assert!(pruned.validate_topology().is_ok(), "missing {key} must not invalidate topology");
        let target=scenario::Target{surface:"main".into(),selector:scenario::Selector::TestId("x".into())};
        let _=crate::preview::selectors::unique(&pruned,&target,false);
    }
}

#[test]
fn historical_publication_validates_the_pair_before_mutating() {
    let directory=Directory::new();
    let mut output=Output::create(&directory.0,1024*1024).unwrap();
    // Mismatched pair: nothing is created.
    assert!(publish_historical(&mut output,&tree(3,9),&frame(3,8),None,
        HistoricalStatus{satisfies_failure_revision:Some(true),readiness:None,tree_role:"failure-tree"}).is_err(), "historical publication must reject an unmatched ordinal");
    assert!(!directory.0.join("failure").exists(), "an unmatched pair must not create failure artifacts");
    // Prepublished tree with same build/gen/ordinal but wrong metadata is rejected byte-for-byte.
    let mut wrong_metadata=tree(3,9);
    wrong_metadata.metadata_revision=99;
    output.json("failure/tree.json","failure-tree",&serde_json::to_value(&wrong_metadata).unwrap()).unwrap();
    let before=std::fs::read(directory.0.join("failure/tree.json")).unwrap();
    assert!(publish_historical(&mut output,&tree(3,9),&frame(3,9),None,
        HistoricalStatus{satisfies_failure_revision:Some(true),readiness:None,tree_role:"failure-tree"}).is_err(), "prepublished tree must match metadata revision");
    assert_eq!(std::fs::read(directory.0.join("failure/tree.json")).unwrap(),before, "existing tree bytes must survive rejection");
    assert!(!directory.0.join("failure/frame.json").exists(), "rejection must not publish the missing frame");
    // Prepublished frame with a different complete stamp is rejected before mutation.
    let directory=Directory::new();
    let mut output=Output::create(&directory.0,1024*1024).unwrap();
    let mut other=frame(3,9);
    other.validation_frame.draw_id=99;
    output.json("failure/frame.json","historical-frame",&serde_json::to_value(&other).unwrap()).unwrap();
    assert!(publish_historical(&mut output,&tree(3,9),&frame(3,9),None,
        HistoricalStatus{satisfies_failure_revision:Some(true),readiness:None,tree_role:"failure-tree"}).is_err(), "prepublished frame must match the draw ID");
    assert!(!directory.0.join("failure/tree.json").exists(), "frame mismatch must not publish a tree");
    assert!(!directory.0.join("failure/last-valid.png").exists(), "frame mismatch must not publish pixels");
    // Prepublished pixels with different bytes are rejected.
    let directory=Directory::new();
    let mut output=Output::create(&directory.0,1024*1024).unwrap();
    output.bytes("failure/last-valid.png","historical-pixels",b"original").unwrap();
    assert!(publish_historical(&mut output,&tree(3,9),&frame(3,9),Some(b"different"),
        HistoricalStatus{satisfies_failure_revision:None,readiness:None,tree_role:"historical-tree"}).is_err(), "conflicting historical pixels must be rejected");
    assert_eq!(std::fs::read(directory.0.join("failure/last-valid.png")).unwrap(),b"original", "conflicting pixels must not overwrite existing bytes");
}

#[test]
fn live_and_recovered_historical_share_identity_but_differ_in_freshness() {
    let directory=Directory::new();
    let mut output=Output::create(&directory.0,1024*1024).unwrap();
    let live=publish_historical(&mut output,&tree(3,9),&frame(3,9),Some(b"png"),
        HistoricalStatus{satisfies_failure_revision:Some(true),readiness:Some(json!({"appReady":true})),tree_role:"failure-tree"}).unwrap();
    assert_eq!(live["pixels"],"failure/last-valid.png", "the live historical publication must name its pixel artifact");
    let live_frame:Value=serde_json::from_slice(&std::fs::read(directory.0.join("failure/frame.json")).unwrap()).unwrap();
    assert_eq!(live_frame["satisfiesFailureRevision"],true, "live evidence carries known freshness");
    assert_eq!(live_frame["failureReadiness"]["appReady"],true, "live evidence carries failure readiness");
    // Repeated publication with the same pair is idempotent.
    publish_historical(&mut output,&tree(3,9),&frame(3,9),Some(b"png"),
        HistoricalStatus{satisfies_failure_revision:None,readiness:None,tree_role:"historical-tree"}).unwrap();
    let recovered_frame:Value=serde_json::from_slice(&std::fs::read(directory.0.join("failure/frame.json")).unwrap()).unwrap();
    assert_eq!(recovered_frame,live_frame, "idempotent recovery preserves existing live evidence");
    // Live pixels supplied again agree by digest.
    assert!(publish_historical(&mut output,&tree(3,9),&frame(3,9),Some(b"png"),
        HistoricalStatus{satisfies_failure_revision:None,readiness:None,tree_role:"historical-tree"}).is_ok(), "identical live pixels must match the published digest");
}

#[test]
fn historical_retains_pixels_larger_than_the_json_read_cap() {
    let directory=Directory::new();
    let mut output=Output::create(&directory.0,16*1024*1024).unwrap();
    let pixels=vec![0xAB_u8;8*1024*1024+1];
    publish_historical(&mut output,&tree(3,9),&frame(3,9),Some(&pixels),
        HistoricalStatus{satisfies_failure_revision:Some(true),readiness:None,tree_role:"failure-tree"}).unwrap();
    // A second identical publication keeps the retained bytes and succeeds.
    publish_historical(&mut output,&tree(3,9),&frame(3,9),Some(&pixels),
        HistoricalStatus{satisfies_failure_revision:None,readiness:None,tree_role:"historical-tree"}).unwrap();
    // Recovery over the same retained bytes succeeds too.
    assert_eq!(recover(&mut output).unwrap()["available"],true, "recovery must accept pixels larger than the JSON read limit");
    assert_eq!(std::fs::read(directory.0.join("failure/last-valid.png")).unwrap(),pixels, "recovery preserves the full pixel artifact");
}

#[test]
fn recovery_completes_a_half_published_failure_without_overwriting_its_tree() {
    let directory=Directory::new();
    let mut output=Output::create(&directory.0,1024*1024).unwrap();
    output.json("evidence/generation-0000000001-frame-0000000001.tree.json","validation-tree",&serde_json::to_value(tree(1,1)).unwrap()).unwrap();
    output.json("evidence/generation-0000000001-frame-0000000001.frame.json","validation-frame",&serde_json::to_value(frame(1,1)).unwrap()).unwrap();
    output.json("failure/tree.json","failure-tree",&serde_json::to_value(tree(1,1)).unwrap()).unwrap();
    let before=std::fs::read(directory.0.join("failure/tree.json")).unwrap();
    // A recreated surface may restart its ordinal; it cannot replace the failure's selected generation.
    output.json("evidence/generation-0000000002-frame-0000000001.tree.json","validation-tree",&serde_json::to_value(tree(2,1)).unwrap()).unwrap();
    output.json("evidence/generation-0000000002-frame-0000000001.frame.json","validation-frame",&serde_json::to_value(frame(2,1)).unwrap()).unwrap();
    let result=recover(&mut output).unwrap();
    assert_eq!(result["available"],true, "the half-published tree has a valid frame to recover");
    assert_eq!(std::fs::read(directory.0.join("failure/tree.json")).unwrap(),before, "recovery must not overwrite the selected generation's tree");
    let saved:Value=serde_json::from_slice(&read_evidence(&directory.0.join("failure/frame.json")).unwrap()).unwrap();
    assert_eq!(saved["validationFrame"]["ordinal"],1, "the recovered frame must match the selected tree ordinal");
    assert_eq!(saved["generation"],1, "the recovered frame must match the selected tree generation");
    assert_eq!(saved["historical"],true, "the recovered frame is historical");
    assert!(saved["satisfiesFailureRevision"].is_null(), "recovery cannot prove freshness at the failure revision");
    assert_eq!(recover(&mut output).unwrap(),result, "repeating recovery must preserve the published evidence");
}

#[test]
fn recovery_rejects_a_corrupt_candidate_pixel_digest() {
    let directory=Directory::new();
    let mut output=Output::create(&directory.0,1024*1024).unwrap();
    output.json("evidence/generation-0000000001-frame-0000000001.tree.json","validation-tree",&serde_json::to_value(tree(1,1)).unwrap()).unwrap();
    let mut capture=frame(1,1);
    capture.capture_id=Some("cap-one".into());
    capture.png_digest=Some("sha256:not-the-png".into());
    output.json("captures/cap-one.frame.json","capture-frame",&serde_json::to_value(&capture).unwrap()).unwrap();
    output.bytes("captures/cap-one.png","capture-pixels",b"png-bytes").unwrap();
    output.json("evidence/generation-0000000001-frame-0000000001.frame.json","validation-frame",&serde_json::to_value(frame(1,1)).unwrap()).unwrap();
    assert!(recover(&mut output).is_err(), "recovery must reject pixels with a mismatched digest");
}

#[test]
fn recovery_skips_only_empty_documents_to_the_last_complete_checkpoint() {
    let directory=Directory::new();
    let mut output=Output::create(&directory.0,1024*1024).unwrap();
    output.json("evidence/generation-0000000001-frame-0000000002.tree.json","validation-tree",&serde_json::to_value(tree(1,2)).unwrap()).unwrap();
    output.json("evidence/generation-0000000001-frame-0000000002.frame.json","validation-frame",&serde_json::to_value(frame(1,2)).unwrap()).unwrap();
    // The newest pair documents an empty post-fault document.
    let mut empty=tree(1,3);
    empty.nodes.clear();
    output.json("evidence/generation-0000000001-frame-0000000003.tree.json","validation-tree",&serde_json::to_value(&empty).unwrap()).unwrap();
    output.json("evidence/generation-0000000001-frame-0000000003.frame.json","validation-frame",&serde_json::to_value(frame(1,3)).unwrap()).unwrap();
    let result=recover(&mut output).unwrap();
    assert_eq!(result["available"],true, "an older complete checkpoint is recoverable");
    let saved:Value=serde_json::from_slice(&std::fs::read(directory.0.join("failure/frame.json")).unwrap()).unwrap();
    assert_eq!(saved["validationFrame"]["ordinal"],2, "recovery must skip the empty checkpoint and select the latest complete pair");
}

#[test]
fn recovery_rejects_corrupt_nonempty_checkpoints() {
    for corrupt in ["duplicate-token","dangling-parent"] {
        let directory=Directory::new();
        let mut output=Output::create(&directory.0,1024*1024).unwrap();
        let mut bad=tree(1,2);
        if corrupt=="duplicate-token" { bad.nodes.push(bad.nodes[0].clone()); }
        else { bad.nodes[0].parent=Some("absent".into()); }
        output.json("evidence/generation-0000000001-frame-0000000002.tree.json","validation-tree",&serde_json::to_value(&bad).unwrap()).unwrap();
        output.json("evidence/generation-0000000001-frame-0000000002.frame.json","validation-frame",&serde_json::to_value(frame(1,2)).unwrap()).unwrap();
        assert_eq!(recover(&mut output).unwrap_err().code,"E_OUTPUT_IO","recovery must reject corrupt nonempty checkpoint: {corrupt}");
        assert!(!directory.0.join("failure").exists(),"corrupt checkpoint must not publish evidence: {corrupt}");
    }
    // A pair-identity failure errors even when the tree itself is empty.
    let directory=Directory::new();
    let mut output=Output::create(&directory.0,1024*1024).unwrap();
    let mut empty=tree(1,2);
    empty.nodes.clear();
    output.json("evidence/generation-0000000001-frame-0000000002.tree.json","validation-tree",&serde_json::to_value(&empty).unwrap()).unwrap();
    output.json("evidence/generation-0000000001-frame-0000000002.frame.json","validation-frame",&serde_json::to_value(frame(1,3)).unwrap()).unwrap();
    assert_eq!(recover(&mut output).unwrap_err().code,"E_OUTPUT_IO", "an empty tree cannot excuse mismatched frame identity");
    assert!(!directory.0.join("failure").exists(), "mismatched identity must not publish evidence");
}
