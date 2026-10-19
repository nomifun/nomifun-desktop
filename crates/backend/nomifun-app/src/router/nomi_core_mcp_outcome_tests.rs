use super::*;
use nomifun_agent_contracts::{
    AgentBindingValue, AgentSessionLiveRecord, AgentSessionMetadata, CorrelationId,
    EventProducerId, IdempotencyKey, OperationId, PresetRevisionRef, ResolvedSnapshotRef,
    SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef,
};
use nomifun_agent_session::{
    AgentEffectState, AgentSessionStore, CreateSessionRequest, NativeExecutionClaim,
    NativeExecutionLease,
};
use nomifun_db::{CreateMcpServerParams, SqliteMcpServerRepository};
use nomifun_mcp::{
    McpCleanupFacts, McpCleanupStatus, McpExecutionFacts, McpInvocationOutcome, McpOwnerError,
};
use serde_json::{Value, json};

struct CredentialFailure;

#[async_trait]
impl nomifun_mcp::McpCredentialAuthority for CredentialFailure {
    async fn resolve(
        &self,
        _: nomifun_mcp::McpCredentialLookup,
    ) -> Result<Option<nomifun_mcp::McpCredential>, McpOwnerError> {
        Err(McpOwnerError::new("MCP_CREDENTIAL_EXPIRED", "The exact credential expired before dispatch"))
    }
}

struct Fixture {
    _database: nomifun_db::Database,
    store: AgentSessionStore,
    host: NomiCoreMcpHost,
    session: AgentSessionLiveRecord,
    turn: OperationId,
    started: nomifun_agent_contracts::EventId,
    tool: super::super::nomi_core_mcp_catalog::FrozenMcpTool,
    resource: TypedResourceBinding,
    lease: NativeExecutionLease,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_transport("http", r#"{"url":"http://127.0.0.1:1/mcp"}"#).await
    }

