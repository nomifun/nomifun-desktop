use super::*;
use nomifun_agent_contracts::{
    AgentBindingValue, CompanionSessionRole, SemanticSessionEventDraft, SessionEventAppend,
    SessionEventKind, TypedResourceBinding,
};
use nomifun_conversation::CanonicalAgentSessionOwner;

fn principal() -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".into(),
        principal_id: uuid::Uuid::now_v7().to_string(),
    }
}

struct UnexpectedCompleter;
#[async_trait::async_trait]
impl nomifun_companion::learner::CompanionCompleter for UnexpectedCompleter {
    async fn complete(
        &self,
        _provider: &str,
        _model: &str,
        _system: &str,
        _user: &str,
        _max_tokens: Option<u32>,
    ) -> Result<String, AppError> {
        panic!("canonical shared context must not make a model call")
    }
}

async fn memory_service(
    owner: &PrincipalRef,
) -> (
    tempfile::TempDir,
    std::sync::Arc<nomifun_companion::CompanionService>,
    String,
) {
    let directory = tempfile::tempdir().unwrap();
    let service = nomifun_companion::CompanionService::start(
        directory.path(),
        std::sync::Arc::new(nomifun_realtime::BroadcastEventBus::new(16)),
        &owner.principal_id,
        std::sync::Arc::new(UnexpectedCompleter),
        std::sync::Arc::new(nomifun_skill_library::skill_service::resolve_skill_paths(
            directory.path(),
            directory.path(),
        )),
    )
    .await
    .unwrap();
    let companion = service
        .create_companion("Shared context fixture", "test persona")
        .await
        .unwrap()
        .companion_id;
    (directory, service, companion)
}

#[tokio::test]
async fn runtime_companion_handles_release_the_real_service_owner_and_fail_closed_after_drop() {
    let owner = principal();
    let (_directory, service, _) = memory_service(&owner).await;
    let assembly_handle = std::sync::Arc::downgrade(&service);
    let session_handle = assembly_handle.clone();
    let observation_owner =
        crate::router::engine_kernel_session::companion_owner(&session_handle).unwrap();
    assert!(std::sync::Arc::ptr_eq(&observation_owner, &service));
    drop(observation_owner);
    assert_eq!(
        std::sync::Arc::strong_count(&service),
        1,
        "weak Runtime handles cannot retain the AppServices lifecycle owner"
    );
    drop(service);
    assert!(assembly_handle.upgrade().is_none());
    assert!(
        crate::router::engine_kernel_session::companion_owner(&session_handle).is_err(),
        "a vanished memory owner is unavailable, not an empty or unauthorized context"
    );
}

fn binding(owner: &PrincipalRef, companion: &str, robot: Option<&str>) -> AgentBindingValue {
    let mut resources = ["companion", "companion_memory"]
        .into_iter()
        .map(|kind| TypedResourceBinding {
            binding_id: format!("{kind}-binding").into(),
            resource_kind: kind.into(),
            resource_id: companion.into(),
            owner_id: owner.principal_id.clone(),
            operations: ["read".into()].into(),
            connection_config_ref: None,
            typed_parameters: Default::default(),
        })
        .collect::<Vec<_>>();
    if let Some(robot) = robot {
        resources.push(TypedResourceBinding {
            binding_id: "robot-binding".into(),
            resource_kind: "robot".into(),
            resource_id: robot.into(),
            owner_id: owner.principal_id.clone(),
            operations: ["vision".into()].into(),
            connection_config_ref: None,
            typed_parameters: Default::default(),
        });
    }
    AgentBindingValue {
        preset_revision_ref: nomifun_agent_contracts::PresetRevisionRef {
            preset_id: "shared-context-fixture".into(),
            revision: 1,
            revision_digest: "a".repeat(64).into(),
        },
        resolved_snapshot_ref: nomifun_agent_contracts::ResolvedSnapshotRef {
            snapshot_id: "shared-context-fixture".into(),
            snapshot_digest: "b".repeat(64).into(),
        },
        typed_resource_bindings: resources,
        binding_version: 1,
    }
}

async fn open(
    canonical: &CanonicalAgentSessionOwner,
    owner: &PrincipalRef,
    companion: &str,
    robot: Option<&str>,
) -> AgentSessionId {
    canonical
        .open_with_companion_scope(
            owner.clone(),
            binding(owner, companion, robot),
            None,
            Vec::new(),
            CompanionSessionOrigin {
                companion_id: companion.into(),
                role: if robot.is_some() {
                    CompanionSessionRole::Robot
                } else {
                    CompanionSessionRole::Companion
                },
                robot_id: robot.map(str::to_owned),
            },
            &format!("shared-context-{}", uuid::Uuid::now_v7()),
            1,
        )
        .await
        .unwrap()
        .session
        .agent_session_id
}

#[tokio::test]
async fn robot_context_role_uses_canonical_opening_and_the_exact_current_snapshot() {
    let database=nomifun_db::init_database_memory().await.unwrap();
    let canonical=CanonicalAgentSessionOwner::from_pool(database.pool().clone()).await.unwrap();
    let owner=principal();
    let companion=uuid::Uuid::now_v7().to_string();
    let robot=open(&canonical,&owner,&companion,Some("physical-robot")).await;
    let desktop=open(&canonical,&owner,&companion,None).await;
    let expected=binding(&owner,&companion,None).resolved_snapshot_ref;
    for (session,expected_role) in [(&robot,CompanionSessionRole::Robot),(&desktop,CompanionSessionRole::Companion)] {
        let role=read_product_origin_at_snapshot(canonical.store(),&owner,session,false,Some(&expected)).await.unwrap().unwrap();
        assert_eq!(role.role,expected_role);
    }
    let mut stale=expected.clone();
    stale.snapshot_digest="c".repeat(64).into();
    assert!(read_product_origin_at_snapshot(canonical.store(),&owner,&robot,false,Some(&stale)).await.is_err(),
        "a new role proof cannot authorize another frozen Attempt snapshot");
    let mut spoofed=binding(&owner,&companion,None);
    spoofed.typed_resource_bindings[0].typed_parameters.insert("channel_platform".into(),"robot".into());
    let ordinary=canonical.open(owner.clone(),spoofed,None,Vec::new(),"ordinary-param-spoof",2).await.unwrap().session.agent_session_id;
    assert!(read_product_origin_at_snapshot(canonical.store(),&owner,&ordinary,false,Some(&expected)).await.unwrap().is_none(),
        "an explicit Persona resource parameter does not enroll a physical product role");
    assert!(read_product_origin_at_snapshot(canonical.store(),&principal(),&robot,false,Some(&expected)).await.is_err());
}

