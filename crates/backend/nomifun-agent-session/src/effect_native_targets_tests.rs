use super::*;
use nomifun_agent_contracts::{NativeMutationNamespace, NativeMutationPlatform, NativeMutationSemantics, NativeMutationTarget, NativeMutationTargets};

fn target(object: Option<&str>, parent: &str, components: &[&str], ancestors: &[&str], subtree: bool) -> NativeMutationTarget {
    NativeMutationTarget {
        platform: NativeMutationPlatform::Windows,
        volume_id: "volume-main".into(),
        volume_wide: false,
        object_id: object.map(str::to_owned),
        namespace: NativeMutationNamespace { parent_id: parent.into(), components: components.iter().map(|part| part.encode_utf16().collect()).collect() },
        ancestor_ids: ancestors.iter().map(|id| (*id).into()).collect(),
        subtree,
        semantics: NativeMutationSemantics::Follow,
    }
}

fn footprint(targets: Vec<NativeMutationTarget>) -> NativeMutationTargets {
    NativeMutationTargets { version: 1, targets }
}

async fn request(store: &AgentSessionStore, key: &str, action: &str, domain: &str) -> EffectEventRequest {
    let operation = format!("native-operation-{key}");
    let (session, started) = create_turn(store, key, &operation).await;
    let capability = if action.starts_with("workspace.files/") { "workspace.files" } else { "workspace.process" };
    let tool = append(&session.agent_session_id, &format!("native-tool-{key}"), "runtime-supervisor", &format!("native-tool-{key}"),
        "tool/call-started", &format!("tool-{key}"), Some(started),
        json!({"operation_id":operation,"capability_id":capability,"action_id":action}));
    let tool = store.append_event(&tool).await.unwrap().ack.unwrap();
    EffectEventRequest {
        agent_session_id: session.agent_session_id, effect_id: format!("native-effect-{key}"),
        turn_id: operation.clone().into(), operation_id: operation.into(), owner_domain: domain.into(),
        capability_module: capability.into(), action_id: action.into(), resource_binding_id: None,
        resource_key: Some(format!("workspace:project-{key}")), input_digest: digest('6'), recorded_at: 100,
        event_id: format!("native-start-{key}").into(), producer_id: "capability_host".into(),
        idempotency_key: format!("native-lifecycle-{key}").into(), correlation_id: format!("native-effect-{key}").into(),
        strategy: EffectStrategy::ManagedEffect, causation_event_id: Some(tool.event_id),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
    }
}

fn settlement(request: &EffectEventRequest, suffix: &str, payload: serde_json::Value) -> EffectEventRequest {
    EffectEventRequest { event_id: format!("{}:{suffix}", request.effect_id).into(),
        causation_event_id: Some(request.event_id.clone()), producer_id: "runtime_supervisor".into(),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)), recorded_at: 101, ..request.clone() }
}

fn conflicted(result: Result<crate::SessionEventAppendResult, SessionStoreError>) {
    assert!(matches!(result, Err(SessionStoreError::NativeMutationTargetUnsettled { .. })));
}

#[tokio::test]
async fn native_effect_cross_project_aliases_and_execution_domains_share_pending_and_unknown_fence() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let first = request(&store, "native-external-first", "workspace.files/delete", "workspace").await;
    let alias = request(&store, "native-external-alias", "workspace.files/delete", "system:administrator").await;
    let disjoint = request(&store, "native-external-disjoint", "workspace.files/delete", "workspace").await;
    let original = footprint(vec![target(Some("volume:file-42"), "parent-one", &["FILE.TXT"], &["root", "parent-one"], false)]);
    let alias_targets = footprint(vec![target(Some("volume:file-42"), "different-parent", &["HARDLINK.TXT"], &["root", "different-parent"], false)]);
    store.record_effect_started_with_native_targets(first.clone(), original.clone()).await.unwrap();
    conflicted(store.record_effect_started_with_native_targets(alias.clone(), alias_targets.clone()).await);
    store.record_effect_started_with_native_targets(disjoint, footprint(vec![target(Some("volume:file-43"), "parent-one", &["OTHER.TXT"], &["root", "parent-one"], false)])).await.unwrap();
    store.record_effect_terminal(settlement(&first, "unknown", json!({"recovery":"process_restart_external_reconciliation_required"})), EffectTerminalState::Uncertain).await.unwrap();
    conflicted(store.record_effect_started_with_native_targets(alias.clone(), alias_targets.clone()).await);
    let reopened = AgentSessionStore::from_pool(store.test_pool().clone()).await.unwrap();
    conflicted(reopened.record_effect_started_with_native_targets(alias, alias_targets).await);
    let start: String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE event_id=?").bind(first.event_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&start).unwrap()["native_mutation_targets"], serde_json::to_value(original).unwrap());
    assert_eq!(store.read_effect(&first.agent_session_id, &first.effect_id).await.unwrap().unwrap().state, AgentEffectState::Unknown);
}

