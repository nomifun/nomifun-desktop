//! In-session companion summon lifecycle tests (spec §设计 B4).
//!
//! Summon set/clear must require an idle conversation (409 otherwise), persist
//! `extra.summon` with a server-stamped `summoned_at`, and recycle the cached
//! runtime so the change takes effect on the next message — the same contract
//! as a knowledge-binding change.

use super::*;
use crate::service::summon::SetSummonRequest;

const SUMMON_COMPANION_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000001";
const SUMMON_MEMORY_ID_1: &str = "0190f5fe-7c00-7a00-8abc-000000000101";
const SUMMON_MEMORY_ID_2: &str = "0190f5fe-7c00-7a00-8abc-000000000102";

fn summon_service() -> (
    ConversationService,
    Arc<MockRepo>,
    Arc<MockAgentRuntimeRegistry>,
) {
    let repo = Arc::new(MockRepo::new());
    let broadcaster = Arc::new(MockBroadcaster::new());
    let registry_impl = Arc::new(MockAgentRuntimeRegistry::new());
    let runtime_registry: Arc<dyn AgentRuntimeRegistry> = registry_impl.clone();
    let svc = ConversationService::new(
        Arc::<str>::from(TEST_USER_1),
        std::env::temp_dir(),
        broadcaster,
        Arc::new(FixedSkillResolver { names: vec![] }),
        runtime_registry,
        repo.clone(),
        Arc::new(StubAgentMetadataRepo),
        Arc::new(StubAcpSessionRepo::default()),
        Arc::new(crate::NoExecutionConversationBoundary),
    );
    (svc, repo, registry_impl)
}

async fn create_nomi_conversation(svc: &ConversationService) -> String {
    let req: CreateConversationRequest = serde_json::from_value(json!({
        "type": "nomi",
        "model": { "provider_id": PROVIDER_ID_1, "model": "m1" },
        "extra": { "workspace": "/project" }
    }))
    .unwrap();
    svc.create(TEST_USER_1, req).await.unwrap().conversation_id
}

fn summon_request() -> SetSummonRequest {
    serde_json::from_value(json!({
        "companion_id": SUMMON_COMPANION_ID,
        "memory_ids": [SUMMON_MEMORY_ID_1, SUMMON_MEMORY_ID_2],
        "skill_exclusions": ["heavy-refactor"],
    }))
    .unwrap()
}

async fn extra_of(repo: &MockRepo, conversation_id: &str) -> serde_json::Value {
    let row = repo.get(conversation_id).await.unwrap().unwrap();
    serde_json::from_str(&row.extra).unwrap()
}

#[tokio::test]
async fn set_summon_stamps_persists_and_recycles_runtime_when_idle() {
    let (svc, repo, registry) = summon_service();
    let conversation_id = create_nomi_conversation(&svc).await;

    let config = svc
        .set_summon(TEST_USER_1, &conversation_id, summon_request())
        .await
        .unwrap();
    assert_eq!(config.companion_id, SUMMON_COMPANION_ID);
    assert_eq!(config.memory_ids, vec![SUMMON_MEMORY_ID_1, SUMMON_MEMORY_ID_2]);
    assert_eq!(config.skill_exclusions, vec!["heavy-refactor"]);
    assert!(config.summoned_at > 0, "server must stamp summoned_at");

    let extra = extra_of(&repo, &conversation_id).await;
    assert_eq!(extra["summon"], serde_json::to_value(&config).unwrap());
    assert_eq!(
        registry.termination_wait_count(),
        1,
        "summon must await runtime teardown so the change takes effect on the next message"
    );
    assert_eq!(
        registry.termination_records(),
        vec![(conversation_id.clone(), Some(AgentKillReason::ConfigurationChanged))],
        "summon recycles are deliberate config changes and must never be booked \
         as crashes by the restart governor"
    );
}

#[tokio::test]
async fn set_summon_rejects_persisted_running_conversation() {
    let (svc, repo, registry) = summon_service();
    let conversation_id = create_nomi_conversation(&svc).await;
    repo.update(
        &conversation_id,
        &ConversationRowUpdate {
            status: Some("running".to_owned()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let err = svc
        .set_summon(TEST_USER_1, &conversation_id, summon_request())
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Conflict(_)), "{err:?}");
    let extra = extra_of(&repo, &conversation_id).await;
    assert!(extra.get("summon").is_none(), "409 must not persist a summon");
    assert_eq!(registry.termination_wait_count(), 0);
}

#[tokio::test]
async fn set_summon_rejects_active_turn() {
    let (svc, repo, registry) = summon_service();
    let conversation_id = create_nomi_conversation(&svc).await;
    let _turn = svc.runtime_state().try_acquire_turn(&conversation_id).unwrap();

    let err = svc
        .set_summon(TEST_USER_1, &conversation_id, summon_request())
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Conflict(_)), "{err:?}");
    let extra = extra_of(&repo, &conversation_id).await;
    assert!(extra.get("summon").is_none());
    assert_eq!(registry.termination_wait_count(), 0);
}