async fn user_input(
    store: &AgentSessionStore,
    id: &AgentSessionId,
    content: &str,
    source: Option<AgentInputSource>,
) {
    let key = uuid::Uuid::now_v7().to_string();
    let snapshot = store
        .get_live_session(id)
        .await
        .unwrap()
        .agent_binding
        .resolved_snapshot_ref;
    store
        .start_turn(
            id,
            "session_api".into(),
            format!("{key}:input").into(),
            key.clone().into(),
            StrictJsonValue(json!({"content": content, "files":[],"inject_skills":[], "hidden": false,
            "admission":{"route_identity":nomifun_agent_contracts::ChatRouteIdentity::new(
                "shared-context-fixture@1","agent_chat","fixture-route".into(),1),"resolved_snapshot_ref":snapshot},
            "origin": source.as_ref().map(|_| "robot"), "input_source": source})),
        )
        .await
        .unwrap();
    store
        .cancel_active_turn(id, format!("{key}:close").into(), "session_api".into())
        .await
        .unwrap();
}

async fn projected(store: &AgentSessionStore, id: &AgentSessionId, content: &str) {
    projected_payload(
        store,
        id,
        json!({"content":content,"origin":"agent_execution",
        "operation_id":uuid::Uuid::now_v7().to_string()}),
    )
    .await;
}