#[tokio::test]
async fn native_effect_two_independent_database_pools_have_one_atomic_winner_and_reopen_preserves_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("native-fence.sqlite");
    let db = nomifun_db::init_database(&path).await.unwrap();
    let first = AgentSessionStore::from_pool(db.pool().clone()).await.unwrap();
    let second = AgentSessionStore::connect_existing(&path).await.unwrap();
    let left = request(&first, "native-race-left", "workspace.files/write", "workspace").await;
    let right = request(&second, "native-race-right", "workspace.files/write", "workspace:administrator").await;
    let mut missing=target(None, "volume:pinned-parent", &["MISSING.TXT"], &["volume:root", "volume:pinned-parent"], false);
    missing.volume_wide=true;
    let shared = footprint(vec![missing]);
    let (left_result, right_result) = tokio::join!(
        first.record_effect_started_with_native_targets(left.clone(), shared.clone()),
        second.record_effect_started_with_native_targets(right.clone(), shared.clone()));
    assert_ne!(left_result.is_ok(), right_result.is_ok(), "two Store pools must not both admit the missing target");
    let (loser, winner) = if left_result.is_err() { conflicted(left_result); (left, right) } else { conflicted(right_result); (right, left) };
    first.test_pool().close().await;
    second.test_pool().close().await;
    drop(first); drop(second); db.close().await;
    let reopened = AgentSessionStore::connect_existing(&path).await.unwrap();
    conflicted(reopened.record_effect_started_with_native_targets(loser.clone(), shared.clone()).await);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM agent_effects WHERE state='pending'").fetch_one(reopened.test_pool()).await.unwrap(), 1);
    reopened.record_effect_terminal(settlement(&winner,"unknown-after-reopen",json!({"recovery":"process_restart_external_reconciliation_required"})),EffectTerminalState::Uncertain).await.unwrap();
    reopened.test_pool().close().await;
    let reopened_again=AgentSessionStore::connect_existing(&path).await.unwrap();
    conflicted(reopened_again.record_effect_started_with_native_targets(loser, shared).await);
    assert_eq!(reopened_again.read_effect(&winner.agent_session_id,&winner.effect_id).await.unwrap().unwrap().state,AgentEffectState::Unknown);
}

#[tokio::test]
async fn native_effect_rename_admits_all_targets_or_none_and_missing_parent_ranges_conflict() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let prior = request(&store, "native-rename-prior", "workspace.files/rename", "workspace").await;
    store.record_effect_started_with_native_targets(prior, footprint(vec![
        target(Some("prior-rename-source"),"prior-parent", &["PRIOR.TXT"], &["prior-parent"],false),
        target(None,"pinned-root", &["NEW", "DEST.TXT"], &["pinned-root"],false),
    ])).await.unwrap();
    let rename = request(&store, "native-rename", "workspace.files/rename", "workspace").await;
    let source = target(Some("source-object"), "source-parent", &["SOURCE.TXT"], &["source-parent"], false);
    let destination = target(None, "pinned-root", &["NEW"], &["pinned-root"], false);
    conflicted(store.record_effect_started_with_native_targets(rename.clone(), footprint(vec![source.clone(),destination])).await);
    assert!(store.read_effect(&rename.agent_session_id, &rename.effect_id).await.unwrap().is_none());
    let source_only = request(&store, "native-unreserved-source", "workspace.files/delete", "workspace").await;
    store.record_effect_started_with_native_targets(source_only, footprint(vec![source])).await.unwrap();
}