#[tokio::test]
async fn clear_summon_removes_marker_and_recycles_once() {
    let (svc, repo, registry) = summon_service();
    let conversation_id = create_nomi_conversation(&svc).await;
    svc.set_summon(TEST_USER_1, &conversation_id, summon_request())
        .await
        .unwrap();

    svc.clear_summon(TEST_USER_1, &conversation_id).await.unwrap();
    let extra = extra_of(&repo, &conversation_id).await;
    assert!(extra.get("summon").is_none(), "clear must remove extra.summon");
    assert_eq!(registry.termination_wait_count(), 2, "set + clear each recycle once");

    // Clearing an already-clear conversation is idempotent and must not
    // disturb the runtime again.
    svc.clear_summon(TEST_USER_1, &conversation_id).await.unwrap();
    assert_eq!(registry.termination_wait_count(), 2);
}

#[tokio::test]
async fn clear_summon_rejects_active_turn() {
    let (svc, repo, _registry) = summon_service();
    let conversation_id = create_nomi_conversation(&svc).await;
    svc.set_summon(TEST_USER_1, &conversation_id, summon_request())
        .await
        .unwrap();
    let _turn = svc.runtime_state().try_acquire_turn(&conversation_id).unwrap();

    let err = svc.clear_summon(TEST_USER_1, &conversation_id).await.unwrap_err();
    assert!(matches!(err, AppError::Conflict(_)), "{err:?}");
    let extra = extra_of(&repo, &conversation_id).await;
    assert!(extra.get("summon").is_some(), "409 must not clear the summon");
}

#[tokio::test]
async fn set_summon_rejects_companion_conversations() {
    // A companion conversation already IS the companion; summoning inside it
    // would blur the persona boundary (persona 不接管).
    let (svc, repo, _registry) = summon_service();
    let conversation_id = create_nomi_conversation(&svc).await;
    let mut extra = extra_of(&repo, &conversation_id).await;
    extra["companion_session"] = json!(true);
    repo.update(
        &conversation_id,
        &ConversationRowUpdate {
            extra: Some(extra.to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let err = svc
        .set_summon(TEST_USER_1, &conversation_id, summon_request())
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
}

#[tokio::test]
async fn set_summon_validates_ids_and_ownership() {
    let (svc, _repo, _registry) = summon_service();
    let conversation_id = create_nomi_conversation(&svc).await;

    // Foreign user → NotFound (ownership fence).
    let err = svc
        .set_summon(TEST_USER_2, &conversation_id, summon_request())
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)), "{err:?}");

    // Malformed memory id → BadRequest.
    let bad: SetSummonRequest = serde_json::from_value(json!({
        "companion_id": SUMMON_COMPANION_ID,
        "memory_ids": ["not-a-memory-id"],
    }))
    .unwrap();
    let err = svc.set_summon(TEST_USER_1, &conversation_id, bad).await.unwrap_err();
    assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");

    // Malformed companion id → BadRequest.
    let bad: SetSummonRequest = serde_json::from_value(json!({
        "companion_id": "not-a-companion-id",
    }))
    .unwrap();
    let err = svc.set_summon(TEST_USER_1, &conversation_id, bad).await.unwrap_err();
    assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
}

#[tokio::test]
async fn set_summon_dedups_memory_ids_preserving_order() {
    let (svc, _repo, _registry) = summon_service();
    let conversation_id = create_nomi_conversation(&svc).await;
    let req: SetSummonRequest = serde_json::from_value(json!({
        "companion_id": SUMMON_COMPANION_ID,
        "memory_ids": [SUMMON_MEMORY_ID_2, SUMMON_MEMORY_ID_1, SUMMON_MEMORY_ID_2],
    }))
    .unwrap();
    let config = svc.set_summon(TEST_USER_1, &conversation_id, req).await.unwrap();
    assert_eq!(config.memory_ids, vec![SUMMON_MEMORY_ID_2, SUMMON_MEMORY_ID_1]);
}
