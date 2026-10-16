use nomifun_db::*;
use nomifun_agent_contracts::*;
use serde_json::{Value,json};
use std::collections::{BTreeMap,BTreeSet};

const PROVIDER: &str = "0190f5fe-7c00-7a00-8000-000000000081";
const PRESET: &str = "0190f5fe-7c00-7a00-8000-000000000082";

async fn seeded() -> nomifun_db::Database {
    let db = init_database_memory().await.unwrap();
    let caps = [NewProviderModelCapability {task:"chat",protocol:"openai.chat_text",traits:"[]",connection_role:"default",provider_params:"{}",..Default::default()}];
    SqliteProviderRepository::new(db.pool().clone()).create(CreateProviderParams {
        provider_id:Some(PROVIDER),platform:"openai",name:"Delete fixture",base_url:"https://example.invalid/v1",
        auth_scheme:"bearer",credentials_encrypted:"fixture",enabled:true,bedrock_config:None,sort_order:None,
    },&NewProviderModel {model:"remove",enabled:true,capabilities:&caps,..Default::default()},&[]).await.unwrap();
    SqliteProviderModelRepository::new(db.pool().clone()).save(PROVIDER,0,
        &NewProviderModel {model:"sibling",enabled:true,capabilities:&caps,..Default::default()}).await.unwrap();
    db
}

#[tokio::test]
async fn deleted_choices_cannot_be_restored_by_customer_service_or_template_writers() {
    for whole_provider in [false, true] {
        let db = seeded().await;
        let owner = installation_owner_id(db.pool()).await.unwrap();
        let cs = SqliteCustomerServiceRepository::new(db.pool().clone());
        let cs_input = NewCsAgentRow {
            cs_agent_id: nomifun_common::generate_id(), name: "Current support".into(),
            greeting: "Hello".into(), persona: "Keep".into(), service_policy: "Keep".into(),
            provider_id: Some(PROVIDER.into()), model: Some("remove".into()),
            knowledge_base_ids: "[]".into(), enabled: true, max_concurrent: 1,
            audit_retention_days: 30, created_at: 1, updated_at: 1,
        };
        let current_cs = cs.create_agent(&cs_input).await.unwrap();
        let templates = SqliteAgentExecutionTemplateRepository::new(db.pool().clone());
        let participant = NewAgentExecutionTemplateParticipant {
            template_participant_id: nomifun_common::generate_id(),
            source_agent_id: "0190f5fe-7c00-7a00-8000-000000000114".into(),
            preset_id: None, preset_revision: None, agent_snapshot: None,
            provider_id: Some(PROVIDER.into()), model: Some("remove".into()),
            role: None, capability: None, constraints: None, description: None,
            system_prompt: None, enabled_skills: "[]".into(),
            disabled_builtin_skills: "[]".into(), sort_order: 0,
        };
        let template = templates.create_template(&owner, &CreateAgentExecutionTemplateParams {
            name: "Current template".into(), description: None, max_parallel: Some(1),
            work_dir: None, context: None, participants: vec![participant.clone()],
        }).await.unwrap();
        if whole_provider {
            SqliteProviderRepository::new(db.pool().clone()).delete(PROVIDER).await.unwrap();
        } else {
            SqliteProviderModelRepository::new(db.pool().clone()).delete_coordinated(&CoordinatedProviderModelDelete {
                provider_id: PROVIDER.into(), model: "remove".into(), expected_config_revision: 1,
                cleanup: Default::default(),
            }).await.unwrap();
        }
        let stale_cs = UpdateCsAgentParams {
            name: Some("Must roll back".into()), provider_id: Some(Some(PROVIDER.into())),
            model: Some(Some("remove".into())), ..Default::default()
        };
        assert!(cs.update_agent(&current_cs.cs_agent_id, &stale_cs, 10).await.is_err());
        let retained = cs.get_agent(&current_cs.cs_agent_id).await.unwrap().unwrap();
        assert_eq!(retained.name, "Current support");
        assert_eq!((retained.provider_id, retained.model), (None, None));
        let mut stale_new_cs = cs_input.clone();
        stale_new_cs.cs_agent_id = nomifun_common::generate_id();
        assert!(cs.create_agent(&stale_new_cs).await.is_err());
        assert_eq!(cs.list_agents().await.unwrap().len(), 1);

        assert!(templates.update_template(&owner, &template.template.execution_template_id,
            template.template.version, &UpdateAgentExecutionTemplateParams {
                name: Some("Must roll back".into()), participants: Some(vec![participant.clone()]),
                ..Default::default()
            }).await.is_err());
        let retained = templates.get_template(&owner, &template.template.execution_template_id).await.unwrap().unwrap();
        assert_eq!(retained.template.name, "Current template");
        assert_eq!((&retained.participants[0].provider_id, &retained.participants[0].model), (&None, &None));
        let mut cleared = participant;
        cleared.provider_id = None;
        cleared.model = None;
        let saved = templates.update_template(&owner, &template.template.execution_template_id,
            retained.template.version, &UpdateAgentExecutionTemplateParams {
                participants: Some(vec![cleared]), ..Default::default()
            }).await.unwrap();
        assert_eq!((&saved.participants[0].provider_id, &saved.participants[0].model), (&None, &None));
        cs.update_agent(&current_cs.cs_agent_id, &UpdateCsAgentParams {
            provider_id: Some(None), model: Some(None), ..Default::default()
        }, 11).await.unwrap();
    }
}