#[tokio::test]
async fn native_effect_directory_alias_ancestry_and_stream_base_identity_are_fenced() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let directory = request(&store,"native-directory","workspace.files/delete","workspace").await;
    let mut range = target(Some("directory-object"),"root", &["DIRECTORY"], &["root"],true);
    range.semantics = NativeMutationSemantics::Entry;
    store.record_effect_started_with_native_targets(directory.clone(),footprint(vec![range])).await.unwrap();
    let child = request(&store,"native-directory-alias-child","workspace.files/delete","workspace").await;
    conflicted(store.record_effect_started_with_native_targets(child,footprint(vec![target(Some("child-object"),"directory-object", &["FILE:STREAM"], &["root","directory-object"],false)])).await);
    let outside_alias=request(&store,"native-directory-external-hardlink","workspace.files/delete","workspace").await;
    conflicted(store.record_effect_started_with_native_targets(outside_alias,footprint(vec![target(Some("outside-object"),"outside-parent", &["ALIAS"], &["outside-parent"],false)])).await);
    let other_volume=request(&store,"native-directory-other-volume","workspace.files/delete","workspace").await;
    let mut disjoint=target(Some("other-volume-object"),"other-volume-parent", &["OTHER"], &["other-volume-parent"],false);
    disjoint.volume_id="other-volume".into();
    store.record_effect_started_with_native_targets(other_volume,footprint(vec![disjoint])).await.unwrap();
    store.record_effect_terminal(settlement(&directory,"failed",json!({"error":"known pre-dispatch rejection"})),EffectTerminalState::Failed).await.unwrap();
    let base = request(&store,"native-stream-base","workspace.files/delete","workspace").await;
    store.record_effect_started_with_native_targets(base,footprint(vec![target(Some("stream-base-object"),"unrelated-parent", &["BASE"], &["unrelated-parent"],false)])).await.unwrap();
    let stream = request(&store,"native-stream","workspace.files/delete","administrator").await;
    conflicted(store.record_effect_started_with_native_targets(stream,footprint(vec![target(Some("stream-base-object"),"alias-parent", &["BASE"], &["alias-parent"],false)])).await);
}

#[tokio::test]
async fn native_effect_legacy_absence_is_not_disjointness_and_opaque_process_is_not_precise_file_footprint() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let legacy = request(&store,"native-legacy","workspace.files/delete","workspace").await;
    store.record_effect_started(legacy.clone()).await.unwrap();
    let before: String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE event_id=?").bind(legacy.event_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    let next = request(&store,"native-legacy-next","workspace.files/delete","administrator").await;
    conflicted(store.record_effect_started_with_native_targets(next,footprint(vec![target(Some("entirely-other-object"),"other-parent", &["OTHER"], &["other-parent"],false)])).await);
    let process = request(&store,"native-opaque-process","workspace.process/exec","workspace").await;
    store.record_effect_started(process).await.unwrap();
    let backfill=settlement(&legacy,"forged-backfill",json!({"native_mutation_targets":footprint(vec![target(Some("invented-past-object"),"invented-parent", &["FILE"], &["invented-parent"],false)])}));
    assert!(matches!(store.record_effect_terminal(backfill,EffectTerminalState::Failed).await,Err(SessionStoreError::InvalidEvent(_))));
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT inline_json FROM agent_events WHERE event_id=?").bind(legacy.event_id.as_ref()).fetch_one(store.test_pool()).await.unwrap(),before);
    assert_eq!(store.read_effect(&legacy.agent_session_id,&legacy.effect_id).await.unwrap().unwrap().state,AgentEffectState::Pending);
}

#[tokio::test]
async fn native_effect_settlement_releases_fence_without_rewriting_original_targets_and_rejects_backfill() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let first = request(&store,"native-settle","workspace.files/delete","workspace").await;
    let facts = footprint(vec![target(Some("object"),"parent", &["FILE"], &["parent"],false)]);
    let first_result=store.record_effect_started_with_native_targets(first.clone(),facts.clone()).await.unwrap();
    assert!(store.record_effect_started_with_native_targets(first.clone(),facts.clone()).await.unwrap().duplicate);
    let changed = footprint(vec![target(Some("changed"),"parent", &["FILE"], &["parent"],false)]);
    assert!(matches!(store.record_effect_started_with_native_targets(first.clone(),changed.clone()).await,Err(SessionStoreError::IdempotencyConflict(_))));
    let forged=settlement(&first,"forged",json!({"native_mutation_targets":changed}));
    assert!(matches!(store.record_effect_terminal(forged,EffectTerminalState::Failed).await,Err(SessionStoreError::InvalidEvent(_))));
    store.record_effect_terminal(settlement(&first,"failed",json!({"error":"known pre-dispatch rejection"})),EffectTerminalState::Failed).await.unwrap();
    let next=request(&store,"native-settle-next","workspace.files/delete","administrator").await;
    store.record_effect_started_with_native_targets(next,facts).await.unwrap();
    assert!(first_result.ack.is_some());
}

