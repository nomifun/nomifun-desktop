//! Current-generation normalization preserves authority bytes and disables
//! unproven launches without authoring grants or touching Session history.
use nomifun_agent_contracts::*;
use nomifun_db::*;
use serde_json::{Value, json};
use sqlx::{
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};
static MIGRATOR: Migrator = sqlx::migrate!();
const PROVIDER: &str = "0190f5fe-7c00-7a00-8000-000000000081";
const PRESET: &str = "0190f5fe-7c00-7a00-8000-000000000082";
const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000083";
fn artifacts(
    owner: &str,
) -> (
    AgentPresetRevision,
    ResolvedSnapshotEnvelope,
    ProviderAgentRevisionCleanup,
) {
    let route_id = ModelRouteId::from("chat-route");
    let route = ChatRouteRecord {
        schema: ChatRouteRecordSchema::V1,
        task: ChatRouteTask::AgentChat,
        primary: ChatRouteCandidate {
            model_route_id: route_id.clone(),
            input_constraints: None,
            model_route_revision: 1,
            provider_id: PROVIDER.into(),
            model: "remove".into(),
            protocol: ChatRouteProtocol::OpenaiChat,
            connection_config_ref: "default".into(),
            config_revision_digest: "a".repeat(64).into(),
            credential_ref: "fixture-credential".into(),
            features: [
                ChatRouteFeature::TextInput,
                ChatRouteFeature::TextOutput,
                ChatRouteFeature::ToolCalls,
            ]
            .into_iter()
            .collect(),
            activation_features: BTreeSet::new(),
        },
        failovers: vec![],
    };
    let mut revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: PRESET.into(),
            revision: 1,
            revision_digest: "".into(),
        },
        payload: AgentPresetRevisionPayload {
            schema_version: "1.0.0".into(),
            model_route_refs: BTreeMap::from([(CHAT_MODEL_TASK_AGENT_CHAT.into(), route_id)]),
            chat_route_records: BTreeMap::from([(CHAT_MODEL_TASK_AGENT_CHAT.into(), route)]),
            enabled_capabilities: vec![],
            skill_bindings: vec![],
            context_order: vec![],
            middleware_order: vec![],
            system_role_provider_overrides: BTreeMap::new(),
            persona: "keep persona".into(),
            instructions: "keep instructions".into(),
            starter_prompts: vec![],
            runtime_policy: Default::default(),
        },
        contribution_locks: vec![],
        created_by: owner.into(),
        created_at_ms: 1,
        reason: None,
    };
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    revision.validate().unwrap();
    let content = ResolvedSnapshotContent {
        schema_version: "1.0.0".into(),
        resolver_version: "1.0.0".into(),
        preset_revision_ref: revision.reference.clone(),
        required_runtime_protocol_version: "1.0.0".into(),
        required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest: "a".repeat(64).into(),
        required_runtime_features: BTreeSet::new(),
        compiled_runtime_profile_digest: "b".repeat(64).into(),
        model_route_refs: revision.payload.model_route_refs.clone(),
        chat_route_identity: revision.chat_route_identity().unwrap(),
        enabled_capabilities: vec![],
        required_resource_kinds: BTreeSet::new(),
        capability_allowlist: BTreeSet::new(),
        context_order: vec![],
        middleware_order: vec![],
        skill_locks: vec![],
        mcp_tool_locks: vec![],
        resolved_role_providers: BTreeMap::new(),
        canonical_schema_manifest_digest: "c".repeat(64).into(),
        target_contribution_manifest_digest: "d".repeat(64).into(),
    };
    let snapshot = ResolvedSnapshotEnvelope {
        snapshot_ref: ResolvedSnapshotRef {
            snapshot_id: nomifun_common::generate_id().into(),
            snapshot_digest: digest_payload(&content).unwrap(),
        },
        content,
        actor: PrincipalRef {
            principal_kind: "user".into(),
            principal_id: owner.into(),
        },
        scene: "test".into(),
        surface: "desktop".into(),
        audience: "owner".into(),
        created_at_ms: 1,
        resolver_run_id: nomifun_common::generate_id().into(),
        availability_evidence_revision: "frozen".into(),
    };
    snapshot.validate().unwrap();
    let mut next = revision.clone();
    next.reference.revision = 2;
    next.payload =
        catalog_model_detached_payload(&revision.payload, PROVIDER, Some("remove")).unwrap();
    next.reference.revision_digest = next.revision_digest().unwrap();
    next.created_at_ms = 2;
    let mut next_snapshot = snapshot.clone();
    next_snapshot.snapshot_ref.snapshot_id = nomifun_common::generate_id().into();
    next_snapshot.content.preset_revision_ref = next.reference.clone();
    next_snapshot.content.model_route_refs = next.payload.model_route_refs.clone();
    next_snapshot.content.chat_route_identity = next.chat_route_identity().unwrap();
    next_snapshot.content.compiled_runtime_profile_digest = "e".repeat(64).into();
    next_snapshot.snapshot_ref.snapshot_digest = digest_payload(&next_snapshot.content).unwrap();
    next_snapshot.validate().unwrap();
    let patch = ProviderAgentRevisionCleanup {
        expected_current: revision.reference.clone(),
        revision: next,
        snapshot: next_snapshot,
    };
    (revision, snapshot, patch)
}