#[tokio::test]
async fn registered_preferences_reject_deleted_exact_models_without_interpreting_external_json() {
    let db = seeded().await;
    let preferences = SqliteClientPreferenceRepository::new(db.pool().clone());
    let selected = json!({"provider_id":PROVIDER,"model":"remove"});
    let cases = [
        ("nomi.defaultModel", selected.clone()),
        ("channels.test.defaultModel", selected.clone()),
        ("models.default.imageGeneration", selected.clone()),
        ("tools.speechToText", selected.clone()),
        ("tools.textToSpeech", selected.clone()),
        ("nomi.collaborationModels", json!([selected.clone()])),
        ("agent.model_failover", json!({"enabled":true,"queue":[selected.clone()]})),
        ("knowledge.retrieval", json!({"embedding":{"mode":"remote","provider_id":PROVIDER,"model":"remove"},"rerank":{"mode":"local"}})),
    ];
    for (key, value) in &cases {
        preferences.upsert_batch(&[(key, &value.to_string())]).await.unwrap();
    }
    SqliteProviderModelRepository::new(db.pool().clone()).delete_coordinated(&CoordinatedProviderModelDelete {
        provider_id:PROVIDER.into(), model:"remove".into(), expected_config_revision:1,
        cleanup:Default::default(),
    }).await.unwrap();
    for (key, value) in &cases {
        let before: Vec<_> = preferences.get_all().await.unwrap().into_iter()
            .map(|row| (row.key, row.value)).collect();
        assert!(preferences.upsert_batch(&[("theme", "\"must-not-save\""), (key, &value.to_string())])
            .await.is_err(), "a stale registered selection was restored through {key}");
        let after: Vec<_> = preferences.get_all().await.unwrap().into_iter()
            .map(|row| (row.key, row.value)).collect();
        assert_eq!(after, before, "failed selection must roll back the entire preference batch");
    }
    preferences.upsert_batch(&[("tools.speechToText", &json!({"provider_id":PROVIDER}).to_string())])
        .await.unwrap();
    preferences.upsert_batch(&[("external.vendor.config", &selected.to_string())]).await.unwrap();
    preferences.upsert_batch(&[("nomi.defaultModel", &json!({"provider_id":PROVIDER,"model":"sibling"}).to_string())])
        .await.unwrap();
}