async fn projected_payload(store: &AgentSessionStore, id: &AgentSessionId, payload: Value) {
    let key = uuid::Uuid::now_v7().to_string();
    let page = store
        .read_events(
            id,
            Some(&nomifun_agent_contracts::SessionEventCursor {
                agent_session_id: id.clone(),
                seq: store.head(id).await.unwrap().last_seq.saturating_sub(1),
            }),
            1,
        )
        .await
        .unwrap();
    store
        .append_event(&SessionEventAppend {
            agent_session_id: id.clone(),
            event_id: key.clone().into(),
            producer_id: "session_api".into(),
            idempotency_key: format!("projected:{key}").into(),
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("message/assistant-projected".into()),
                kind_version: 1,
                correlation_id: key.clone().into(),
                causation_event_id: Some(page.events[0].event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
            },
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn generic_content_only_and_failure_projections_keep_metadata_while_corrupt_typed_reports_fail_closed()
 {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let canonical = CanonicalAgentSessionOwner::from_pool(database.pool().clone())
        .await
        .unwrap();
    let store = canonical.store();
    let owner = principal();
    let (_directory, service, companion) = memory_service(&owner).await;
    let current = open(&canonical, &owner, &companion, None).await;
    let source = open(&canonical, &owner, &companion, None).await;
    user_input(store, &source, "A real source statement", None).await;
    projected_payload(
        store,
        &source,
        json!({"content":"GENERIC_CONTENT_ONLY_BODY"}),
    )
    .await;
    projected_payload(
        store,
        &source,
        json!({"content":"VALID_FAILURE_NOTICE_BODY","origin":"directed_result_failure",
        "operation_id":"result-continuation:failure-notice"}),
    )
    .await;
    let recent = recent_context(
        database.pool(),
        store,
        &service,
        &owner,
        &current,
        &companion,
    )
    .await
    .unwrap();
    let notices = recent[0]["domain_results"].as_array().unwrap();
    assert_eq!(notices.len(), 2);
    assert!(
        notices
            .iter()
            .all(|notice| notice["content_omitted_without_source_refs"] == true)
    );
    let encoded = serde_json::to_string(&recent).unwrap();
    assert!(!encoded.contains("GENERIC_CONTENT_ONLY_BODY"));
    assert!(!encoded.contains("VALID_FAILURE_NOTICE_BODY"));
    projected_payload(
        store,
        &source,
        json!({"content":"CORRUPT_REPORT_BODY","origin":"agent_execution",
        "operation_id":"typed-report","delegation_report":{"source":false}}),
    )
    .await;
    assert!(
        recent_context(
            database.pool(),
            store,
            &service,
            &owner,
            &current,
            &companion
        )
        .await
        .is_err(),
        "an explicit malformed typed report cannot downgrade into an ordinary notice"
    );
}

#[tokio::test]
async fn canonical_recent_context_shares_new_inputs_across_roles_without_projection_or_scope_leaks()
{
    let database = nomifun_db::init_database_memory().await.unwrap();
    let canonical = CanonicalAgentSessionOwner::from_pool(database.pool().clone())
        .await
        .unwrap();
    let store = canonical.store();
    let owner = principal();
    let (_memory_directory, service, companion) = memory_service(&owner).await;
    let desktop = open(&canonical, &owner, &companion, None).await;
    let robot = open(&canonical, &owner, &companion, Some("physical-robot")).await;
    let other = open(&canonical, &owner, &uuid::Uuid::now_v7().to_string(), None).await;
    user_input(
        store,
        &robot,
        "明天改成九点出发",
        Some(AgentInputSource::AuthenticatedRobot {
            companion_id: companion.clone(),
            robot_id: "physical-robot".into(),
            connection_id: "live-connection".into(),
            presented_question_id: None,
        }),
    )
    .await;
    user_input(store, &other, "OTHER_COMPANION_SECRET", None).await;
    // A selected resource alone cannot enroll an ordinary Session in product
    // history. This is the same frozen data but a different canonical opening.
    let ordinary = canonical
        .open(
            owner.clone(),
            binding(&owner, &companion, None),
            None,
            Vec::new(),
            "ordinary-same-resource",
            2,
        )
        .await
        .unwrap()
        .session
        .agent_session_id;
    user_input(store, &ordinary, "ORDINARY_SESSION_SECRET", None).await;
    for index in 0..6 {
        projected(store, &robot, &format!("Desktop task notice {index}")).await;
    }
    // Corrupting disposable UI data must not supply any shared context.
    sqlx::query("UPDATE agent_messages SET projection_json=? WHERE session_id=?")
        .bind("{\"content\":\"PROJECTION_ONLY_SECRET\"}")
        .bind(robot.as_ref())
        .execute(database.pool())
        .await
        .unwrap();
    let recent = recent_context(
        database.pool(),
        store,
        &service,
        &owner,
        &desktop,
        &companion,
    )
    .await
    .unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0]["agent_session_id"], robot.as_ref());
    assert_eq!(
        recent[0]["user_inputs"].as_array().unwrap().len(),
        1,
        "many terminal/domain notices cannot evict the most recent real input"
    );
    let rendered = serde_json::to_string(&recent).unwrap();
    assert!(rendered.contains("明天改成九点出发"));
    assert!(rendered.contains("authenticated_robot"));
    assert!(!rendered.contains("OTHER_COMPANION_SECRET"));
    assert!(!rendered.contains("ORDINARY_SESSION_SECRET"));
    assert!(!rendered.contains("PROJECTION_ONLY_SECRET"));
    assert!(product_origin(store, &principal(), &desktop).await.is_err());

    // Publish and then explicitly forget a genuinely provenance-linked memory
    // through the real Companion store/service. Its original canonical speech
    // must not reappear through the shared recent/history readers.
    let source_id:String=sqlx::query_scalar("SELECT source_message_id FROM agent_turns WHERE session_id=? ORDER BY accepted_at DESC LIMIT 1")
        .bind(robot.as_ref()).fetch_one(database.pool()).await.unwrap();
    let memory_store = nomifun_companion::CompanionStore::open(
        &_memory_directory
            .path()
            .join(nomifun_companion::COMPANION_SHARED_REL_DIR),
        Some(&companion),
    )
    .await
    .unwrap();
    let generation = memory_store.memory_generation(&companion).await.unwrap();
    memory_store
        .apply_learning_batch(
            &companion,
            &generation,
            &[],
            &[],
            &[nomifun_companion::LearnedMemoryCandidate {
                kind: "profile".into(),
                content: "明天改成九点出发".into(),
                tags: Vec::new(),
                importance: 0.8,
                source_event_ids: vec![nomifun_common::CompanionEventId::new().into_string()],
                source_message_refs: vec![nomifun_companion::store::CompanionMemoryMessageSource {
                    session_id: robot.as_ref().into(),
                    message_id: source_id,
                }],
            }],
        )
        .await
        .unwrap();
    let memory = memory_store
        .dump_memories_all()
        .await
        .unwrap()
        .into_iter()
        .find(|memory| !memory.source_message_refs.is_empty())
        .unwrap();
    service
        .delete_memory(
            &memory.memory_id,
            &nomifun_companion::store::MemoryActor::Companion(companion.clone()),
        )
        .await
        .unwrap();
    let forgotten = recent_context(
        database.pool(),
        store,
        &service,
        &owner,
        &desktop,
        &companion,
    )
    .await
    .unwrap();
    assert!(
        !serde_json::to_string(&forgotten)
            .unwrap()
            .contains("明天改成九点出发")
    );
    let recall = recall_companion_history(
        database.pool(),
        store,
        &service,
        &owner.principal_id,
        &companion,
        nomifun_companion::CompanionHistoryRecallQuery {
            query: "九点".into(),
            limit: 3,
            byte_budget: 4096,
            source_session_id: Some(robot.as_ref().into()),
            since_ts: None,
            until_ts: None,
            before: None,
        },
    )
    .await
    .unwrap();
    assert!(recall.fragments.is_empty());

    // A later committed statement is visible without waiting for a learner or
    // reopening either Runtime; clearing its source removes its shared tail.
    user_input(store, &robot, "取消明天的出行安排", None).await;
    let updated = recent_context(
        database.pool(),
        store,
        &service,
        &owner,
        &desktop,
        &companion,
    )
    .await
    .unwrap();
    assert!(
        serde_json::to_string(&updated)
            .unwrap()
            .contains("取消明天的出行安排")
    );
    let clear = nomifun_agent_session::ClearSessionContext {
        idempotency_key: "clear-robot-context".into(),
        reason: "user_requested".into(),
    };
    let nomifun_agent_session::ContextClearPreparation::Ready(boundary) = store
        .preflight_context_clear(&owner, &robot, &clear)
        .await
        .unwrap()
    else {
        panic!("fresh clear")
    };
    store
        .clear_context(&owner, &robot, &clear, &boundary)
        .await
        .unwrap();
    assert!(
        recent_context(
            database.pool(),
            store,
            &service,
            &owner,
            &desktop,
            &companion
        )
        .await
        .unwrap()
        .is_empty()
    );
}

#[test]
fn typed_input_source_preserves_human_origin_and_rejects_host_or_forged_robot_speech() {
    let scope = CompanionSessionOrigin {
        companion_id: "companion".into(),
        role: CompanionSessionRole::Robot,
        robot_id: Some("robot".into()),
    };
    let input = |value| serde_json::from_value::<AcceptedInput>(value).unwrap();
    assert!(input(json!({"content":"user"})).is_user_speech(&scope));
    assert!(input(json!({"content":"production receipt","files":["notes.txt"],"inject_skills":[],
        "admission":{"route_identity":{"provider":"fixture"},"resolved_snapshot_ref":{"snapshot_id":"snapshot"}},
        "idmm_decision":{"host_owned":"not part of the client request"}})).is_user_speech(&scope));
    assert!(!input(json!({"content":"wake","origin":"cron"})).is_user_speech(&scope));
    assert!(!input(json!({"content":"hidden","hidden":true})).is_user_speech(&scope));
    assert!(!input(json!({"content":"forged","origin":"robot"})).is_user_speech(&scope));
    assert!(!input(json!({"content":"other","origin":"robot","input_source":{
        "kind":"authenticated_robot","companion_id":"other","robot_id":"robot","connection_id":"connection"}})).is_user_speech(&scope));
    assert!(input(json!({"content":"spoken","origin":"robot","input_source":{
        "kind":"authenticated_robot","companion_id":"companion","robot_id":"robot","connection_id":"connection"}})).is_user_speech(&scope));
}

#[test]
fn retired_result_source_is_strict_readonly_history_and_never_human_or_live_ingress() {
    let source = json!({"kind":"directed_delegation_result","report_message_id":uuid::Uuid::now_v7().to_string(),
        "report_seq":12,"execution_id":uuid::Uuid::now_v7().to_string(),"source_turn_operation_id":"original-turn"});
    let input = json!({"content":"recorded result data","hidden":true,"origin":"directed_delegation_result",
        "channel_platform":"robot","input_source":source});
    let parsed: AcceptedInput = serde_json::from_value(input.clone()).unwrap();
    let scope = CompanionSessionOrigin {companion_id:"same-companion".into(),role:CompanionSessionRole::Robot,robot_id:Some("device".into())};
    assert!(!parsed.is_user_speech(&scope));
    assert_eq!(historical_input_source_wire(&input).unwrap(),Some(source.clone()));
    assert!(serde_json::from_value::<AgentInputSource>(source).is_err(),"the removed callback remains forbidden at live ingress");
    for path in ["hidden","origin","channel_platform","report_message_id","report_seq","execution_id","source_turn_operation_id","unknown"] {
        let mut invalid=input.clone();
        match path {
            "hidden"=>invalid[path]=false.into(),
            "origin"|"channel_platform"=>invalid[path]="other".into(),
            "report_seq"=>invalid["input_source"][path]=0.into(),
            "unknown"=>invalid["input_source"][path]=true.into(),
            _=>invalid["input_source"][path]="".into(),
        }
        assert!(serde_json::from_value::<AcceptedInput>(invalid).is_err(),"{path}");
    }
}

#[tokio::test]
async fn retired_hidden_result_history_proves_report_and_original_human_without_mutating_facts() {
    use nomifun_agent_contracts::{OperationId, canonical_assistant_step_message_id, digest_bytes};
    use nomifun_agent_session::{EffectEventRequest,EffectStrategy,EffectTerminalState};
    let database=nomifun_db::init_database_memory().await.unwrap();
    let canonical=CanonicalAgentSessionOwner::from_pool(database.pool().clone()).await.unwrap();
    let store=canonical.store();let owner=principal();
    let (_directory,service,companion)=memory_service(&owner).await;
    let session=open(&canonical,&owner,&companion,Some("historical-device")).await;
    let current=open(&canonical,&owner,&companion,None).await;
    let parent=OperationId::from("historical-parent");
    let (human,parent_start)=store.start_turn(&session,"session_api".into(),"historical-human".into(),parent.clone(),
        StrictJsonValue(json!({"content":"historical original request","hidden":false}))).await.unwrap();
    let human=human.ack.unwrap().event_id;let parent_start=parent_start.ack.unwrap().event_id;
    let append=|id:&str,producer:&str,key:&str,kind:&str,correlation:&str,cause:nomifun_agent_contracts::EventId,payload:Value|SessionEventAppend {
        agent_session_id:session.clone(),event_id:id.into(),producer_id:producer.into(),idempotency_key:key.into(),
        semantic_event:SemanticSessionEventDraft {kind:SessionEventKind(kind.into()),kind_version:1,correlation_id:correlation.into(),
            causation_event_id:Some(cause),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(payload))},
    };
    let tool=store.append_event(&append("historical-tool","runtime_supervisor","historical-tool","tool/call-started","historical-call",parent_start.clone(),
        json!({"operation_id":"historical-tool-op","capability_id":"agent.collaboration","action_id":"agent/delegate","call_id":"historical-call"})))
        .await.unwrap().ack.unwrap().event_id;
    let effect=EffectEventRequest {agent_session_id:session.clone(),effect_id:"historical-effect".into(),turn_id:parent.clone(),
        operation_id:"historical-tool-op".into(),owner_domain:"wave5.product".into(),capability_module:"agent.collaboration".into(),
        action_id:"agent/delegate".into(),resource_binding_id:None,resource_key:None,input_digest:"c".repeat(64).into(),recorded_at:1,
        event_id:"historical-effect-start".into(),producer_id:"capability_host".into(),idempotency_key:"historical-effect-start".into(),
        correlation_id:"historical-effect".into(),strategy:EffectStrategy::ManagedEffect,causation_event_id:Some(tool),
        payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({})))};
    let effect_start=store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap().event_id;
    // Production retains the lifecycle key and changes its producer at the
    // capability_host -> owning_plugin handoff; (producer, key) is unique.
    store.record_effect_terminal(EffectEventRequest {event_id:"historical-effect-return".into(),producer_id:"owning_plugin".into(),
        recorded_at:2,causation_event_id:Some(effect_start),..effect},EffectTerminalState::Succeeded).await.unwrap();
    let parent_terminal=store.append_turn_terminal(&append("historical-parent-done","runtime_supervisor","historical-parent-done","turn/completed",parent.as_ref(),parent_start,
        json!({})),&parent).await.unwrap().ack.unwrap().event_id;
    let execution=uuid::Uuid::now_v7().to_string();let report_id=uuid::Uuid::now_v7().to_string();
    let report_operation=format!("exec-lead-report:{execution}:event:7");
    let source=json!({"agent_session_id":session,"turn_operation_id":parent,"tool_operation_id":"historical-tool-op","effect_id":"historical-effect"});
    sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,adaptation_policy,decision_policy,delegation_policy,max_parallel,initial_plan_input,created_at,updated_at) VALUES(?,?,'historical request','completed','adaptive','ask_user','disabled',1,?,1,1)")
        .bind(&execution).bind(&owner.principal_id).bind(json!({"mode":"directed","source":source,"plan":{"steps":[]}}).to_string()).execute(database.pool()).await.unwrap();
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,created_at,updated_at) VALUES(?,?,'lead',1,1)")
        .bind(session.as_ref()).bind(&execution).execute(database.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,7,'status_changed','system',?,?,1)")
        .bind(&execution).bind(&owner.principal_id).bind(json!({"status":"completed","lead_report_operation_id":report_operation}).to_string()).execute(database.pool()).await.unwrap();
    let report=store.append_event(&append(&report_id,"session_api",&format!("agent-execution-projection:{report_operation}"),"message/assistant-projected",&report_id,parent_terminal,
        json!({"content":"historical recorded result","origin":"agent_execution_report","operation_id":report_operation,
            "delegation_report":{"source":source,"execution_id":execution,"execution_event_sequence":7,"status":"completed","attempts":[]}})))
        .await.unwrap().ack.unwrap();
    let retired=json!({"kind":"directed_delegation_result","report_message_id":report_id,"report_seq":report.seq,
        "execution_id":execution,"source_turn_operation_id":parent});
    let input=json!({"content":json!({"kind":"directed_delegation_result","execution_id":execution,"status":"completed",
        "report_message_id":report_id,"report_seq":report.seq,"result":"historical recorded result","result_truncated":false}).to_string(),
        "hidden":true,"origin":"directed_delegation_result","channel_platform":"robot","input_source":retired});
    let hidden=OperationId::from("historical-hidden-result");
    let (hidden_root,hidden_start)=store.start_turn(&session,"session_api".into(),"historical-hidden".into(),hidden.clone(),StrictJsonValue(input.clone())).await.unwrap();
    let hidden_root=hidden_root.ack.unwrap().event_id;let hidden_start=hidden_start.ack.unwrap().event_id;
    store.append_event(&append("historical-model-step","runtime_supervisor","historical-model-step","runtime/progress-recorded",hidden.as_ref(),hidden_root.clone(),
        json!({"producer_seq":0,"event":{"event":"model_step_started","step":1,"operation_id":format!("{}:model:1",hidden.as_ref())}}))).await.unwrap();
    let message=canonical_assistant_step_message_id(hidden_root.as_ref(),1).unwrap();let answer="historical sealed result";
    let part=store.append_event(&append("historical-answer-part","runtime_supervisor","historical-answer-part","message/content-part",&message,hidden_root.clone(),json!({"content":answer,"turn_id":hidden_root}))).await.unwrap().ack.unwrap().event_id;
    store.append_event(&append("historical-answer-complete","runtime_supervisor","historical-answer-complete","message/completed",&message,part,
        json!({"part_count":1,"content_digest":digest_bytes(answer.as_bytes())}))).await.unwrap();
    store.append_turn_terminal(&append("historical-hidden-done","runtime_supervisor","historical-hidden-done","turn/completed",hidden.as_ref(),hidden_start,
        json!({"model_steps":1})),&hidden).await.unwrap();
    let before=store.read_events(&session,None,500).await.unwrap();
    let sources=history::resolve_companion_turn_human_sources(database.pool(),store,&owner,&companion,&session,hidden.as_ref(),i64::MAX as u64).await.unwrap().unwrap();
    assert_eq!(sources,vec![nomifun_companion::store::CompanionMemoryMessageSource {session_id:session.as_ref().into(),message_id:human.as_ref().into()}]);
    let recent=recent_context(database.pool(),store,&service,&owner,&current,&companion).await.unwrap();
    assert!(serde_json::to_string(&recent).unwrap().contains(answer));
    let recalled=recall_companion_history(database.pool(),store,&service,&owner.principal_id,&companion,
        nomifun_companion::CompanionHistoryRecallQuery {query:"historical".into(),limit:12,byte_budget:16*1024,
            source_session_id:Some(session.as_ref().into()),since_ts:None,until_ts:None,before:None}).await.unwrap();
    let recalled_answer=recalled.fragments.iter().find(|fragment|fragment.turn_id==hidden.as_ref())
        .expect("the exact hidden result Turn must remain readable as assistant history");
    assert_eq!(recalled_answer.text,format!("Verified-source background task response (historical data):\nSealed assistant response (historical data): {answer}"),
        "recall retains the established data-only wrapper instead of returning bare model text");
    assert_eq!(recalled_answer.event_ids,vec![hidden_root.as_ref().to_owned(),"historical-answer-complete".into(),"historical-hidden-done".into()],
        "the recalled answer must identify its accepted input, canonical seal and exact terminal");
    assert!(!recalled.fragments.iter().any(|fragment|fragment.text.contains("result_truncated")),"the retired system input is never promoted to human speech");
    assert_eq!(store.read_events(&session,None,500).await.unwrap(),before,"reading old history must preserve all canonical bytes and events");
    let scope=CompanionSessionOrigin {companion_id:companion,role:CompanionSessionRole::Robot,robot_id:Some("historical-device".into())};
    for field in ["report_message_id","report_seq","execution_id","source_turn_operation_id"] {
        let mut forged=input.clone();forged["input_source"][field]=match field {
            "report_seq"=>json!(report.seq+1),"source_turn_operation_id"=>json!("another-parent"),_=>json!(uuid::Uuid::now_v7().to_string()),
        };
        let forged:AcceptedInput=serde_json::from_value(forged).unwrap();
        let HistoricalInputSource::RetiredResult(source)=forged.input_source.as_ref().unwrap() else{panic!("retired source expected")};
        let proof=history::retired_result_dependency(database.pool(),store,&owner,&session,&scope,&forged,before.next_cursor.seq,source).await;
        assert!(!matches!(proof,Ok(Some(_))),"{field} cannot attribute another report to the original human");
    }
}

