use super::*;
use crate::VoiceActivationFact;
use serde_json::json;
use std::sync::Mutex;

fn target(id: &str) -> WorkTarget {
    WorkTarget {
        agent_session_id: "agent".into(),
        binding_version: 1,
        turn_operation_id: id.into(),
        execution_generation: 1,
    }
}
fn receipt(operation: &str, target: Option<WorkTarget>) -> VoiceWorkReceipt {
    let first_claim_generation=target.as_ref().map(|target|target.execution_generation).filter(|generation|*generation>0);
    VoiceWorkReceipt {
        receipt_id: format!("receipt:{operation}"),
        operation_key: operation.into(),
        target,
        pending_input_id: None,
        pending_input_revision: None,
        first_claim_generation,
        status: VoiceWorkStatus::Applied,
        summary: "canonical fixture".into(),
        duplicate: false,
        speech: None,
    }
}
#[derive(Default)]
struct RecordingPort {
    calls: Mutex<Vec<VoiceWorkRequest>>,
    receipts: Mutex<BTreeMap<String, VoiceWorkReceipt>>,
    queued: bool,
    gate: Mutex<Option<Arc<tokio::sync::Notify>>>,
    entered: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl VoiceWorkPort for RecordingPort {
    async fn answer_approval(
        &self,
        _: &str,
        _: &str,
        _: nomifun_voice_contracts::VoiceApprovalAnswer,
    ) -> Result<VoiceWorkReceipt, VoiceError> {
        Err(invalid("unused approval fixture"))
    }
    async fn context(&self, _: &str, _: &str, _: u64) -> Result<VoiceWorkContext, VoiceError> {
        Err(invalid("unused context fixture"))
    }
    async fn interact(
        &self,
        _: &str,
        _: &str,
        _: u64,
        operation: &str,
        request: VoiceWorkRequest,
    ) -> Result<VoiceWorkReceipt, VoiceError> {
        self.calls.lock().unwrap().push(request.clone());
        let gate = self.gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            self.entered.notify_one();
            gate.notified().await;
        }
        let mut result = match request {
            VoiceWorkRequest::Start { .. } if self.queued => {
                let mut result = receipt(operation, None);
                result.status = VoiceWorkStatus::Queued;
                result.pending_input_id = Some("exact-pending-a".into());
                result.pending_input_revision = Some(1);
                result
            }
            VoiceWorkRequest::Start { .. } => receipt(operation, Some(target("A"))),
            VoiceWorkRequest::Steer { target, .. }
            | VoiceWorkRequest::Observe { target }
            | VoiceWorkRequest::Cancel { target } => receipt(operation, Some(target)),
            VoiceWorkRequest::ReviseQueued {
                pending_input_id,
                expected_revision,
                ..
            } => {
                let mut result = receipt(operation, None);
                result.status = VoiceWorkStatus::Queued;
                result.pending_input_id = Some(pending_input_id);
                result.pending_input_revision = Some(expected_revision + 1);
                result
            }
            _ => receipt(operation, None),
        };
        result.operation_key = operation.into();
        self.receipts
            .lock()
            .unwrap()
            .insert(operation.into(), result.clone());
        Ok(result)
    }
    async fn lookup_operation(
        &self,
        _: &str,
        _: &str,
        operation: &str,
    ) -> Result<Option<VoiceWorkReceipt>, VoiceError> {
        Ok(self.receipts.lock().unwrap().get(operation).cloned())
    }
    async fn observe(&self, _: &str, target: &WorkTarget) -> Result<VoiceWorkReceipt, VoiceError> {
        Ok(receipt("observe", Some(target.clone())))
    }
}