#[tokio::test]
async fn deletion_clears_only_matching_mutable_choices_and_preserves_cron_snapshots() {
    for whole_provider in [false,true] {
        let db = seeded().await;
        let owner = installation_owner_id(db.pool()).await.unwrap();
        let selection = |model| json!({"provider_id":PROVIDER,"model":model});
        for (key,value) in [
            ("nomi.defaultModel",selection("remove")),
            ("channels.test.defaultModel",selection("sibling")),
            ("agent.model_failover",json!({"enabled":true,"queue":[selection("remove"),selection("sibling")],"max_switches":4})),
            ("knowledge.retrieval",json!({"embedding":{"mode":"remote","provider_id":PROVIDER,"model":"remove"},"rerank":{"mode":"remote","provider_id":PROVIDER,"model":"sibling"}})),
            ("agent_session.idmm.fixture",json!({"config":{"mode":"rule_plus_model","bypass_model":selection("remove")},"revision":4,"history":["keep"]})),
        ] {
            sqlx::query("INSERT INTO client_preferences (key,value,updated_at) VALUES (?,?,1)")
                .bind(key).bind(value.to_string()).execute(db.pool()).await.unwrap();
        }
        for model in ["remove","sibling"] {
            sqlx::query("INSERT INTO cs_agents (cs_agent_id,name,provider_id,model,created_at,updated_at) VALUES (?,?,?,?,1,1)")
                .bind(nomifun_common::generate_id()).bind(model).bind(PROVIDER).bind(model).execute(db.pool()).await.unwrap();
            sqlx::query("INSERT INTO cron_jobs (cron_job_id,user_id,name,schedule_kind,schedule_value,payload_message,agent_type,agent_config,agent_snapshot,next_run_at,created_by,created_at,updated_at) VALUES (?,?,?,'every','60000','','nomi',?,'{\"frozen\":\"keep\"}',10000,'user',1,1)")
                .bind(nomifun_common::generate_id()).bind(&owner).bind(model).bind(json!({"provider_id":PROVIDER,"model":model,"workspace":"keep"}).to_string())
                .execute(db.pool()).await.unwrap();
        }
        if whole_provider {
            SqliteProviderRepository::new(db.pool().clone()).delete(PROVIDER).await.unwrap();
        } else {
            assert!(SqliteProviderModelRepository::new(db.pool().clone()).delete_coordinated(&CoordinatedProviderModelDelete {
                provider_id:PROVIDER.into(),model:"remove".into(),expected_config_revision:1,cleanup:Default::default(),
            }).await.unwrap());
        }
        let values: BTreeMap<String,String> = sqlx::query_as::<_,(String,String)>("SELECT key,value FROM client_preferences")
            .fetch_all(db.pool()).await.unwrap().into_iter().collect();
        assert!(!values.contains_key("nomi.defaultModel"));
        assert_eq!(values.contains_key("channels.test.defaultModel"),!whole_provider);
        let failover: Value = serde_json::from_str(&values["agent.model_failover"]).unwrap();
        assert_eq!(failover["queue"],if whole_provider {json!([])} else {json!([selection("sibling")])});
        let retrieval: Value = serde_json::from_str(&values["knowledge.retrieval"]).unwrap();
        assert_eq!(retrieval["embedding"],json!({"mode":"local"}));
        assert_eq!(retrieval["rerank"],if whole_provider {json!({"mode":"local"})} else {json!({"mode":"remote","provider_id":PROVIDER,"model":"sibling"})});
        let idmm: Value = serde_json::from_str(&values["agent_session.idmm.fixture"]).unwrap();
        assert_eq!(idmm["config"]["bypass_model"],json!({}));assert_eq!(idmm["config"]["mode"],"rule_only");
        assert_eq!(idmm["revision"],5);assert_eq!(idmm["history"],json!(["keep"]));
        let cs: Vec<(String,Option<String>,Option<String>)> = sqlx::query_as("SELECT name,provider_id,model FROM cs_agents ORDER BY name")
            .fetch_all(db.pool()).await.unwrap();
        for (name,provider,model) in cs {
            if name=="remove" || whole_provider {assert_eq!((provider,model),(None,None));}
            else {assert_eq!((provider.as_deref(),model.as_deref()),(Some(PROVIDER),Some("sibling")));}
        }
        let jobs: Vec<(String,bool,String,String,i64,Option<i64>)> = sqlx::query_as("SELECT name,enabled,agent_config,agent_snapshot,schedule_revision,next_run_at FROM cron_jobs ORDER BY name")
            .fetch_all(db.pool()).await.unwrap();
        assert_eq!(jobs.len(),2,"cron jobs are preserved");
        for (name,enabled,config,snapshot,revision,next) in jobs {
            assert_eq!(snapshot,"{\"frozen\":\"keep\"}");
            let config: Value = serde_json::from_str(&config).unwrap();assert_eq!(config["workspace"],"keep");
            if name=="remove" || whole_provider {
                assert!(!enabled);assert!(config["provider_id"].is_null());assert!(config["model"].is_null());assert_eq!(revision,2);assert_eq!(next,None);
            } else {assert!(enabled);assert_eq!(config,selection("sibling").as_object().map(|object| {let mut object=object.clone();object.insert("workspace".into(),json!("keep"));Value::Object(object)}).unwrap());assert_eq!(revision,1);assert_eq!(next,Some(10000));}
        }
        if !whole_provider {assert!(SqliteProviderModelRepository::new(db.pool().clone()).get(PROVIDER,"sibling").await.unwrap().is_some());}
    }
}