#[test]
fn combined_observation_budget_keeps_current_memory_and_emits_valid_json_for_escaped_unicode() {
    let current = json!({"memory":{"fingerprint":42,"text":"用户更正：九点"}, "recent_sessions":
        (0..4).map(|_| json!({"text":"\\\"\n中文".repeat(1000)})).collect::<Vec<_>>()});
    let vision = json!({"objects":"物理观察".repeat(1600)}).to_string();
    let result = compose_live_observation(Some(current), Some(vision))
        .unwrap()
        .unwrap();
    assert!(result.len() <= LIVE_CONTEXT_BYTES);
    assert!(
        serde_json::to_vec(&Some(result.as_str())).unwrap().len() + LIVE_WRAPPER_RESERVE
            <= LIVE_CONTEXT_BYTES
    );
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["companion"]["memory"]["fingerprint"], 42);
    assert_eq!(parsed["companion"]["memory"]["text"], "用户更正：九点");
    assert_eq!(parsed["recent_context_truncated"], true);
    let latest = compose_live_observation(
        Some(json!({"memory":{"fingerprint":43,"text":"用户更正：取消出行"}})),
        None,
    )
    .unwrap()
    .unwrap();
    assert!(!latest.contains("九点"));
    assert!(latest.contains("取消出行"));
}