#[tokio::test]
async fn canonical_wait_does_not_lock_media_context_and_retains_inflight_revision() {
    let (mut bridge, port, _directory) = fixture(false).await;
    bridge.transcript(fragment(
        "waiting",
        1,
        "Prepare Friday report",
        0,
        1_000_000,
    ));
    let prepared = bridge
        .prepare_trigger(typed(
            "waiting-start",
            json!({"action":"start","text":"Prepare Friday report"}),
            Some("live:user:waiting"),
        ))
        .unwrap();
    let bridge = Arc::new(tokio::sync::Mutex::new(bridge));
    let gate = Arc::new(tokio::sync::Notify::new());
    *port.gate.lock().unwrap() = Some(gate.clone());
    let execute = tokio::spawn(prepared.execute());
    port.entered.notified().await;
    tokio::time::timeout(std::time::Duration::from_millis(100), async {
        let mut bridge = bridge.lock().await;
        assert!(
            bridge
                .transcript_revision(fragment(
                    "waiting",
                    2,
                    "Prepare Thursday report",
                    0,
                    1_000_000
                ))
                .unwrap()
                .is_none()
        );
        bridge
            .context_checkpoint(
                "live".into(),
                MediaRange {
                    start_us: 1_200_000,
                    end_us: 1_300_000,
                },
                Some(target("B")),
                "concurrent-b-context".into(),
            )
            .unwrap();
    })
    .await
    .expect("canonical execution owns no bridge lock");
    gate.notify_one();
    let executed = execute.await.unwrap().unwrap();
    let completed = bridge.lock().await.finish_trigger(executed);
    assert!(completed.revision_error.is_none());
    assert!(matches!(completed.receipt.target,Some(t) if t==target("A")));
    let correction = completed
        .revision_correction
        .expect("revision during canonical wait is linked after its receipt");
    *port.gate.lock().unwrap() = None;
    let prepared = bridge.lock().await.prepare_trigger(correction).unwrap();
    let executed = prepared.execute().await.unwrap();
    bridge.lock().await.finish_trigger(executed);
    assert!(
        matches!(&port.calls.lock().unwrap()[1],VoiceWorkRequest::Steer{target:t,text} if t==&target("A")&&text.contains("Thursday"))
    );
}
async fn fixture(queued: bool) -> (VoiceWorkBridge, Arc<RecordingPort>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let journal = VoiceJournal::open(directory.path()).unwrap();
    journal
        .activate(VoiceActivationFact {
            context_floor:Some(0),
            lease_revision:Some("lease".into()),
            voice_session_id: "voice".into(),
            epoch: 1,
            owner_id: "owner".into(),
            agent_session_id: "agent".into(),
            binding_version: 1,
            route_digest: "a".repeat(64),
            started_ms: 1,
        })
        .await
        .unwrap();
    let port = Arc::new(RecordingPort {
        queued,
        ..Default::default()
    });
    let mut bridge = VoiceWorkBridge::new(
        port.clone(),
        journal,
        "owner".into(),
        "agent".into(),
        1,
        "voice".into(),
        1,
        VoiceWorkContext {context_floor:0,context_reference:"canonical-context".into(),
            instructions: String::new(),
            initial_facts: vec![],
            observed_target: Some(target("A")),
            facts: vec![],
            approvals: vec![],
        },
    );
    bridge
        .open_context(
            "live".into(),
            Some(VoiceWorkContextFact {
                target: Some(target("A")),
            }),
        )
        .unwrap();
    (bridge, port, directory)
}
fn fragment(id: &str, revision: u64, text: &str, start: u64, end: u64) -> TranscriptFragment {
    TranscriptFragment {
        speaker: VoiceSpeaker::User,
        fragment_id: format!("live:user:{id}"),
        revision,
        commit: TranscriptCommit::Committed,
        text: text.into(),
        media_range: Some(MediaRange {
            start_us: start,
            end_us: end,
        }),
    }
}
fn delegated(id: &str, offset: u64) -> WorkTrigger {
    WorkTrigger::DelegationTrigger {
        upstream_trigger_id: id.into(),
        target: "client".into(),
        offset_ms: offset,
        context_window_ref: Some("live".into()),
    }
}
fn typed(id: &str, args: serde_json::Value, source: Option<&str>) -> WorkTrigger {
    WorkTrigger::TypedToolCall {
        upstream_trigger_id: id.into(),
        name: "nomi_work".into(),
        arguments: args,
        transcript_window_ref: source.map(str::to_owned),
    }
}