fn artifacts(owner: &str) -> (AgentPresetRevision,ResolvedSnapshotEnvelope,ProviderAgentRevisionCleanup) {
    let route_id = ModelRouteId::from("chat-route");
    let route = ChatRouteRecord {schema:ChatRouteRecordSchema::V1,task:ChatRouteTask::AgentChat,
        primary:ChatRouteCandidate {model_route_id:route_id.clone(),model_route_revision:1,provider_id:PROVIDER.into(),model:"remove".into(),
            protocol:ChatRouteProtocol::OpenaiChat,connection_config_ref:"default".into(),config_revision_digest:"a".repeat(64).into(),credential_ref:"fixture-credential".into(),
            features:[ChatRouteFeature::TextInput,ChatRouteFeature::TextOutput,ChatRouteFeature::ToolCalls].into_iter().collect(),activation_features:BTreeSet::new()},failovers:vec![]};
    let mut revision = AgentPresetRevision {reference:PresetRevisionRef {preset_id:PRESET.into(),revision:1,revision_digest:"".into()},
        payload:AgentPresetRevisionPayload {schema_version:"1.0.0".into(),model_route_refs:BTreeMap::from([(CHAT_MODEL_TASK_AGENT_CHAT.into(),route_id)]),
            chat_route_records:BTreeMap::from([(CHAT_MODEL_TASK_AGENT_CHAT.into(),route)]),enabled_capabilities:vec![],skill_bindings:vec![],
            context_order:vec![],middleware_order:vec![],system_role_provider_overrides:BTreeMap::new(),persona:"keep persona".into(),instructions:"keep instructions".into(),starter_prompts:vec![],runtime_policy:Default::default()},
        contribution_locks:vec![],created_by:owner.into(),created_at_ms:1,reason:None};
    revision.reference.revision_digest=revision.revision_digest().unwrap();revision.validate().unwrap();
    let content = ResolvedSnapshotContent {schema_version:"1.0.0".into(),resolver_version:"1.0.0".into(),preset_revision_ref:revision.reference.clone(),
        required_runtime_protocol_version:"1.0.0".into(),required_runtime_profile:RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest:"a".repeat(64).into(),required_runtime_features:BTreeSet::new(),compiled_runtime_profile_digest:"b".repeat(64).into(),
        model_route_refs:revision.payload.model_route_refs.clone(),chat_route_identity:revision.chat_route_identity().unwrap(),enabled_capabilities:vec![],required_resource_kinds:BTreeSet::new(),capability_allowlist:BTreeSet::new(),
        context_order:vec![],middleware_order:vec![],skill_locks:vec![],mcp_tool_locks:vec![],resolved_role_providers:BTreeMap::new(),canonical_schema_manifest_digest:"c".repeat(64).into(),target_contribution_manifest_digest:"d".repeat(64).into()};
    let snapshot = ResolvedSnapshotEnvelope {snapshot_ref:ResolvedSnapshotRef {snapshot_id:nomifun_common::generate_id().into(),snapshot_digest:digest_payload(&content).unwrap()},content,
        actor:PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()},scene:"test".into(),surface:"desktop".into(),audience:"owner".into(),created_at_ms:1,resolver_run_id:nomifun_common::generate_id().into(),availability_evidence_revision:"frozen".into()};
    snapshot.validate().unwrap();
    let mut next = revision.clone();next.reference.revision=2;
    next.payload=catalog_model_detached_payload(&revision.payload,PROVIDER,Some("remove")).unwrap();
    next.reference.revision_digest=next.revision_digest().unwrap();next.created_at_ms=2;
    let mut next_snapshot=snapshot.clone();next_snapshot.snapshot_ref.snapshot_id=nomifun_common::generate_id().into();
    next_snapshot.content.preset_revision_ref=next.reference.clone();next_snapshot.content.model_route_refs=next.payload.model_route_refs.clone();next_snapshot.content.chat_route_identity=next.chat_route_identity().unwrap();
    next_snapshot.content.compiled_runtime_profile_digest="e".repeat(64).into();
    next_snapshot.snapshot_ref.snapshot_digest=digest_payload(&next_snapshot.content).unwrap();next_snapshot.validate().unwrap();
    let patch=ProviderAgentRevisionCleanup {expected_current:revision.reference.clone(),revision:next,snapshot:next_snapshot};
    (revision,snapshot,patch)
}