#[test]
fn stored_canonical_source_payload_rejects_cross_session_and_digest_damage() {
    let body = json!({"content":"trusted canonical input"});
    let bytes = nomifun_agent_contracts::canonical_json_bytes(&body).unwrap();
    let mut source = SourceEvent {
        seq: 1,
        event_id: "source".into(),
        kind: "message/user-accepted".into(),
        kind_version: 1,
        correlation_id: "message".into(),
        producer_id: "session_api".into(),
        idempotency_key: "source".into(),
        causation_event_id: None,
        inline_json: None,
        payload_id: Some("payload".into()),
        payload_session_id: Some("session".into()),
        payload_body: Some(
            serde_json::to_vec(&SessionPayloadBody::Json(StrictJsonValue(body.clone()))).unwrap(),
        ),
        payload_digest: Some(
            nomifun_agent_contracts::digest_bytes(&bytes)
                .as_ref()
                .into(),
        ),
        payload_byte_len: Some(bytes.len() as i64),
    };
    assert_eq!(source.value("session").unwrap(), body);
    assert!(source.value("different-session").is_err());
    source.payload_digest = Some("0".repeat(64));
    assert!(source.value("session").is_err());
}

#[test]
fn disappearing_sibling_sources_are_skipped_but_current_session_and_storage_errors_fail_closed() {
    assert!(
        source_read::<()>(Err(SessionStoreError::Deleted("sibling".into())), true)
            .unwrap()
            .is_none()
    );
    assert!(
        source_read::<()>(Err(SessionStoreError::NotFound("sibling".into())), true)
            .unwrap()
            .is_none()
    );
    assert!(source_read::<()>(Err(SessionStoreError::Deleted("current".into())), false).is_err());
    assert!(
        source_read::<()>(
            Err(SessionStoreError::InvalidPayload("damaged source".into())),
            true
        )
        .is_err()
    );
    assert!(
        source_read::<()>(
            Err(SessionStoreError::Sqlite(sqlx::Error::PoolClosed)),
            true
        )
        .is_err()
    );
}