    async fn with_transport(transport: &str, configuration: &str) -> Self {
        let owner = uuid::Uuid::now_v7().to_string();
        let database = nomifun_db::init_database_memory_with_owner(
            nomifun_common::UserId::parse(owner.clone()).unwrap(),
        ).await.unwrap();
        let store = AgentSessionStore::from_pool(database.pool().clone()).await.unwrap();
        let repository = Arc::new(SqliteMcpServerRepository::new(database.pool().clone()));
        let tools = json!([{"name":"echo","description":"Echo one input",
            "input_schema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}]).to_string();
        let row = repository.create(CreateMcpServerParams {
            name:"outcome-fixture", description:None, enabled:true, transport_type:transport,
            transport_config:configuration,
            tools:Some(&tools), original_json:None, builtin:false,
        }).await.unwrap();
        let tool = super::super::nomi_core_mcp_catalog::server_tools(row).unwrap().remove(0);
        let mut resource = TypedResourceBinding {
            binding_id:"pending".into(), resource_kind:"mcp_server".into(),
            resource_id:tool.lock.server_id.as_ref().to_owned().into(), owner_id:owner.clone(),
            operations:["connect".into(),"read".into(),"invoke".into()].into(),
            connection_config_ref:Some(tool.connection_config_ref.clone()), typed_parameters:BTreeMap::new(),
        };
        resource.binding_id = nomifun_agent_contracts::resource_definition_id(&resource).unwrap();
        let id = uuid::Uuid::now_v7().to_string();
        let session = AgentSessionLiveRecord {
            agent_session_id:id.clone().into(),
            owner_ref:PrincipalRef {principal_kind:"user".into(),principal_id:owner.clone()},
            metadata:AgentSessionMetadata {purpose:Default::default(),app_project_id:None,
                title:Some("MCP typed outcome".into()),archived:false,pinned:false,
                reasoning_effort:None,companion:None},
            agent_binding:AgentBindingValue {
                preset_revision_ref:PresetRevisionRef {preset_id:uuid::Uuid::now_v7().to_string().into(),
                    revision:1,revision_digest:"a".repeat(64).into()},
                resolved_snapshot_ref:ResolvedSnapshotRef {snapshot_id:"mcp-outcome-snapshot".into(),snapshot_digest:"b".repeat(64).into()},
                typed_resource_bindings:vec![resource.clone()],binding_version:1,
            },
            remote_binding_provenance:None,parent_session_id:None,fork_base_payload_id:None,next_seq:1,
        };
        let created = store.create_session(CreateSessionRequest::new(session,nomifun_common::now_ms(),
            format!("create-mcp:{id}"),"session_api".into(),format!("create-mcp:{id}").into(),id.clone().into())).await.unwrap();
        store.append_event(&event(&created.session.agent_session_id,"ready","session/ready",&id,
            Some(created.opening_ack.event_id),json!({}))).await.unwrap();
        let turn: OperationId = format!("mcp-parent:{id}").into();
        let route = nomifun_agent_contracts::ChatRouteIdentity::new("mcp-test@1",
            nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,"mcp-test-route".into(),1);
        let (_, start) = store.start_turn(&created.session.agent_session_id,"session_api".into(),"mcp-user-root".into(),turn.clone(),
            StrictJsonValue(json!({"content":"Use the exact MCP tool once.","admission":{"route_identity":route,
                "resolved_snapshot_ref":created.session.agent_binding.resolved_snapshot_ref}}))).await.unwrap();
        let lease = store.claim_native_execution(NativeExecutionClaim {
            owner:created.session.owner_ref.clone(),agent_session_id:created.session.agent_session_id.clone(),
            operation_id:turn.clone(),snapshot:created.session.agent_binding.resolved_snapshot_ref.clone(),
            active_set_generation:0,holder:"mcp-outcome-native-owner".into(),expected_fence:0,checkpoint:None,
        }).await.unwrap();
        let host = NomiCoreMcpHost {
            receipts:super::super::mcp_effect_receipts::McpEffectReceipts::new(database.pool().clone()),
            source:NomiCoreMcpRuntimeBindingSource {repository,installation_owner:owner.into()},
            owner:McpOwnerAdapter::new(Arc::new(McpOwner::new(Arc::new(CredentialFailure),
                reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap()))),
            sessions:Mutex::new(BTreeMap::new()),
        };
        Self {_database:database,store,host,session:created.session,turn,
            started:start.record.unwrap().event_id,tool,resource,lease}
    }

    async fn call(&self, key: &str) -> McpExecutionContext {
        let operation: OperationId = format!("mcp-call:{key}").into();
        let mut call = event(&self.session.agent_session_id,&format!("call:{key}"),"tool/call-started",key,
            Some(self.started.clone()),json!({"operation_id":operation,"call_id":key,
                "capability_id":self.tool.lock.capability_id,"action_id":self.tool.action()}));
        call.producer_id = "runtime_supervisor".into();
        self.store.append_native_event(&self.lease,&call,None).await.unwrap();
        McpExecutionContext {
            principal:self.session.owner_ref.clone(),agent_session_id:self.session.agent_session_id.clone(),
            turn_id:self.turn.clone(),resolved_snapshot_ref:self.session.agent_binding.resolved_snapshot_ref.clone(),
            operation_id:operation,capability_id:self.tool.lock.capability_id.clone(),
            resource_bindings:vec![self.resource.clone()],mcp_tool_lock:Some(self.tool.lock.clone()),
            control:nomifun_agent_kernel::InvocationControl::new(tokio_util::sync::CancellationToken::new()),
        }
    }

    async fn begin(&self, context: &McpExecutionContext) -> super::super::mcp_effect_receipts::McpEffectReceipt {
        self.host.receipts.begin(&context.principal.principal_id,context.agent_session_id.as_ref(),
            context.operation_id.as_ref(),context.capability_id.as_ref(),&context.turn_id,&context.resolved_snapshot_ref).await.unwrap()
    }
}

fn event(session: &nomifun_agent_contracts::AgentSessionId, key: &str, kind: &str, correlation: &str,
    causation: Option<nomifun_agent_contracts::EventId>, value: Value) -> SessionEventAppend {
    SessionEventAppend {agent_session_id:session.clone(),event_id:key.to_owned().into(),
        producer_id:EventProducerId::from("session_api"),idempotency_key:IdempotencyKey::from(key.to_owned()),
        semantic_event:SemanticSessionEventDraft {kind:SessionEventKind(kind.into()),kind_version:1,
            correlation_id:CorrelationId::from(correlation.to_owned()),causation_event_id:causation,
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(value))}}
}

fn outcome(result: Result<Value,McpOwnerError>, dispatched: bool, cleanup: McpCleanupStatus) -> McpInvocationOutcome<StrictJsonValue> {
    McpInvocationOutcome {result:result.map(StrictJsonValue),execution:McpExecutionFacts {
        request_dispatched:dispatched,local_process_started:false,cancellation_requested:false,
    },cleanup:McpCleanupFacts {local:McpCleanupStatus::Confirmed,remote:cleanup,
        error:(cleanup == McpCleanupStatus::Unproven).then(||McpOwnerError::new("MCP_SESSION_CLEANUP_FAILED","remote cleanup is unproven"))}}
}

#[tokio::test]
async fn pre_dispatch_credential_failure_settles_and_does_not_poison_the_same_host() {
    let f = Fixture::new().await;
    for key in ["first","second"] {
        let error = f.host.invoke(f.call(key).await,StrictJsonValue(json!({"text":"hello"}))).await.unwrap_err();
        assert_eq!(error.canonical_code().as_ref(),"MCP_CREDENTIAL_EXPIRED");
        f.host.ensure_settled(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.unwrap();
    }
    let effects = f.store.list_effects(&f.session.agent_session_id).await.unwrap();
    assert_eq!(effects.len(),2);
    assert!(effects.iter().all(|effect|effect.state == AgentEffectState::Rejected));
}

#[tokio::test]
async fn cleanup_uncertainty_keeps_the_observed_result_in_the_original_effect() {
    let f = Fixture::new().await;
    let context = f.call("returned-but-cleanup-unknown").await;
    let receipt = f.begin(&context).await;
    let observation = json!({"isError":true,"content":[{"type":"text","text":"The original business observation"}]});
    let outcome = outcome(Ok(observation.clone()),true,McpCleanupStatus::Unproven);
    f.host.receipts.settle(receipt,&outcome).await.unwrap();
    let effects = f.store.list_effects(&f.session.agent_session_id).await.unwrap();
    assert_eq!(effects.len(),1);
    assert_eq!(effects[0].turn_id,f.turn);
    assert_eq!(effects[0].state,AgentEffectState::Unknown);
    let payload = effects[0].bounded_observation.as_ref().unwrap();
    assert_eq!(payload["observation"],observation);
    assert_eq!(payload["execution"]["returned"],true);
    assert_eq!(payload["cleanup"]["remote"],"unproven");
    assert_eq!(project_owner_outcome(outcome).unwrap_err().canonical_code().as_ref(),"MCP_SESSION_CLEANUP_FAILED");
    assert!(f.host.ensure_settled(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.is_err());
    assert!(f.host.ensure_shutdown_resources_closed(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.is_err(),
        "a returned business observation is not proof of the original native cleanup");
}

#[tokio::test]
async fn known_business_failure_is_returned_and_late_settlement_keeps_its_original_turn() {
    let f = Fixture::new().await;
    let context = f.call("returned-business-failure").await;
    let receipt = f.begin(&context).await;
    f.store.cancel_exact_turn(&f.session.agent_session_id,&f.turn,"cancel-parent".into(),"session_api".into()).await.unwrap();
    let value = json!({"isError":true,"content":[{"type":"text","text":"Missing external prerequisite"}]});
    f.host.receipts.settle(receipt,&outcome(Ok(value.clone()),true,McpCleanupStatus::Confirmed)).await.unwrap();
    let effects = f.store.list_effects(&f.session.agent_session_id).await.unwrap();
    assert_eq!(effects[0].state,AgentEffectState::Returned);
    assert_eq!(effects[0].turn_id,f.turn);
    assert_eq!(effects[0].operation_id,context.operation_id);
    assert_eq!(effects[0].bounded_observation.as_ref().unwrap()["observation"],value);
}

#[tokio::test]
async fn canonical_reconciliation_unblocks_the_same_host_without_restarting_it() {
    let f = Fixture::new().await;
    let context = f.call("unknown").await;
    let next = f.call("must-not-dispatch-after-unknown").await;
    let guard = f.host.claim_session(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).unwrap();
    let receipt = f.begin(&context).await;
    f.host.receipts.settle(receipt,&outcome(Err(McpOwnerError::new("MCP_REQUEST_CANCELLED","Cancellation is not remote stop proof")),true,McpCleanupStatus::Confirmed)).await.unwrap();
    drop(guard);
    assert!(f.host.ensure_settled(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.is_err());
    let blocked = f.host.invoke(next,
        StrictJsonValue(json!({"text":"do not repeat"}))).await.unwrap_err();
    assert_eq!(blocked.canonical_code().as_ref(),"MCP_SESSION_UNCERTAIN");
    assert_eq!(f.store.list_effects(&f.session.agent_session_id).await.unwrap().len(),1);
    let effect = f.store.list_effects(&f.session.agent_session_id).await.unwrap().remove(0);
    f.store.cancel_exact_turn(&f.session.agent_session_id,&f.turn,"cancel-for-reconciliation".into(),"session_api".into()).await.unwrap();
    f.store.reconcile_native_effect_by_owner(&f.session.owner_ref,&f.session.agent_session_id,
        &nomifun_agent_session::NativeEffectReconciliationRequest {
            operation_id:f.turn.clone(),expected_pause_revision:0,idempotency_key:"mcp-owner-confirmation".into(),
            effect_id:effect.effect_id,call_id:None,expected_input_digest:effect.input_digest,
            outcome:nomifun_agent_session::NativeVerifiedOutcome::ConfirmedFailed,
            evidence:nomifun_agent_session::NativeOwnerEvidence {verified:true,evidence_digest:"c".repeat(64).into(),reference:"verified original remote operation and local cleanup".into()},
            cleanup_attestation:None,
        }).await.unwrap();
    f.host.ensure_settled(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.unwrap();
    let _next = f.host.claim_session(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).unwrap();
}

#[tokio::test]
async fn duplicate_receipt_never_authorizes_a_second_dispatch() {
    let f = Fixture::new().await;
    let context = f.call("duplicate").await;
    let receipt = f.begin(&context).await;
    f.host.receipts.settle(receipt,&outcome(Ok(json!({"content":[]})),true,McpCleanupStatus::Confirmed)).await.unwrap();
    assert!(f.host.receipts.begin(&context.principal.principal_id,context.agent_session_id.as_ref(),
        context.operation_id.as_ref(),context.capability_id.as_ref(),&context.turn_id,&context.resolved_snapshot_ref).await.is_err());
    assert_eq!(f.store.list_effects(&f.session.agent_session_id).await.unwrap().len(),1);
}

#[tokio::test]
async fn business_reconciliation_requires_independent_unproven_cleanup_evidence() {
    let f = Fixture::new().await;
    let context = f.call("cleanup-attestation").await;
    let receipt = f.begin(&context).await;
    f.host.receipts.settle(receipt,&outcome(Ok(json!({"content":[{"type":"text","text":"Observed before cleanup failed"}]})),
        true,McpCleanupStatus::Unproven)).await.unwrap();
    let effect = f.store.list_effects(&f.session.agent_session_id).await.unwrap().remove(0);
    f.store.cancel_exact_turn(&f.session.agent_session_id,&f.turn,"cancel-cleanup-attestation".into(),"session_api".into()).await.unwrap();
    let mut request = nomifun_agent_session::NativeEffectReconciliationRequest {
        operation_id:f.turn.clone(),expected_pause_revision:0,idempotency_key:"mcp-cleanup-confirmation".into(),
        effect_id:effect.effect_id.clone(),call_id:None,expected_input_digest:effect.input_digest,
        outcome:nomifun_agent_session::NativeVerifiedOutcome::ConfirmedSucceeded,
        evidence:nomifun_agent_session::NativeOwnerEvidence {verified:true,evidence_digest:"d".repeat(64).into(),reference:"verified the business outcome".into()},
        cleanup_attestation:None,
    };
    let cursor = f.store.current_cursor(&f.session.agent_session_id).await.unwrap();
    assert!(f.store.reconcile_native_effect_by_owner(&f.session.owner_ref,&f.session.agent_session_id,&request).await.is_err());
    assert_eq!(f.store.current_cursor(&f.session.agent_session_id).await.unwrap(),cursor);
    assert!(f.host.ensure_settled(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.is_err());
    request.cleanup_attestation = Some(nomifun_agent_session::NativeOwnerEvidence {
        verified:false,evidence_digest:"e".repeat(64).into(),reference:"unverified cleanup".into(),
    });
    assert!(f.store.reconcile_native_effect_by_owner(&f.session.owner_ref,&f.session.agent_session_id,&request).await.is_err());
    request.cleanup_attestation.as_mut().unwrap().verified = true;
    f.store.reconcile_native_effect_by_owner(&f.session.owner_ref,&f.session.agent_session_id,&request).await.unwrap();
    f.host.ensure_settled(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.unwrap();
    let facts = f.store.chat_causality_facts(&f.session.agent_session_id,&f.turn).await.unwrap();
    let audit = facts.events.iter().find(|event|event.kind.0 == "runtime/effect-reconciliation-attested").unwrap();
    assert_eq!(facts.event_payloads[audit.event_id.as_ref()]["cleanup_attestation"]["verified"],true);
    let _next = f.host.claim_session(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).unwrap();
}

#[test]
fn old_invocation_guard_cannot_release_a_successor_instance() {
    let key = ("owner".into(),"session".into());
    let old = Arc::new(());
    let next = Arc::new(());
    let sessions = Mutex::new(BTreeMap::from([(key.clone(),old.clone())]));
    let guard = InvocationGuard {sessions:&sessions,key:key.clone(),invocation:old};
    sessions.lock().unwrap().insert(key.clone(),next.clone());
    drop(guard);
    assert!(Arc::ptr_eq(sessions.lock().unwrap().get(&key).unwrap(),&next));
}

#[tokio::test]
async fn pending_mcp_reconciliation_requires_cleanup_attestation_and_keeps_the_live_guard() {
    let f = Fixture::new().await;
    let context = f.call("pending-owner").await;
    let guard = f.host.claim_session(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).unwrap();
    let _receipt = f.begin(&context).await;
    let effect = f.store.list_effects(&f.session.agent_session_id).await.unwrap().remove(0);
    f.store.cancel_exact_turn(&f.session.agent_session_id,&f.turn,"cancel-pending-owner".into(),"session_api".into()).await.unwrap();
    let mut request = nomifun_agent_session::NativeEffectReconciliationRequest {
        operation_id:f.turn.clone(),expected_pause_revision:0,idempotency_key:"mcp-pending-confirmation".into(),
        effect_id:effect.effect_id.clone(),call_id:None,expected_input_digest:effect.input_digest,
        outcome:nomifun_agent_session::NativeVerifiedOutcome::ConfirmedFailed,
        evidence:nomifun_agent_session::NativeOwnerEvidence {verified:true,evidence_digest:"f".repeat(64).into(),reference:"verified the original operation outcome".into()},
        cleanup_attestation:None,
    };
    assert!(f.store.reconcile_native_effect_by_owner(&f.session.owner_ref,&f.session.agent_session_id,&request).await.is_err());
    request.cleanup_attestation = Some(nomifun_agent_session::NativeOwnerEvidence {
        verified:true,evidence_digest:"e".repeat(64).into(),reference:"independently verified original owner cleanup".into(),
    });
    f.store.reconcile_native_effect_by_owner(&f.session.owner_ref,&f.session.agent_session_id,&request).await.unwrap();
    assert!(!f.store.has_unsettled_effects(&f.session.agent_session_id).await.unwrap());
    assert!(f.host.ensure_shutdown_resources_closed(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.is_err(),
        "an independent receipt cannot release the original still-live invocation guard");
    assert!(f.host.ensure_settled(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.is_err());
    drop(guard);
    f.host.ensure_settled(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.unwrap();
}

#[tokio::test]
async fn historical_missing_cleanup_needs_attestation_and_malformed_cleanup_fails_closed() {
    for malformed in [false,true] {
        let f = Fixture::new().await;
        let context = f.call("historical-cleanup").await;
        let _receipt = f.begin(&context).await;
        let effect = f.store.list_effects(&f.session.agent_session_id).await.unwrap().remove(0);
        let mut payload = json!({"observation":{"historical_owner_error":"outcome unknown"}});
        if malformed { payload["cleanup"] = json!({"local":"definitely-clean","remote":"confirmed"}); }
        f.store.record_effect_terminal(nomifun_agent_session::EffectEventRequest {
            agent_session_id:f.session.agent_session_id.clone(),effect_id:effect.effect_id.clone(),turn_id:effect.turn_id.clone(),
            operation_id:effect.operation_id.clone(),owner_domain:effect.owner_domain.clone(),
            capability_module:effect.capability_module.clone(),action_id:effect.action_id.clone(),
            resource_binding_id:effect.resource_binding_id.clone(),resource_key:effect.resource_key.clone(),input_digest:effect.input_digest.clone(),
            recorded_at:nomifun_common::now_ms(),event_id:format!("historical-unknown:{}",effect.effect_id).into(),
            producer_id:"owning_plugin".into(),idempotency_key:format!("effect:{}",effect.effect_id).into(),
            correlation_id:effect.effect_id.clone().into(),strategy:effect.strategy,causation_event_id:Some(effect.started_event_id),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
        },nomifun_agent_session::EffectTerminalState::Uncertain).await.unwrap();
        assert!(f.host.ensure_shutdown_resources_closed(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.is_err(),
            "a historical/malformed receipt is not an original cleanup return");
        f.store.cancel_exact_turn(&f.session.agent_session_id,&f.turn,"cancel-historical-cleanup".into(),"session_api".into()).await.unwrap();
        let mut request = nomifun_agent_session::NativeEffectReconciliationRequest {
            operation_id:f.turn.clone(),expected_pause_revision:0,idempotency_key:"historical-cleanup-confirmation".into(),
            effect_id:effect.effect_id,call_id:None,expected_input_digest:effect.input_digest,
            outcome:nomifun_agent_session::NativeVerifiedOutcome::ConfirmedFailed,
            evidence:nomifun_agent_session::NativeOwnerEvidence {verified:true,evidence_digest:"d".repeat(64).into(),reference:"verified historical business outcome".into()},
            cleanup_attestation:None,
        };
        assert!(f.store.reconcile_native_effect_by_owner(&f.session.owner_ref,&f.session.agent_session_id,&request).await.is_err());
        request.cleanup_attestation = Some(nomifun_agent_session::NativeOwnerEvidence {
            verified:true,evidence_digest:"e".repeat(64).into(),reference:"independently verified historical owner cleanup".into(),
        });
        let reconciled = f.store.reconcile_native_effect_by_owner(&f.session.owner_ref,&f.session.agent_session_id,&request).await;
        if malformed {
            assert!(matches!(reconciled,Err(nomifun_agent_session::SessionStoreError::InvalidEvent(_))));
            assert!(f.host.ensure_settled(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.is_err());
        } else {
            reconciled.unwrap();
            f.host.ensure_settled(&f.session.owner_ref.principal_id,f.session.agent_session_id.as_ref()).await.unwrap();
        }
    }
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_stdio_cancellation_proves_resource_closure_without_authorizing_dispatch_or_resume() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("original-mcp-request.json");
    let script = root.path().join("original-pending-mcp.ps1");
    std::fs::write(&script, r#"$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$pending=$null
while($null-ne($line=[Console]::ReadLine())) {
  $request=ConvertFrom-Json -InputObject $line
  switch([string]$request.method) {
    'initialize' {[Console]::WriteLine((@{jsonrpc='2.0';id=$request.id;result=@{protocolVersion=$request.params.protocolVersion;capabilities=@{tools=@{}};serverInfo=@{name='original-owner';version='1'}}}|ConvertTo-Json -Depth 20 -Compress))}
    'tools/list' {[Console]::WriteLine((@{jsonrpc='2.0';id=$request.id;result=@{tools=@(@{name='echo';inputSchema=@{type='object';properties=@{text=@{type='string'}};required=@('text')}})}}|ConvertTo-Json -Depth 20 -Compress))}
    'tools/call' {
      if($null-ne$pending){throw 'Only one original business call is authorized'}
      $pending=$request.id
      [IO.File]::WriteAllText($env:NOMI_MCP_ORIGINAL_MARKER,(@{pid=$PID;request_id=$pending;text=$request.params.arguments.text}|ConvertTo-Json -Compress))
    }
    'notifications/cancelled' {
      if([string]$request.params.requestId-eq[string]$pending){
        [Console]::WriteLine((@{jsonrpc='2.0';id=$pending;error=@{code=-32800;message='Original request cancelled'}}|ConvertTo-Json -Depth 20 -Compress))
        $pending=$null
      }
    }
  }
  [Console]::Out.Flush()
}
"#).unwrap();
    let powershell = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    assert!(powershell.is_file());
    let configuration = json!({"command":powershell,"args":["-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File",script],
        "env":{"NOMI_MCP_ORIGINAL_MARKER":marker}}).to_string();
    let mut fixture = Fixture::with_transport("stdio", &configuration).await;
    fixture.host.owner = McpOwnerAdapter::new(Arc::new(McpOwner::new(
        Arc::new(nomifun_mcp::AnonymousMcpCredentialAuthority),
        reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap(),
    )));
    let f = Arc::new(fixture);
    let checkpoint_state = StrictJsonValue(json!({"version":1,"model_steps":0,"turn_operation_id":f.turn,
        "active_set_generation":0,"binding":{"agent_session_id":f.session.agent_session_id,
            "resolved_snapshot_ref":f.session.agent_binding.resolved_snapshot_ref}}));
    let checkpoint_digest = digest_payload(&checkpoint_state.0).unwrap();
    let mut checkpoint_event = event(&f.session.agent_session_id, "native-resource-checkpoint", "runtime/progress-recorded", f.turn.as_ref(),
        Some(f.started.clone()), json!({"producer_seq":1,"event":{"event":"execution_checkpoint_saved","step":0,"revision":1,"digest":checkpoint_digest}}));
    checkpoint_event.producer_id = "runtime_supervisor".into();
    let checkpoint = f.store.save_native_checkpoint(&checkpoint_event, nomifun_agent_session::NativeCheckpointWrite {
        owner:f.session.owner_ref.clone(),operation_id:f.turn.clone(),snapshot:f.session.agent_binding.resolved_snapshot_ref.clone(),
        active_set_generation:0,expected_revision:0,execution_fence:f.lease.fence(),lease:Some(f.lease.clone()),state:checkpoint_state.clone(),
    }).await.unwrap();
    let context = f.call("native-pending").await;
    let next = f.call("must-not-redispatch").await;
    let cancellation = context.control.cancellation();
    let owner = f.clone();
    let invocation = tokio::spawn(async move {
        owner.host.invoke(context, StrictJsonValue(json!({"text":"original-native-pending"}))).await
    });
    let observed: Value = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if let Ok(bytes) = std::fs::read(&marker) {
                if let Ok(value) = serde_json::from_slice(&bytes) { break value; }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("the actual stdio child must receive the original tools/call");
    assert!(observed["pid"].as_u64().unwrap() > 0);
    assert_eq!(observed["text"], "original-native-pending");
    assert!(f.host.ensure_shutdown_resources_closed(&f.session.owner_ref.principal_id, f.session.agent_session_id.as_ref()).await.is_err(),
        "the original live invocation and pending receipt still own cleanup");
    cancellation.cancel();
    let error = tokio::time::timeout(std::time::Duration::from_secs(15), invocation).await.unwrap().unwrap().unwrap_err();
    assert_eq!(error.canonical_code().as_ref(), "MCP_REQUEST_CANCELLED");
    let effects = f.store.list_effects(&f.session.agent_session_id).await.unwrap();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].state, AgentEffectState::Unknown);
    let payload = effects[0].bounded_observation.as_ref().unwrap();
    assert_eq!(payload["execution"]["local_process_started"], true);
    assert_eq!(payload["execution"]["returned"], false);
    assert_eq!(payload["cleanup"], json!({"local":"confirmed","remote":"not_required","error":null}));
    let cursor = f.store.current_cursor(&f.session.agent_session_id).await.unwrap();
    f.host.ensure_shutdown_resources_closed(&f.session.owner_ref.principal_id, f.session.agent_session_id.as_ref()).await.unwrap();
    let hosted = super::super::hosted_effect_receipts::HostedEffectReceipts::new(f._database.pool().clone());
    hosted.ensure_shutdown_resources_closed(&f.session.owner_ref.principal_id, f.session.agent_session_id.as_ref()).await.unwrap();
    assert!(f.host.ensure_settled(&f.session.owner_ref.principal_id, f.session.agent_session_id.as_ref()).await.is_err());
    assert!(hosted.ensure_settled(&f.session.owner_ref.principal_id, f.session.agent_session_id.as_ref()).await.is_err());
    let blocked = f.host.invoke(next, StrictJsonValue(json!({"text":"must not restart"}))).await.unwrap_err();
    assert_eq!(blocked.canonical_code().as_ref(), "MCP_SESSION_UNCERTAIN");
    assert_eq!(f.store.current_cursor(&f.session.agent_session_id).await.unwrap(), cursor,
        "physical shutdown observations never settle, reconcile or replay business state");
    assert_eq!(f.store.list_effects(&f.session.agent_session_id).await.unwrap(), effects);
    let pause = f.store.pause_native_execution(&f.lease, "NATIVE_RECOVERY_RECONCILIATION_REQUIRED", true).await.unwrap();
    let head = f.store.head(&f.session.agent_session_id).await.unwrap();
    let paused_cursor = f.store.current_cursor(&f.session.agent_session_id).await.unwrap();
    f.host.ensure_shutdown_resources_closed(&f.session.owner_ref.principal_id, f.session.agent_session_id.as_ref()).await.unwrap();
    let request = nomifun_agent_session::NativeResumeRequest {
        operation_id:f.turn.clone(),idempotency_key:"do-not-resume-unknown-business".into(),
        expected_pause_revision:pause.revision,expected_checkpoint_revision:checkpoint.revision,
        expected_checkpoint_digest:checkpoint.digest,budget:Default::default(),cleanup_attestation:None,
    };
    let resumed = f.store.commit_native_resume(&f.session.owner_ref,&f.session.agent_session_id,&request,
        nomifun_agent_session::NativeResumePreparation {
            expected_head_seq:head.last_seq,expected_fence:pause.execution_fence,
            snapshot:f.session.agent_binding.resolved_snapshot_ref.clone(),active_set_generation:head.active_set_generation,
            checkpoint_state,observations:vec![],
        }).await;
    assert!(matches!(resumed, Err(nomifun_agent_session::SessionStoreError::RecoveryRequiresReconciliation)),
        "native resource cleanup cannot authorize replay of an unknown business operation: {resumed:?}");
    assert_eq!(f.store.current_cursor(&f.session.agent_session_id).await.unwrap(), paused_cursor);
    assert_eq!(f.store.list_effects(&f.session.agent_session_id).await.unwrap(), effects);
}