async fn seed_artifacts(db: &nomifun_db::Database, hidden: bool) -> (AgentPresetRevision,ResolvedSnapshotEnvelope,ProviderAgentRevisionCleanup) {
    let owner=installation_owner_id(db.pool()).await.unwrap();let result=artifacts(&owner);
    persist_artifacts(db,&owner,hidden,&result.0,&result.1).await;
    result
}

async fn persist_artifacts(db: &nomifun_db::Database, owner: &str, hidden: bool, source: &AgentPresetRevision, snapshot: &ResolvedSnapshotEnvelope) {
    sqlx::query("INSERT INTO agent_presets (preset_id,owner_ref_json,source_json,display_json,current_stable_revision,created_at) VALUES (?,?,'\"user\"',?,1,1)")
        .bind(PRESET).bind(json!({"user_id":owner}).to_string()).bind(json!({"display_name":"Keep Agent","description":null,"session_only":hidden}).to_string()).execute(db.pool()).await.unwrap();
    let mut tx=db.pool().begin().await.unwrap();insert_agent_preset_revision_tx(&mut tx,source,snapshot).await.unwrap();tx.commit().await.unwrap();
}

#[tokio::test]
async fn bound_existing_cron_uses_the_owned_sessions_exact_revision_and_only_its_primary_model() {
    for case in ["no-config","unrelated-config","failover-only","other-model","other-owner","remote"] {
        for whole_provider in [false,true] {
            // Same-provider siblings are only unaffected by exact-model deletion.
            if whole_provider && matches!(case,"failover-only"|"other-model") {continue;}
            let db=seeded().await;
            let job_owner=installation_owner_id(db.pool()).await.unwrap();
            let session_owner=if case=="other-owner" {
                let user="0190f5fe-7c00-7a00-8000-000000000084";
                sqlx::query("INSERT INTO users (user_id,username,password_hash,created_at,updated_at) VALUES (?,'cron_other','hash',1,1)")
                    .bind(user).execute(db.pool()).await.unwrap();
                user.to_owned()
            } else {job_owner.clone()};
            let (mut source,mut snapshot,_)=artifacts(&session_owner);
            if matches!(case,"failover-only"|"other-model") {
                let route=source.payload.chat_route_records.get_mut(CHAT_MODEL_TASK_AGENT_CHAT).unwrap();
                if case=="failover-only" {
                    let mut fallback=route.primary.clone();
                    fallback.model_route_id="fallback-route".into();
                    route.failovers.push(fallback);
                }
                route.primary.model="sibling".into();
                source.reference.revision_digest=source.revision_digest().unwrap();
                snapshot.content.preset_revision_ref=source.reference.clone();
                snapshot.content.chat_route_identity=source.chat_route_identity().unwrap();
                snapshot.snapshot_ref.snapshot_digest=digest_payload(&snapshot.content).unwrap();
            }
            persist_artifacts(&db,&session_owner,false,&source,&snapshot).await;
            let patches=if let Some(payload)=catalog_model_detached_payload(&source.payload,PROVIDER,if whole_provider {None} else {Some("remove")}) {
                let mut revision=source.clone();revision.payload=payload;revision.reference.revision+=1;
                revision.reference.revision_digest=revision.revision_digest().unwrap();
                let mut next=snapshot.clone();next.snapshot_ref.snapshot_id=nomifun_common::generate_id().into();
                next.content.preset_revision_ref=revision.reference.clone();
                next.content.model_route_refs=revision.payload.model_route_refs.clone();
                next.content.chat_route_identity=revision.chat_route_identity().unwrap();
                next.content.compiled_runtime_profile_digest="e".repeat(64).into();
                next.snapshot_ref.snapshot_digest=digest_payload(&next.content).unwrap();
                vec![ProviderAgentRevisionCleanup {expected_current:source.reference.clone(),revision,snapshot:next}]
            } else {vec![]};
            let expected_current=if patches.is_empty() {1} else {2};
            let mut session_binding=AgentBindingValue {
                preset_revision_ref:source.reference.clone(), resolved_snapshot_ref:snapshot.snapshot_ref.clone(),
                typed_resource_bindings:vec![], binding_version:1,
            };
            if case=="remote" {
                session_binding.preset_revision_ref.preset_id="0190f5fe-7c00-7a00-8000-000000000085".into();
            }
            let binding=serde_json::to_string(&session_binding).unwrap();
            let session_id=nomifun_common::generate_id();
            sqlx::query("INSERT INTO agent_sessions (agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES (?,?,'live','Preserve',0,0,?,1,1)")
                .bind(&session_id).bind(json!({"principal_kind":"user","principal_id":session_owner}).to_string())
                .bind(&binding).execute(db.pool()).await.unwrap();
            if case=="remote" {
                sqlx::query("UPDATE agent_sessions SET remote_binding_id=?,remote_binding_version=1 WHERE agent_session_id=?")
                    .bind("0190f5fe-7c00-7a00-8000-000000000086").bind(&session_id).execute(db.pool()).await.unwrap();
            }
            let config=(case=="unrelated-config").then(|| json!({"name":"Nomi","provider_id":PROVIDER,"model":"sibling"}).to_string());
            let job_id=nomifun_common::generate_id();
            sqlx::query("INSERT INTO cron_jobs (cron_job_id,user_id,name,schedule_kind,schedule_value,payload_message,execution_mode,agent_type,agent_config,conversation_id,next_run_at,created_by,created_at,updated_at) VALUES (?,?,'Bound','every','60000','','existing','nomi',?,?,10000,'user',1,1)")
                .bind(&job_id).bind(&job_owner).bind(&config).bind(&session_id).execute(db.pool()).await.unwrap();
            let before_revision:(String,String)=sqlx::query_as("SELECT payload_json,revision_digest FROM agent_preset_revisions WHERE revision_no=1")
                .fetch_one(db.pool()).await.unwrap();
            let before_snapshot:String=sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
                .bind(snapshot.snapshot_ref.snapshot_id.as_ref()).fetch_one(db.pool()).await.unwrap();
            let plan=ProviderModelCleanupPlan {agent_revisions:patches,..Default::default()};
            if whole_provider {
                SqliteProviderRepository::new(db.pool().clone()).delete_coordinated(&CoordinatedProviderDelete {
                    provider_id:PROVIDER.into(),expected_config_revision:1,cleanup:plan,
                }).await.unwrap();
            } else {
                SqliteProviderModelRepository::new(db.pool().clone()).delete_coordinated(&CoordinatedProviderModelDelete {
                    provider_id:PROVIDER.into(),model:"remove".into(),expected_config_revision:1,cleanup:plan,
                }).await.unwrap();
            }
            let should_stop=matches!(case,"no-config"|"unrelated-config");
            let (enabled,revision,next):(bool,i64,Option<i64>)=sqlx::query_as("SELECT enabled,schedule_revision,next_run_at FROM cron_jobs WHERE cron_job_id=?")
                .bind(&job_id).fetch_one(db.pool()).await.unwrap();
            assert_eq!((enabled,revision,next),if should_stop {(false,2,None)} else {(true,1,Some(10000))},"{case}/{whole_provider}");
            let current:i64=sqlx::query_scalar("SELECT current_stable_revision FROM agent_presets WHERE preset_id=?")
                .bind(PRESET).fetch_one(db.pool()).await.unwrap();assert_eq!(current,expected_current);
            let after_binding:String=sqlx::query_scalar("SELECT agent_binding_json FROM agent_sessions WHERE agent_session_id=?")
                .bind(&session_id).fetch_one(db.pool()).await.unwrap();assert_eq!(after_binding,binding);
            let after_revision:(String,String)=sqlx::query_as("SELECT payload_json,revision_digest FROM agent_preset_revisions WHERE revision_no=1")
                .fetch_one(db.pool()).await.unwrap();assert_eq!(after_revision,before_revision);
            let after_snapshot:String=sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
                .bind(snapshot.snapshot_ref.snapshot_id.as_ref()).fetch_one(db.pool()).await.unwrap();assert_eq!(after_snapshot,before_snapshot);
        }
    }
}