fn with_workspace(
    mut revision: AgentPresetRevision,
    mut snapshot: ResolvedSnapshotEnvelope,
) -> (
    AgentPresetRevision,
    ResolvedSnapshotEnvelope,
    AgentBindingValue,
) {
    let lock = ContributionLock {
        source_kind: ContributionSourceKind::PlatformBuiltin,
        source_identity: "fixture.package".into(),
        mount_id: None,
        mcp_binding_id: None,
        contribution_id: "capability:fixture.workspace".into(),
        contract_digest: "a".repeat(64).into(),
    };
    let action = CapabilityActionDescriptor {
        action_id: "fixture.workspace/read".into(),
        input_schema: "fixture-input".into(),
        output_schema: "fixture-output".into(),
        effect_class: EffectClass::ReadLocal,
        presentation: ToolPresentationKind::FunctionTool,
        required_resource_operations: Some(BTreeMap::from([(
            "workspace".into(),
            BTreeSet::from(["read".into()]),
        )])),
    };
    let mut knowledge_action = action.clone();
    knowledge_action.action_id = "fixture.workspace/knowledge-write".into();
    knowledge_action.required_resource_operations = Some(BTreeMap::from([(
        "knowledge_base".into(),
        BTreeSet::from(["write".into()]),
    )]));
    let mut mcp_action = action.clone();
    mcp_action.action_id = "fixture.workspace/mcp-invoke".into();
    mcp_action.required_resource_operations = Some(BTreeMap::from([(
        "mcp_server".into(),
        BTreeSet::from(["invoke".into()]),
    )]));
    let selected = CapabilitySelection {
        capability: CapabilityRef {
            id: "fixture.workspace".into(),
        },
        action_allowlist: BTreeSet::from([
            action.action_id.clone(),
            knowledge_action.action_id.clone(),
            mcp_action.action_id.clone(),
        ]),
    };
    let capability = ResolvedCapability {
        consumption: Default::default(),
        dependency_refs: vec![],
        capability: selected.capability.clone(),
        source_package: PackageRef {
            id: "fixture.package".into(),
            version: "1.0.0".into(),
        },
        contribution_id: lock.contribution_id.clone(),
        contribution_lock: lock.clone(),
        resolved_mount_id: None,
        resolved_source: PluginSourceMetadata {
            source_kind: PluginSourceKind::Bundled,
            source_identity: "fixture.package".into(),
            source_digest: Some("b".repeat(64).into()),
        },
        target_artifact_digest: "b".repeat(64).into(),
        schema_digest: lock.contract_digest.clone(),
        dependency_path: vec![selected.capability.id.clone()],
        required_runtime_features: BTreeSet::new(),
        display_name: None,
        description: None,
        actions: vec![action, knowledge_action, mcp_action],
        required_resource_kinds: BTreeSet::from([
            "workspace".into(),
            "knowledge_base".into(),
            "mcp_server".into(),
        ]),
        action_allowlist: selected.action_allowlist.clone(),
    };
    revision.payload.enabled_capabilities = vec![selected];
    revision.contribution_locks = vec![lock];
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    revision.validate().unwrap();
    snapshot.content.preset_revision_ref = revision.reference.clone();
    snapshot.content.enabled_capabilities = vec![capability];
    snapshot.content.capability_allowlist = BTreeSet::from(["fixture.workspace".into()]);
    snapshot.content.required_resource_kinds = BTreeSet::from([
        "workspace".into(),
        "knowledge_base".into(),
        "mcp_server".into(),
    ]);
    snapshot.snapshot_ref.snapshot_digest = digest_payload(&snapshot.content).unwrap();
    snapshot.validate().unwrap();
    let mut resource = TypedResourceBinding {
        binding_id: "".into(),
        resource_kind: "workspace".into(),
        resource_id: "fixture-workspace".into(),
        owner_id: OWNER.into(),
        operations: BTreeSet::from(["read".into()]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::from([("workspace_root".into(), "/tmp/fixture".into())]),
    };
    resource.binding_id = resource_definition_id(&resource).unwrap();
    let mut knowledge = resource.clone();
    knowledge.resource_kind = "knowledge_base".into();
    knowledge.resource_id = "0190f5fe-7c00-7a00-8000-000000000091".into();
    knowledge.typed_parameters.clear();
    knowledge.binding_id = resource_definition_id(&knowledge).unwrap();
    let mut mcp = knowledge.clone();
    mcp.resource_kind = "mcp_server".into();
    mcp.resource_id = "0190f5fe-7c00-7a00-8000-000000000092".into();
    mcp.operations = BTreeSet::from(["discover".into()]);
    mcp.binding_id = resource_definition_id(&mcp).unwrap();
    let binding = AgentBindingValue {
        preset_revision_ref: revision.reference.clone(),
        resolved_snapshot_ref: snapshot.snapshot_ref.clone(),
        typed_resource_bindings: vec![resource, knowledge, mcp],
        binding_version: 1,
    };
    (revision, snapshot, binding)
}

async fn prefix(path: &std::path::Path) -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true)
                .foreign_keys(false),
        )
        .await
        .unwrap();
    Migrator {
        migrations: Cow::Owned(MIGRATOR.iter().take(11).cloned().collect()),
        ..Migrator::DEFAULT
    }
    .run(&pool)
    .await
    .unwrap();
    // This fixture's schema predates migration 016, so its declared
    // lineage is the pre-016 manifest digest, not the current manifest.
    sqlx::query("INSERT INTO schema_metadata(singleton_key,data_generation,root_instance_id,migration_head,seed_manifest_digest,canonical_schema_manifest_digest,projection_schema_version) VALUES('canonical',?,'main-sqlite-agent-store',?,?,?,?)")
        .bind(i64::from(AGENT_STORE_DATA_GENERATION)).bind(i64::from(AGENT_STORE_MIGRATION_HEAD)).bind(digest_payload(&official_preset_seed_manifest_payload()).unwrap().as_ref()).bind("ce5cb186b9ba1a44c7c4699f4e1932542068537149c2a76ec880e2b8cc0bf8d9").bind(i64::from(AGENT_STORE_PROJECTION_SCHEMA_VERSION)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(user_id,username,password_hash,created_at,updated_at) VALUES(?,'owner','hash',1,1)").bind(OWNER).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO installation_identity(singleton_key,owner_user_id) VALUES('installation',?)",
    )
    .bind(OWNER)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO providers(provider_id,platform,name,base_url,auth_scheme,credentials_encrypted,enabled,created_at,updated_at) VALUES(?,'openai','fixture','https://example.invalid','bearer','fixture',1,1,1)").bind(PROVIDER).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO provider_models(provider_id,model,created_at,updated_at) VALUES(?,'remove',1,1)").bind(PROVIDER).execute(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn forward_binding_normalization_preserves_exact_authority_and_rejects_unproven_grants() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("normalization.sqlite");
    let pool = prefix(&path).await;
    let (source, original_snapshot, _) = artifacts(OWNER);
    let (revision, snapshot, binding) = with_workspace(source, original_snapshot);
    sqlx::query("INSERT INTO agent_presets(preset_id,owner_ref_json,source_json,display_json,current_stable_revision,created_at) VALUES(?,?,'\"user\"','{}',1,1)").bind(PRESET).bind(json!({"user_id":OWNER}).to_string()).execute(&pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    insert_agent_preset_revision_tx(&mut tx, &revision, &snapshot)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let source_rows: Vec<(String, String)> =
        sqlx::query_as("SELECT payload_json,revision_digest FROM agent_preset_revisions")
            .fetch_all(&pool)
            .await
            .unwrap();
    let source_snapshots: Vec<String> =
        sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots")
            .fetch_all(&pool)
            .await
            .unwrap();
    let anchor = nomifun_common::generate_id();
    let anchor_binding = serde_json::to_string(&binding).unwrap();
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES(?,?,'live','Preserve anchor',0,0,?,1,1)")
        .bind(&anchor).bind(json!({"principal_kind":"user","principal_id":OWNER}).to_string()).bind(&anchor_binding).execute(&pool).await.unwrap();
    for case in [
        "valid",
        "new-with-anchor",
        "empty-operations",
        "multiple-workspaces",
        "wrong-definition-digest",
        "wrong-owner",
        "conflicting-inline",
        "bare-model",
    ] {
        let mut candidate = binding.clone();
        if case == "multiple-workspaces" {
            let mut second = candidate.typed_resource_bindings[0].clone();
            second.resource_id = "another-workspace".into();
            second.binding_id = resource_definition_id(&second).unwrap();
            candidate.typed_resource_bindings.push(second);
        }
        if case == "empty-operations" {
            candidate.typed_resource_bindings[0].operations.clear();
            candidate.typed_resource_bindings[0].binding_id =
                resource_definition_id(&candidate.typed_resource_bindings[0]).unwrap();
        }
        if case == "wrong-definition-digest" {
            candidate.typed_resource_bindings[0].binding_id = "wrong".into();
        }
        if case == "wrong-owner" {
            candidate.typed_resource_bindings[0].owner_id =
                "0190f5fe-7c00-7a00-8000-000000000099".into();
        }
        let projected =
            json!({"preset_id":PRESET,"preset_revision":1,"canonical_binding":candidate});
        let mut config = json!({"name":"Original","provider_id":PROVIDER,"model":"remove","agent_snapshot":projected});
        if case == "conflicting-inline" {
            config["agent_snapshot"]["preset_revision"] = json!(2);
        }
        if case == "bare-model" {
            config.as_object_mut().unwrap().remove("agent_snapshot");
        }
        sqlx::query("INSERT INTO cron_jobs(cron_job_id,user_id,name,schedule_kind,schedule_value,payload_message,execution_mode,agent_type,agent_config,preset_id,preset_revision,agent_snapshot,conversation_id,next_run_at,created_by,created_at,updated_at) VALUES(?, ?,?,'every','60000','Original user instruction','new_conversation','nomi',?,?,?, ?,?,60001,'user',1,1)")
            .bind(nomifun_common::generate_id()).bind(OWNER).bind(case).bind(config.to_string()).bind(if case=="bare-model"{None}else{Some(PRESET)}).bind(if case=="bare-model"{None}else{Some(1i64)}).bind(if case=="bare-model"{None}else{Some(projected.to_string())}).bind((case=="new-with-anchor").then_some(anchor.as_str())).execute(&pool).await.unwrap();
    }
    sqlx::query("UPDATE sqlite_sequence SET seq=900 WHERE name='cron_jobs'")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    for _ in 0..2 {
        let db = init_database(&path).await.unwrap();
        let rows:Vec<(String,bool,Option<String>,Option<String>,String,String)>=sqlx::query_as("SELECT name,enabled,agent_binding,configuration_error,payload_message,model_selection FROM cron_jobs ORDER BY name").fetch_all(db.pool()).await.unwrap();
        for (name, enabled, saved, error, message, model) in rows {
            assert_eq!(message, "Original user instruction");
            assert_eq!(
                serde_json::from_str::<Value>(&model).unwrap(),
                json!({"provider_id":PROVIDER,"model":"remove"})
            );
            if name == "valid" || name == "new-with-anchor" {
                assert!(enabled);
                assert!(error.is_none());
                assert_eq!(
                    serde_json::from_str::<AgentBindingValue>(&saved.unwrap()).unwrap(),
                    binding
                );
            } else {
                assert!(!enabled, "{name} was silently authorized");
                assert!(saved.is_none());
                assert_eq!(
                    error.as_deref(),
                    Some("CRON_CONFIGURATION_RESELECTION_REQUIRED")
                );
            }
        }
        assert_eq!(
            sqlx::query_as::<_, (String, String)>(
                "SELECT payload_json,revision_digest FROM agent_preset_revisions"
            )
            .fetch_all(db.pool())
            .await
            .unwrap(),
            source_rows
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT envelope_json FROM agent_runtime_snapshots")
                .fetch_all(db.pool())
                .await
                .unwrap(),
            source_snapshots
        );
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM cron_jobs WHERE execution_mode='new_conversation' AND conversation_id IS NOT NULL").fetch_one(db.pool()).await.unwrap(),0);
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT agent_binding_json FROM agent_sessions WHERE agent_session_id=?"
            )
            .bind(&anchor)
            .fetch_one(db.pool())
            .await
            .unwrap(),
            anchor_binding
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT seq FROM sqlite_sequence WHERE name='cron_jobs'")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            900
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name='cron_job_runs'"
            )
            .fetch_one(db.pool())
            .await
            .unwrap(),
            0
        );
        db.close().await;
    }
}