#[tokio::test]
async fn ordinary_public_answer_uses_exact_closed_turn_seals_and_never_prior_step_narration() {
    use crate::router::engine_journal::{EngineJournalWrite, test_fixture};
    use history::PublicAnswerRead;
    use nomifun_agent_runtime::AgentEngineEvent;
    for (final_step, expected) in [(2, Some("Actual final answer")), (3, None)] {
        let (journal, pool) = test_fixture().await;
        let store = AgentSessionStore::from_pool(pool.clone()).await.unwrap();
        let id: String = sqlx::query_scalar("SELECT agent_session_id FROM agent_sessions LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let session = AgentSessionId::from(id);
        journal
            .append(
                serde_json::to_string(&AgentEngineEvent::OutputTextDelta {
                    step: 1,
                    text: "Earlier narration is not the final result".into(),
                    source: None,
                })
                .unwrap(),
                None,
                EngineJournalWrite::Progress,
            )
            .await
            .unwrap();
        assert!(matches!(
            history::closed_turn_answer(&pool, &store, &session, "turn", 0)
                .await
                .unwrap(),
            PublicAnswerRead::Unavailable
        ));
        journal
            .append(
                serde_json::to_string(&AgentEngineEvent::OutputTextDelta {
                    step: 2,
                    text: "Actual final answer".into(),
                    source: None,
                })
                .unwrap(),
                None,
                EngineJournalWrite::Progress,
            )
            .await
            .unwrap();
        journal
            .append(
                json!({"event":"host_cleanup_proven"}).to_string(),
                None,
                EngineJournalWrite::Cleanup,
            )
            .await
            .unwrap();
        journal
            .append(
                serde_json::to_string(&AgentEngineEvent::TurnCompleted {
                    model_steps: final_step,
                    finish_reason: nomifun_chat_model_broker::ChatFinishReason::Completed,
                })
                .unwrap(),
                None,
                EngineJournalWrite::Terminal,
            )
            .await
            .unwrap();
        match (
            history::closed_turn_answer(&pool, &store, &session, "turn", 0)
                .await
                .unwrap(),
            expected,
        ) {
            (PublicAnswerRead::Answer(answer), Some(expected)) => {
                assert_eq!(answer.text, expected);
                assert!(!answer.text.contains("Earlier narration"));
                assert!(answer.event_ids.len() >= 2);
                let owner = store.get_live_session(&session).await.unwrap().owner_ref;
                clear_with_command(
                    &store,
                    &owner,
                    &session,
                    format!("companion-archive-context:{}", uuid::Uuid::now_v7()),
                    "companion_archive",
                )
                .await;
                assert!(matches!(
                    history::closed_turn_answer(&pool, &store, &session, "turn", 0)
                        .await
                        .unwrap(),
                    PublicAnswerRead::Unavailable
                ));
                let floor = history::explicit_history_floor(&pool, &session)
                    .await
                    .unwrap();
                assert_eq!(floor, 0);
                let PublicAnswerRead::Answer(archived) = history::closed_turn_answer_at_floor(
                    &pool, &store, &session, "turn", floor, false,
                )
                .await
                .unwrap() else {
                    panic!(
                        "an automatic archive retains the exact sealed public result for explicit recall"
                    );
                };
                assert_eq!(archived.text, expected);
                clear_with_command(
                    &store,
                    &owner,
                    &session,
                    format!("user-context-clear:{}", uuid::Uuid::now_v7()),
                    "user_requested",
                )
                .await;
                let floor = history::explicit_history_floor(&pool, &session)
                    .await
                    .unwrap();
                assert!(matches!(
                    history::closed_turn_answer_at_floor(
                        &pool, &store, &session, "turn", floor, false
                    )
                    .await
                    .unwrap(),
                    PublicAnswerRead::Unavailable
                ));
            }
            (PublicAnswerRead::Unavailable, None) => {}
            _ => panic!("only the proven exact final step may be shared"),
        }
    }
}

#[tokio::test]
async fn explicit_history_recall_finds_older_inputs_beyond_recent_slots_with_scope_and_byte_limits()
{
    let database = nomifun_db::init_database_memory().await.unwrap();
    let canonical = CanonicalAgentSessionOwner::from_pool(database.pool().clone())
        .await
        .unwrap();
    let store = canonical.store();
    let owner = principal();
    let (_memory_directory, service, companion) = memory_service(&owner).await;
    let desktop = open(&canonical, &owner, &companion, None).await;
    let other = open(&canonical, &owner, &uuid::Uuid::now_v7().to_string(), None).await;
    user_input(store, &desktop, "上周的远足计划：七点出发", None).await;
    for index in 0..5 {
        user_input(
            store,
            &desktop,
            &format!("后来另一个普通话题 {index}"),
            None,
        )
        .await;
    }
    user_input(store, &other, "私有伙伴的出发安排", None).await;
    let request = |source| nomifun_companion::CompanionHistoryRecallQuery {
        query: "出发".into(),
        limit: 3,
        byte_budget: 2048,
        source_session_id: source,
        since_ts: None,
        until_ts: None,
        before: None,
    };
    let page = recall_companion_history(
        database.pool(),
        store,
        &service,
        &owner.principal_id,
        &companion,
        request(None),
    )
    .await
    .unwrap();
    assert_eq!(page.fragments.len(), 1);
    assert!(page.fragments[0].text.contains("七点出发"));
    assert_eq!(page.fragments[0].session_id, desktop.as_ref());
    assert!(!page.fragments[0].event_ids.is_empty());
    assert!(serde_json::to_vec(&page).unwrap().len() <= 2048);
    assert!(
        recall_companion_history(
            database.pool(),
            store,
            &service,
            &owner.principal_id,
            &companion,
            request(Some(other.as_ref().into()))
        )
        .await
        .is_err()
    );
}

async fn clear_with_command(
    store: &AgentSessionStore,
    owner: &PrincipalRef,
    id: &AgentSessionId,
    key: String,
    reason: &str,
) {
    let request = nomifun_agent_session::ClearSessionContext {
        idempotency_key: key.into(),
        reason: reason.into(),
    };
    let nomifun_agent_session::ContextClearPreparation::Ready(boundary) = store
        .preflight_context_clear(owner, id, &request)
        .await
        .unwrap()
    else {
        panic!("fresh canonical clear");
    };
    store
        .clear_context(owner, id, &request, &boundary)
        .await
        .unwrap();
}

#[tokio::test]
async fn explicit_history_survives_more_than_512_host_archives_but_user_clear_and_spoofed_reason_are_hard_boundaries()
 {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let canonical = CanonicalAgentSessionOwner::from_pool(database.pool().clone())
        .await
        .unwrap();
    let store = canonical.store();
    let owner = principal();
    let (_directory, service, companion) = memory_service(&owner).await;
    let current = open(&canonical, &owner, &companion, None).await;
    let source = open(&canonical, &owner, &companion, None).await;
    user_input(store, &source, "ARCHIVE_REACHABILITY original source", None).await;
    // Real Store writer produces the reserved archive command, exact admission
    // and predecessor. A long history must not turn a fixed proof-page budget
    // into permanent loss of reachability.
    for _ in 0..513 {
        clear_with_command(
            store,
            &owner,
            &source,
            format!("companion-archive-context:{}", uuid::Uuid::now_v7()),
            "companion_archive",
        )
        .await;
    }
    assert!(
        recent_context(
            database.pool(),
            store,
            &service,
            &owner,
            &current,
            &companion
        )
        .await
        .unwrap()
        .is_empty()
    );
    let query = || nomifun_companion::CompanionHistoryRecallQuery {
        query: "ARCHIVE_REACHABILITY".into(),
        limit: 3,
        byte_budget: 2048,
        source_session_id: Some(source.as_ref().into()),
        since_ts: None,
        until_ts: None,
        before: None,
    };
    let recalled = recall_companion_history(
        database.pool(),
        store,
        &service,
        &owner.principal_id,
        &companion,
        query(),
    )
    .await
    .unwrap();
    assert_eq!(recalled.fragments.len(), 1);
    assert!(recalled.fragments[0].text.contains("original source"));

    // Even a formally written payload with the same reason has no archive
    // exemption when it belongs to the public clear command namespace.
    clear_with_command(
        store,
        &owner,
        &source,
        format!(
            "user-context-clear:companion-archive-context:{}",
            uuid::Uuid::now_v7()
        ),
        "companion_archive",
    )
    .await;
    assert!(
        recall_companion_history(
            database.pool(),
            store,
            &service,
            &owner.principal_id,
            &companion,
            query()
        )
        .await
        .unwrap()
        .fragments
        .is_empty()
    );
    user_input(
        store,
        &source,
        "ARCHIVE_REACHABILITY after spoofed reason",
        None,
    )
    .await;
    clear_with_command(
        store,
        &owner,
        &source,
        format!("companion-archive-context:{}", uuid::Uuid::now_v7()),
        "companion_archive",
    )
    .await;
    assert_eq!(
        recall_companion_history(
            database.pool(),
            store,
            &service,
            &owner.principal_id,
            &companion,
            query()
        )
        .await
        .unwrap()
        .fragments
        .len(),
        1
    );
    clear_with_command(
        store,
        &owner,
        &source,
        format!("user-context-clear:{}", uuid::Uuid::now_v7()),
        "user_requested",
    )
    .await;
    assert!(
        recall_companion_history(
            database.pool(),
            store,
            &service,
            &owner.principal_id,
            &companion,
            query()
        )
        .await
        .unwrap()
        .fragments
        .is_empty()
    );
}

#[tokio::test]
async fn malformed_or_overflowing_archive_admission_cannot_erase_a_history_hard_boundary() {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let canonical = CanonicalAgentSessionOwner::from_pool(database.pool().clone())
        .await
        .unwrap();
    let store = canonical.store();
    let owner = principal();
    let (_directory, service, companion) = memory_service(&owner).await;
    let source = open(&canonical, &owner, &companion, None).await;
    user_input(store, &source, "MALFORMED_ARCHIVE protected source", None).await;
    clear_with_command(
        store,
        &owner,
        &source,
        format!("companion-archive-context:{}", uuid::Uuid::now_v7()),
        "companion_archive",
    )
    .await;
    let (event,original):(String,String)=sqlx::query_as("SELECT event_id,inline_json FROM agent_events WHERE session_id=? AND kind='context/cleared' ORDER BY seq DESC LIMIT 1")
        .bind(source.as_ref()).fetch_one(database.pool()).await.unwrap();
    let value: Value = serde_json::from_str(&original).unwrap();
    let generation = value["admission"]["active_set_generation"]
        .as_u64()
        .unwrap();
    let overflow = original.replace(
        &format!("\"active_set_generation\":{generation}"),
        "\"active_set_generation\":18446744073709551616",
    );
    assert_ne!(overflow, original);
    let unknown_field = original.replacen("{", "{\"unexpected\":true,", 1);
    let negative_zero = original.replace(
        &format!("\"active_set_generation\":{generation}"),
        "\"active_set_generation\":-0",
    );
    for damaged in [overflow, unknown_field, negative_zero] {
        sqlx::query("UPDATE agent_events SET inline_json=? WHERE event_id=?")
            .bind(damaged)
            .bind(&event)
            .execute(database.pool())
            .await
            .unwrap();
        assert!(
            history::explicit_history_floor(database.pool(), &source)
                .await
                .is_err(),
            "typed proof decoding must fail closed"
        );
        let page = recall_companion_history(
            database.pool(),
            store,
            &service,
            &owner.principal_id,
            &companion,
            nomifun_companion::CompanionHistoryRecallQuery {
                query: "MALFORMED_ARCHIVE".into(),
                limit: 3,
                byte_budget: 2048,
                source_session_id: Some(source.as_ref().into()),
                since_ts: None,
                until_ts: None,
                before: None,
            },
        )
        .await
        .unwrap();
        assert!(
            page.fragments.is_empty(),
            "the SQL candidate filter must not exempt a malformed host-looking clear"
        );
    }
}

#[tokio::test]
async fn history_query_uses_the_advertised_1024_character_bound_for_ascii_and_unicode() {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let canonical = CanonicalAgentSessionOwner::from_pool(database.pool().clone())
        .await
        .unwrap();
    let store = canonical.store();
    let owner = principal();
    let (_directory, service, companion) = memory_service(&owner).await;
    let source = open(&canonical, &owner, &companion, None).await;
    for text in ["a".repeat(600), "旅".repeat(1024)] {
        user_input(store, &source, &text, None).await;
        let query = nomifun_companion::CompanionHistoryRecallQuery {
            query: text.clone(),
            limit: 1,
            byte_budget: 8192,
            source_session_id: Some(source.as_ref().into()),
            since_ts: None,
            until_ts: None,
            before: None,
        };
        let found = recall_companion_history(
            database.pool(),
            store,
            &service,
            &owner.principal_id,
            &companion,
            query.clone(),
        )
        .await
        .unwrap();
        assert_eq!(found.fragments.len(), 1);
        assert!(found.fragments[0].text.contains(&text));
        let mut too_long = query;
        too_long.query = "旅".repeat(1025);
        assert!(
            recall_companion_history(
                database.pool(),
                store,
                &service,
                &owner.principal_id,
                &companion,
                too_long
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn canonical_human_source_graph_excludes_later_steering_and_never_invents_a_hidden_user() {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let canonical = CanonicalAgentSessionOwner::from_pool(database.pool().clone())
        .await
        .unwrap();
    let store = canonical.store();
    let owner = principal();
    let (_directory, _service, companion) = memory_service(&owner).await;
    let session = open(&canonical, &owner, &companion, None).await;
    let operation = nomifun_agent_contracts::OperationId::from("human-source-turn");
    let (root, _) = store
        .start_turn(
            &session,
            "session_api".into(),
            "source-root".into(),
            operation.clone(),
            StrictJsonValue(json!({"content":"original human input","hidden":false})),
        )
        .await
        .unwrap();
    let (_, earlier) = store
        .steer_exact_turn(
            &session,
            &operation,
            "earlier-steer".into(),
            "session_api".into(),
            StrictJsonValue(json!({"content":"earlier human steer","hidden":false})),
        )
        .await
        .unwrap();
    let (_, later) = store
        .steer_exact_turn(
            &session,
            &operation,
            "later-steer".into(),
            "session_api".into(),
            StrictJsonValue(json!({"content":"later human steer","hidden":false})),
        )
        .await
        .unwrap();
    let root = root.record.unwrap();
    let earlier = earlier.record.unwrap();
    let later = later.record.unwrap();
    let sources = history::resolve_companion_turn_human_sources(
        database.pool(),
        store,
        &owner,
        &companion,
        &session,
        operation.as_ref(),
        later.seq,
    )
    .await
    .unwrap()
    .unwrap();
    let ids = sources
        .iter()
        .map(|source| source.message_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        ids,
        [root.event_id.as_ref(), earlier.event_id.as_ref()].into()
    );
    assert!(
        sources
            .iter()
            .all(|source| source.session_id == session.as_ref())
    );
    assert!(
        history::resolve_companion_turn_human_sources(
            database.pool(),
            store,
            &owner,
            &companion,
            &session,
            operation.as_ref(),
            root.seq
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        history::resolve_companion_turn_human_sources(
            database.pool(),
            store,
            &owner,
            "foreign-companion",
            &session,
            operation.as_ref(),
            later.seq
        )
        .await
        .is_err()
    );
    store
        .cancel_active_turn(&session, "close-human-source".into(), "session_api".into())
        .await
        .unwrap();
    store.start_turn(&session,"session_api".into(),"unproven-hidden".into(),"hidden-turn".into(),
        StrictJsonValue(json!({"content":"unproven host instructions","hidden":true,"origin":"agent_execution"}))).await.unwrap();
    assert!(
        history::resolve_companion_turn_human_sources(
            database.pool(),
            store,
            &owner,
            &companion,
            &session,
            "hidden-turn",
            i64::MAX as u64
        )
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn history_cursor_pages_across_more_than_32_sessions_without_duplicate_sources() {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let canonical = CanonicalAgentSessionOwner::from_pool(database.pool().clone())
        .await
        .unwrap();
    let store = canonical.store();
    let owner = principal();
    let (_directory, service, companion) = memory_service(&owner).await;
    let mut sessions = Vec::new();
    for index in 0..35 {
        let id = open(&canonical, &owner, &companion, None).await;
        user_input(store, &id, &format!("出发安排 {index}"), None).await;
        sessions.push(id);
    }
    let mut request = nomifun_companion::CompanionHistoryRecallQuery {
        query: "出发".into(),
        limit: 12,
        byte_budget: 16 * 1024,
        source_session_id: None,
        since_ts: None,
        until_ts: None,
        before: None,
    };
    let mut ids = std::collections::BTreeSet::new();
    let mut finished = false;
    for _ in 0..5 {
        let page = recall_companion_history(
            database.pool(),
            store,
            &service,
            &owner.principal_id,
            &companion,
            request.clone(),
        )
        .await
        .unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() <= request.byte_budget);
        for fragment in page.fragments {
            assert!(
                ids.insert(fragment.event_ids[0].clone()),
                "a stable cursor must not repeat a source"
            );
        }
        if !page.has_more {
            finished = true;
            break;
        }
        let cursor = page
            .next_cursor
            .expect("an incomplete page needs a real scanned boundary");
        assert_ne!(
            request
                .before
                .as_ref()
                .map(|cursor| (&cursor.session_id, cursor.source_seq)),
            Some((&cursor.session_id, cursor.source_seq))
        );
        request.before = Some(cursor);
    }
    assert!(finished);
    assert_eq!(ids.len(), 35);
    request.before = None;
    request.source_session_id = Some(sessions[0].as_ref().into());
    let exact = recall_companion_history(
        database.pool(),
        store,
        &service,
        &owner.principal_id,
        &companion,
        request,
    )
    .await
    .unwrap();
    assert_eq!(
        exact.fragments.len(),
        1,
        "exact old source is independent of the cross-Session scan budget"
    );
}

#[tokio::test]
async fn small_history_byte_pages_retry_each_unreturned_matching_source() {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let canonical = CanonicalAgentSessionOwner::from_pool(database.pool().clone())
        .await
        .unwrap();
    let store = canonical.store();
    let owner = principal();
    let (_directory, service, companion) = memory_service(&owner).await;
    let session = open(&canonical, &owner, &companion, None).await;
    for index in 0..5 {
        user_input(
            store,
            &session,
            &format!("出发 {index} {}", "x".repeat(160)),
            None,
        )
        .await;
    }
    let mut request = nomifun_companion::CompanionHistoryRecallQuery {
        query: "出发".into(),
        limit: 12,
        byte_budget: 1000,
        source_session_id: Some(session.as_ref().into()),
        since_ts: None,
        until_ts: None,
        before: None,
    };
    let mut ids = std::collections::BTreeSet::new();
    let mut finished = false;
    for _ in 0..8 {
        let page = recall_companion_history(
            database.pool(),
            store,
            &service,
            &owner.principal_id,
            &companion,
            request.clone(),
        )
        .await
        .unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() <= 1000);
        for fragment in page.fragments {
            assert!(ids.insert(fragment.event_ids[0].clone()));
        }
        if !page.has_more {
            finished = true;
            break;
        }
        request.before = Some(
            page.next_cursor
                .expect("a byte-limited page needs an actual retry boundary"),
        );
    }
    assert!(finished);
    assert_eq!(
        ids.len(),
        5,
        "a source that did not fit must be retried, never skipped by its cursor"
    );
}

#[tokio::test]
async fn unified_input_claims_share_original_speech_once_and_pending_input_stays_out_of_context() {
    let database=nomifun_db::init_database_memory().await.unwrap();
    let canonical=CanonicalAgentSessionOwner::from_pool(database.pool().clone()).await.unwrap();
    let store=canonical.store();let owner=principal();
    let (_directory,service,companion)=memory_service(&owner).await;
    let desktop=open(&canonical,&owner,&companion,None).await;
    let robot=open(&canonical,&owner,&companion,Some("physical-robot")).await;
    let snapshot=store.get_live_session(&desktop).await.unwrap().agent_binding.resolved_snapshot_ref;
    let operation=uuid::Uuid::now_v7().to_string();
    let (root,_)=store.start_turn(&desktop,"session_api".into(),format!("{operation}:root").into(),operation.clone().into(),
        StrictJsonValue(json!({"content":"VISIBLE_ROOT","files":[],"inject_skills":[],"hidden":false,
            "admission":{"resolved_snapshot_ref":snapshot,"route_identity":nomifun_agent_contracts::ChatRouteIdentity::new(
                "shared-context-fixture@1","agent_chat","fixture-route".into(),1)}}))).await.unwrap();
    let root_id=root.record.unwrap().event_id;
    let accepted=store.accept_session_input(&owner,&desktop,"session_api".into(),"shared-input-claimed".into(),
        StrictJsonValue(json!({"content":"VISIBLE_ONCE","files":[],"inject_skills":[],"hidden":false}))).await.unwrap().record.unwrap();
    store.claim_session_input_as_steering(&owner,&desktop,&accepted.event_id,&operation.clone().into()).await.unwrap();
    store.accept_session_input(&owner,&desktop,"session_api".into(),"shared-input-pending".into(),
        StrictJsonValue(json!({"content":"PENDING_NEVER_CONTEXT","files":[],"inject_skills":[],"hidden":false}))).await.unwrap();
    let recent=recent_context(database.pool(),store,&service,&owner,&robot,&companion).await.unwrap();
    let rendered=serde_json::to_string(&recent).unwrap();
    assert!(rendered.contains("VISIBLE_ROOT"));
    assert_eq!(rendered.matches("VISIBLE_ONCE").count(),1,"the v2 claim cannot repeat the original utterance");
    assert!(!rendered.contains("PENDING_NEVER_CONTEXT"));
    let source_boundary=store.head(&desktop).await.unwrap().last_seq.checked_add(1).unwrap();
    let sources=history::resolve_companion_turn_human_sources(database.pool(),store,&owner,&companion,&desktop,&operation,source_boundary)
        .await.unwrap().unwrap();
    let ids=sources.iter().map(|source|source.message_id.as_str()).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids,std::collections::BTreeSet::from([root_id.as_ref(),accepted.event_id.as_ref()]));
}