#[tokio::test]
async fn typed_mutation_requires_current_committed_user_input_and_rejects_backchannels() {
    let (mut bridge,port,_directory)=fixture(false).await;
    let start=json!({"action":"start","text":"Prepare a report"});
    assert!(bridge.trigger(typed("invented-before-input",start.clone(),None)).await.is_err());
    bridge.transcript(fragment("backchannel",1,"嗯",0,20_000));
    assert!(bridge.trigger(typed("backchannel-tool",start.clone(),None)).await.is_err());
    bridge.transcript(fragment("media-only",1,"停止说话",10_000,20_000));
    assert!(bridge.trigger(typed("misclassified-media",json!({"action":"cancel","target":target("A")}),None)).await.is_err());
    let mut tentative=fragment("real-input",1,"Prepare a report",20_000,40_000);tentative.commit=TranscriptCommit::Tentative;
    bridge.transcript(tentative);
    assert!(bridge.trigger(typed("tentative-tool",start.clone(),None)).await.is_err());
    bridge.transcript(fragment("real-input",2,"Prepare a report",20_000,40_000));
    assert!(bridge.trigger(typed("unbound-committed-tool",start.clone(),None)).await.is_err());
    let mut explicit=start;explicit["source_ref"]=json!({"fragment_id":"live:user:real-input","revision":2});
    bridge.trigger(typed("committed-tool",explicit,None)).await.unwrap();
    assert_eq!(port.calls.lock().unwrap().len(),1);
}

#[tokio::test]
async fn missing_typed_target_never_uses_successor_presentation_cache() {
    let (mut bridge, port, _directory) = fixture(false).await;
    bridge.note_context_target(Some(target("B"))).unwrap();
    for action in ["steer", "observe", "cancel"] {
        assert!(
            bridge
                .trigger(typed(
                    action,
                    json!({"action":action,"text":"late A"}),
                    None
                ))
                .await
                .is_err()
        );
    }
    assert!(port.calls.lock().unwrap().is_empty());
    bridge.transcript(fragment("cancel-a",1,"Cancel the original task A",0,1_000_000));
    bridge
        .trigger(typed(
            "exact-a",
            json!({"action":"cancel","target":target("A")}),
            Some("live:user:cancel-a"),
        ))
        .await
        .unwrap();
    assert!(
        matches!(&port.calls.lock().unwrap()[0],VoiceWorkRequest::Cancel{target:t} if t==&target("A"))
    );
    assert_eq!(
        bridge.observed_target(),
        Some(&target("B")),
        "an operation targeting A cannot rebind the current context cache"
    );
}
#[tokio::test]
async fn explicit_source_reference_never_rebinds_late_a_to_latest_b_or_a_new_revision(){
    let(mut bridge,port,_directory)=fixture(false).await;
    bridge.transcript(fragment("A",1,"Prepare report A",0,10_000));
    bridge.transcript(fragment("B",1,"Prepare report B",10_000,20_000));
    let a=json!({"action":"start","text":"Prepare report A","source_ref":{"fragment_id":"live:user:A","revision":1}});
    bridge.trigger(typed("late-A",a,None)).await.unwrap();
    let duplicate=json!({"action":"start","text":"Prepare report A","source_ref":{"fragment_id":"live:user:A","revision":1}});
    assert!(bridge.trigger(typed("different-call-same-source",duplicate,None)).await.unwrap().duplicate);
    assert_eq!(port.calls.lock().unwrap().len(),1);
    let correction=bridge.transcript_revision(fragment("A",2,"Prepare corrected report A",0,10_000)).unwrap().unwrap();bridge.trigger(correction).await.unwrap();
    assert!(matches!(&port.calls.lock().unwrap()[1],VoiceWorkRequest::Steer{text,..} if text.contains("corrected report A")));
    assert!(bridge.transcript_revision(fragment("B",2,"Prepare changed report B",10_000,20_000)).unwrap().is_none());
    let stale=json!({"action":"start","text":"stale A","source_ref":{"fragment_id":"live:user:A","revision":1}});
    assert!(bridge.trigger(typed("stale-revision",stale,None)).await.is_err());assert_eq!(port.calls.lock().unwrap().len(),2);
}