#[tokio::test]
async fn current_agent_requires_a_complete_plan_but_hidden_and_historical_artifacts_do_not_block() {
    for hidden in [false,true] {
        let db=seeded().await;let (source,snapshot,patch)=seed_artifacts(&db,hidden).await;
        let before: (String,String) = sqlx::query_as("SELECT payload_json,revision_digest FROM agent_preset_revisions WHERE revision_no=1").fetch_one(db.pool()).await.unwrap();
        let providers=SqliteProviderRepository::new(db.pool().clone());
        if hidden {providers.delete(PROVIDER).await.unwrap();}
        else {
            assert!(matches!(providers.delete(PROVIDER).await.unwrap_err(),DbError::Conflict(message) if message.contains("cleanup plan")));
            providers.delete_coordinated(&CoordinatedProviderDelete {provider_id:PROVIDER.into(),expected_config_revision:1,
                cleanup:ProviderModelCleanupPlan {agent_revisions:vec![patch],..Default::default()}}).await.unwrap();
            let current: i64=sqlx::query_scalar("SELECT current_stable_revision FROM agent_presets WHERE preset_id=?").bind(PRESET).fetch_one(db.pool()).await.unwrap();assert_eq!(current,2);
        }
        assert!(providers.find_by_id(PROVIDER).await.unwrap().is_none());
        let after: (String,String)=sqlx::query_as("SELECT payload_json,revision_digest FROM agent_preset_revisions WHERE revision_no=1").fetch_one(db.pool()).await.unwrap();assert_eq!(after,before);
        assert_eq!(after.0,serde_json::to_string(&source.payload).unwrap());
        let stored: String=sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?").bind(snapshot.snapshot_ref.snapshot_id.as_ref()).fetch_one(db.pool()).await.unwrap();
        assert_eq!(stored,serde_json::to_string(&snapshot).unwrap());
    }
}