#[tokio::test]
async fn native_effect_keeps_original_resource_uniqueness_and_rejects_incomplete_or_opaque_target_facts() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let first=request(&store,"native-resource-first","workspace.files/delete","workspace").await;
    let facts=footprint(vec![target(Some("first-object"),"parent", &["FIRST"], &["parent"],false)]);
    store.record_effect_started_with_native_targets(first.clone(),facts.clone()).await.unwrap();
    let mut second=request(&store,"native-resource-second","workspace.files/delete","workspace").await;
    second.resource_key=first.resource_key.clone();
    assert!(matches!(store.record_effect_started_with_native_targets(second,footprint(vec![target(Some("second-object"),"parent", &["SECOND"], &["parent"],false)])).await,Err(SessionStoreError::Sqlite(_))),"the original owner/resource unique fence is retained even for disjoint explicit targets");
    let process=request(&store,"native-opaque-forged","workspace.process/exec","workspace").await;
    assert!(matches!(store.record_effect_started_with_native_targets(process,facts).await,Err(SessionStoreError::InvalidEvent(_))));
    let incomplete=request(&store,"native-incomplete","workspace.files/delete","workspace").await;
    let mut bad=target(Some("third-object"),"parent", &["THIRD"], &["parent"],false);
    bad.ancestor_ids.clear();
    assert!(matches!(store.record_effect_started_with_native_targets(incomplete,footprint(vec![bad])).await,Err(SessionStoreError::InvalidEvent(_))));
    let incomplete_rename=request(&store,"native-incomplete-rename","workspace.files/rename","workspace").await;
    assert!(matches!(store.record_effect_started_with_native_targets(incomplete_rename,footprint(vec![target(Some("rename-source-only"),"parent", &["SOURCE"], &["parent"],false)])).await,Err(SessionStoreError::InvalidEvent(_))));
}

#[tokio::test]
async fn native_effect_publication_unknown_fences_unobserved_new_object_hardlinks_but_other_volumes_progress() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let publication=request(&store,"native-publication","workspace.files/write","workspace").await;
    let mut target_before_publication=target(None,"pinned-parent", &["NEW.TXT"], &["pinned-parent"],false);
    // A missing target has no FileID yet. Its atomic publication can produce a
    // different object which an external actor can hardlink before recovery.
    assert!(matches!(store.record_effect_started_with_native_targets(publication.clone(),footprint(vec![target_before_publication.clone()])).await,Err(SessionStoreError::InvalidEvent(_))));
    target_before_publication.volume_wide=true;
    store.record_effect_started_with_native_targets(publication.clone(),footprint(vec![target_before_publication])).await.unwrap();
    store.record_effect_terminal(settlement(&publication,"unknown",json!({"recovery":"process_restart_external_reconciliation_required"})),EffectTerminalState::Uncertain).await.unwrap();
    let external_alias=request(&store,"native-published-hardlink","workspace.files/delete","administrator").await;
    conflicted(store.record_effect_started_with_native_targets(external_alias,footprint(vec![target(Some("new-object-not-in-start-facts"),"unrelated-parent", &["HARDLINK"], &["unrelated-parent"],false)])).await);
    let disjoint=request(&store,"native-publication-other-volume","workspace.files/patch","administrator").await;
    let mut other_volume=target(Some("other-object"),"other-parent", &["FILE"], &["other-parent"],false);
    other_volume.volume_id="provably-other-volume".into();
    other_volume.volume_wide=true;
    store.record_effect_started_with_native_targets(disjoint,footprint(vec![other_volume])).await.unwrap();
}