#[tokio::test]
async fn live_timeline_binds_old_a_and_new_b_without_latest_target_fallback() {
    let (mut bridge, port, _directory) = fixture(false).await;
    bridge.note_context_target(Some(target("B"))).unwrap();
    bridge.transcript(fragment("a", 1, "取消这个任务", 0, 1_000_000));
    bridge.trigger(delegated("old-a", 1000)).await.unwrap();
    assert!(
        matches!(&port.calls.lock().unwrap()[0],VoiceWorkRequest::Cancel{target:t} if t==&target("A"))
    );
    bridge
        .context_checkpoint(
            "live".into(),
            MediaRange {
                start_us: 1_200_000,
                end_us: 1_300_000,
            },
            Some(target("B")),
            "verified-b-receipt".into(),
        )
        .unwrap();
    bridge.transcript(fragment("b", 1, "任务进度如何", 1_500_000, 2_000_000));
    bridge.trigger(delegated("new-b", 2000)).await.unwrap();
    assert!(
        matches!(&port.calls.lock().unwrap()[1],VoiceWorkRequest::Observe{target:t} if t==&target("B"))
    );
    assert!(bridge.trigger(delegated("late-a", 900)).await.is_err());
    assert_eq!(port.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn crossing_target_boundary_and_future_transcript_cannot_be_admitted() {
    let (mut bridge, port, _directory) = fixture(false).await;
    bridge
        .context_checkpoint(
            "live".into(),
            MediaRange {
                start_us: 1_000_000,
                end_us: 1_200_000,
            },
            Some(target("B")),
            "b-receipt".into(),
        )
        .unwrap();
    bridge.transcript(fragment("cross", 1, "改成周四", 900_000, 1_300_000));
    assert!(bridge.trigger(delegated("crossing", 1300)).await.is_err());
    assert!(port.calls.lock().unwrap().is_empty());
    let (mut bridge, port, _directory) = fixture(false).await;
    bridge.transcript(fragment("future", 1, "取消这个任务", 1_000_000, 1_500_000));
    assert!(
        bridge
            .trigger(delegated("before-utterance", 900))
            .await
            .is_err()
    );
    assert!(port.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn committed_substantive_revision_corrects_original_a_and_noise_has_no_effect() {
    let (mut bridge, port, _directory) = fixture(false).await;
    bridge.transcript(fragment(
        "request",
        1,
        "Prepare the Friday report",
        0,
        1_000_000,
    ));
    bridge
        .trigger(typed(
            "original",
            json!({"action":"start","text":"Prepare the Friday report"}),
            Some("live:user:request"),
        ))
        .await
        .unwrap();
    bridge.note_context_target(Some(target("B"))).unwrap();
    let correction = bridge
        .transcript_revision(fragment(
            "request",
            2,
            "Prepare the Thursday report",
            0,
            1_000_000,
        ))
        .unwrap()
        .unwrap();
    bridge.trigger(correction).await.unwrap();
    assert!(
        matches!(&port.calls.lock().unwrap()[1],VoiceWorkRequest::Steer{target:t,text} if t==&target("A")&&text.contains("Thursday"))
    );
    let replay = bridge
        .trigger(typed(
            "original",
            json!({"action":"start","text":"Prepare the Friday report"}),
            Some("live:user:request"),
        ))
        .await
        .unwrap();
    assert!(
        replay.duplicate,
        "an old original payload remains a replay after its linked correction"
    );
    assert_eq!(
        port.calls.lock().unwrap().len(),
        2,
        "late original payload cannot undo the Thursday correction"
    );
    assert!(
        bridge
            .transcript_revision(fragment(
                "request",
                3,
                "Prepare the Thursday report!",
                0,
                1_000_000
            ))
            .unwrap()
            .is_none()
    );
    let mut tentative = fragment("request", 4, "Prepare the Monday report", 0, 1_000_000);
    tentative.commit = TranscriptCommit::Tentative;
    assert!(bridge.transcript_revision(tentative).unwrap().is_none());
    assert!(
        bridge
            .transcript_revision(fragment(
                "request",
                5,
                "Prepare the Monday report",
                0,
                1_000_000
            ))
            .unwrap()
            .is_some()
    );
    assert_eq!(port.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn file_punctuation_and_case_revisions_are_substantive() {
    let (mut bridge, port, _directory) = fixture(false).await;
    bridge.transcript(fragment("file", 1, "Update README.md", 0, 1_000_000));
    bridge
        .trigger(typed(
            "file-work",
            json!({"action":"start","text":"Update README.md"}),
            Some("live:user:file"),
        ))
        .await
        .unwrap();
    let correction = bridge
        .transcript_revision(fragment("file", 2, "Update READMEmd", 0, 1_000_000))
        .unwrap()
        .unwrap();
    bridge.trigger(correction).await.unwrap();
    assert!(
        matches!(&port.calls.lock().unwrap()[1],VoiceWorkRequest::Steer{text,..} if text=="Update READMEmd")
    );
    assert!(
        bridge
            .transcript_revision(fragment("file", 3, "Update readmemd", 0, 1_000_000))
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn queued_revision_uses_exact_pending_input_and_never_starts_successor() {
    let (mut bridge, port, _directory) = fixture(true).await;
    bridge.transcript(fragment("queued", 1, "Write Friday report", 0, 1_000_000));
    bridge
        .trigger(typed(
            "queued-input",
            json!({"action":"start","text":"Write Friday report"}),
            Some("live:user:queued"),
        ))
        .await
        .unwrap();
    let correction = bridge
        .transcript_revision(fragment("queued", 2, "Write Thursday report", 0, 1_000_000))
        .unwrap()
        .unwrap();
    bridge.trigger(correction).await.unwrap();
    assert!(
        matches!(&port.calls.lock().unwrap()[1],VoiceWorkRequest::ReviseQueued{pending_input_id,expected_revision:1,text} if pending_input_id=="exact-pending-a"&&text.contains("Thursday"))
    );
    assert_eq!(
        port.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|request| matches!(request, VoiceWorkRequest::Start { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn media_backchannel_and_unbound_approval_never_become_new_work() {
    for text in ["先别念", "嗯", "我批准删除这个文件"] {
        let (mut bridge, port, _directory) = fixture(false).await;
        bridge.transcript(fragment("control", 1, text, 0, 1_000_000));
        let trigger = delegated("control", 1000);
        if text == "先别念" {
            assert_eq!(
                bridge.delegation_media_intent(&trigger).unwrap(),
                Some(VoiceBridgeMediaIntent::InterruptOutput)
            );
        }
        assert!(bridge.trigger(trigger).await.is_err());
        assert!(port.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn explicit_approval_requires_exact_named_case_and_new_presentation_input() {
    let (mut bridge, port, _directory) = fixture(false).await;
    let presentation = nomifun_voice_contracts::VoiceApprovalPresentation {
        question: "是否允许删除 README.md".into(),
        target: nomifun_voice_contracts::ApprovalTarget {
            work_target: target("A"),
            interaction_agent_session_id: "agent".into(),
            interaction_binding_version: 1,
            execution_id: "execution-a".into(),
            step_id: "step-a".into(),
            attempt_id: "attempt-a".into(),
            expected_execution_version: 1,
            expected_step_version: 1,
            expected_attempt_version: 1,
            request_event_sequence: 12,
            action_id: "delete-file".into(),
            question_digest: nomifun_agent_contracts::DigestHex("a".repeat(64)),
            presentation_id: "presentation-a".into(),
            presentation_digest: nomifun_agent_contracts::DigestHex("b".repeat(64)),
            interaction_mode: nomifun_voice_contracts::ApprovalInteractionMode::VoiceAllowed,
        },
    };
    bridge.present_approvals(vec![presentation.clone()]);
    bridge.transcript(fragment("answer", 1, "我批准删除 readme.md", 0, 1_000_000));
    let answer = nomifun_voice_contracts::VoiceApprovalAnswer {
        target: presentation.target.clone(),
        answer: "我批准删除 readme.md".into(),
        presented_context_id: presentation.target.presentation_id.clone(),
    };
    assert!(
        bridge
            .trigger(typed(
                "wrong-file-case",
                json!({"action":"answer_approval","answer":answer}),
                None
            ))
            .await
            .is_err()
    );
    let mut successor = presentation;
    successor.target.presentation_id = "presentation-b".into();
    bridge.present_approvals(vec![successor.clone()]);
    bridge.transcript(fragment("answer", 2, "我批准删除 README.md", 0, 1_000_000));
    let answer = nomifun_voice_contracts::VoiceApprovalAnswer {
        target: successor.target.clone(),
        answer: "我批准删除 README.md".into(),
        presented_context_id: successor.target.presentation_id.clone(),
    };
    assert!(
        bridge
            .trigger(typed(
                "old-input-new-decision",
                json!({"action":"answer_approval","answer":answer.clone()}),
                None
            ))
            .await
            .is_err()
    );
    assert!(port.calls.lock().unwrap().is_empty());
    bridge.transcript(fragment(
        "fresh-answer",
        1,
        "我批准删除 README.md",
        1_500_000,
        2_000_000,
    ));
    bridge
        .trigger(typed(
            "fresh-specific-answer",
            json!({"action":"answer_approval","answer":answer}),
            Some("live:user:fresh-answer"),
        ))
        .await
        .unwrap();
    assert!(
        matches!(&port.calls.lock().unwrap()[0],VoiceWorkRequest::AnswerApproval{answer} if answer.presented_context_id=="presentation-b")
    );
}

include!("target_proof_tests.rs");