#[tokio::test]
async fn invalid_or_stale_agent_patch_rolls_back_catalog_and_all_other_cleanup() {
    for stale in [false,true] {
        let db=seeded().await;let (_,_,mut patch)=seed_artifacts(&db,false).await;
        sqlx::query("INSERT INTO client_preferences (key,value,updated_at) VALUES ('nomi.defaultModel',?,1)").bind(json!({"provider_id":PROVIDER,"model":"remove"}).to_string()).execute(db.pool()).await.unwrap();
        if stale {patch.expected_current.revision_digest="f".repeat(64).into();}
        else {patch.snapshot.content.canonical_schema_manifest_digest="e".repeat(64).into();patch.snapshot.snapshot_ref.snapshot_digest=digest_payload(&patch.snapshot.content).unwrap();}
        let error=SqliteProviderRepository::new(db.pool().clone()).delete_coordinated(&CoordinatedProviderDelete {provider_id:PROVIDER.into(),expected_config_revision:1,
            cleanup:ProviderModelCleanupPlan {agent_revisions:vec![patch],..Default::default()}}).await.unwrap_err();assert!(matches!(error,DbError::Conflict(_)));
        assert!(SqliteProviderRepository::new(db.pool().clone()).find_by_id(PROVIDER).await.unwrap().is_some());
        let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_preset_revisions").fetch_one(db.pool()).await.unwrap();assert_eq!(count,1);
        let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM client_preferences WHERE key='nomi.defaultModel'").fetch_one(db.pool()).await.unwrap();assert_eq!(count,1);
    }
}

#[tokio::test]
async fn exact_model_agent_detachment_keeps_siblings_then_history_does_not_restrict_provider_delete() {
    let db=seeded().await;let (source,_,patch)=seed_artifacts(&db,false).await;
    let models=SqliteProviderModelRepository::new(db.pool().clone());
    assert!(models.delete_coordinated(&CoordinatedProviderModelDelete {provider_id:PROVIDER.into(),model:"remove".into(),expected_config_revision:1,
        cleanup:ProviderModelCleanupPlan {agent_revisions:vec![patch],..Default::default()}}).await.unwrap());
    assert!(models.get(PROVIDER,"sibling").await.unwrap().is_some());
    let current:i64=sqlx::query_scalar("SELECT current_stable_revision FROM agent_presets WHERE preset_id=?").bind(PRESET).fetch_one(db.pool()).await.unwrap();assert_eq!(current,2);
    let providers=SqliteProviderRepository::new(db.pool().clone());providers.delete(PROVIDER).await.unwrap();
    let old:String=sqlx::query_scalar("SELECT payload_json FROM agent_preset_revisions WHERE revision_id=?").bind(source.reference.revision_id()).fetch_one(db.pool()).await.unwrap();
    assert_eq!(old,serde_json::to_string(&source.payload).unwrap());
    assert!(providers.find_by_id(PROVIDER).await.unwrap().is_none());
}
