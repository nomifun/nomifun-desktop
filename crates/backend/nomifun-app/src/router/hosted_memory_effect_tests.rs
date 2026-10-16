// Included in hosted_effect_receipts::tests so these focused tests can call
// the same private repair core without inventing a Wave1StateHandle.
use crate::router::agent_wave1_memory_receipts as memory_receipt_owner;

struct MemoryCase {
    database: nomifun_db::Database,
    store: AgentSessionStore,
    receipts: HostedEffectReceipts,
    session: AgentSessionLiveRecord,
    turn: OperationId,
    operation: OperationId,
    capability: CapabilityId,
    action: ActionId,
    resources: Vec<nomifun_agent_contracts::TypedResourceBinding>,
    companion: String,
    root: nomifun_agent_contracts::SessionEventRecord,
    input: Value,
    ledger: memory_receipt_owner::CompanionMemoryWriteLedger,
    memories: nomifun_companion::store::CompanionStore,
    _lease: nomifun_agent_session::NativeExecutionLease,
}

impl MemoryCase {
    async fn new(start_effect: bool) -> Self {
        let owner = uuid::Uuid::now_v7().to_string();
        let database = nomifun_db::init_database_memory_with_owner(nomifun_common::UserId::parse(owner.clone()).unwrap()).await.unwrap();
        let store = AgentSessionStore::from_pool(database.pool().clone()).await.unwrap();
        let companion = uuid::Uuid::now_v7().to_string(); let id = uuid::Uuid::now_v7().to_string();
        let resources = ["companion","companion_memory"].into_iter().map(|kind| {
            let mut binding = nomifun_agent_contracts::TypedResourceBinding {binding_id:"pending".into(),resource_kind:kind.into(),
                resource_id:companion.clone().into(),owner_id:owner.clone(),operations:["read".into(),"write".into()].into(),
                connection_config_ref:None,typed_parameters:BTreeMap::new()};
            binding.binding_id = nomifun_agent_contracts::resource_definition_id(&binding).unwrap(); binding
        }).collect::<Vec<_>>();
        let session = AgentSessionLiveRecord {agent_session_id:id.clone().into(),owner_ref:PrincipalRef {principal_kind:"user".into(),principal_id:owner},
            metadata:AgentSessionMetadata {purpose:Default::default(),app_project_id:None,title:Some("Canonical memory receipt repair".into()),
                archived:false,pinned:false,reasoning_effort:None,companion:Some(nomifun_agent_contracts::CompanionSessionOrigin {
                    companion_id:companion.clone(),role:nomifun_agent_contracts::CompanionSessionRole::Companion,robot_id:None})},
            agent_binding:AgentBindingValue {preset_revision_ref:PresetRevisionRef {preset_id:uuid::Uuid::now_v7().to_string().into(),revision:1,revision_digest:"a".repeat(64).into()},
                resolved_snapshot_ref:ResolvedSnapshotRef {snapshot_id:"memory-repair-snapshot".into(),snapshot_digest:"b".repeat(64).into()},
                typed_resource_bindings:resources.clone(),binding_version:1},remote_binding_provenance:None,parent_session_id:None,fork_base_payload_id:None,next_seq:1};
        let created = store.create_session(CreateSessionRequest::new(session,nomifun_common::now_ms(),OperationId::from(format!("create-memory:{id}")),
            EventProducerId::from("session_api"),IdempotencyKey::from(format!("create-memory:{id}")),CorrelationId::from(id.clone()))).await.unwrap();
        store.append_event(&event(&created.session.agent_session_id,"memory-ready","session/ready",&id,Some(created.opening_ack.event_id),json!({}))).await.unwrap();
        let turn: OperationId = format!("memory-parent:{id}").into(); let operation: OperationId = format!("memory-write:{id}").into();
        let route = nomifun_agent_contracts::ChatRouteIdentity::new("memory-repair@1",nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,"memory-route".into(),1);
        let (root,start) = store.start_turn(&created.session.agent_session_id,"session_api".into(),"memory-user-root".into(),turn.clone(),
            StrictJsonValue(json!({"content":"Remember this exact durable preference.","admission":{"route_identity":route,
                "resolved_snapshot_ref":created.session.agent_binding.resolved_snapshot_ref}}))).await.unwrap();
        let root = root.record.unwrap(); let start = start.record.unwrap();
        let lease = store.claim_native_execution(nomifun_agent_session::NativeExecutionClaim {owner:created.session.owner_ref.clone(),
            agent_session_id:created.session.agent_session_id.clone(),operation_id:turn.clone(),snapshot:created.session.agent_binding.resolved_snapshot_ref.clone(),
            active_set_generation:0,holder:"memory-repair-native-owner".into(),expected_fence:0,checkpoint:None}).await.unwrap();
        let capability: CapabilityId = "companion.memory".into(); let action: ActionId = "companion.memory/write".into();
        let mut call = event(&created.session.agent_session_id,"memory-tool","tool/call-started","memory-write-call",Some(start.event_id),
            json!({"operation_id":operation,"call_id":"memory-write-call","capability_id":capability,"action_id":action}));
        call.producer_id = "runtime_supervisor".into();
        store.append_native_event(&lease,&call,None).await.unwrap();
        let case = Self {receipts:HostedEffectReceipts::new(database.pool().clone()),ledger:memory_receipt_owner::CompanionMemoryWriteLedger::new(database.pool().clone()),
            database,store,session:created.session,turn,operation,capability,action,resources,companion,root,
            input:json!({"kind":"preference","content":"One memory owner mutation, with a separately durable receipt.","tags":[]}),
            memories:nomifun_companion::store::CompanionStore::open_memory().await.unwrap(),_lease:lease};
        // These are the real public native facts checked by begin_wave1. Only
        // the exact Effect writer is called directly to avoid a fake StateHandle.
        let native = case.store.native_tool_call_causation(&case.session.owner_ref,&case.session.agent_session_id,&case.turn,&case.operation,&case.capability,&case.action).await.unwrap();
        assert_eq!(native.execution_scope.resolved_snapshot_ref(),&case.session.agent_binding.resolved_snapshot_ref);
        if start_effect {
            let receipt = case.receipts.begin_exact(case.store.clone(),case.session.agent_session_id.clone(),case.turn.clone(),case.operation.as_ref(),
                case.capability.as_ref(),case.action.as_ref(),&case.input,Domain::CompanionMemory,Some(case.resource_key()),true).await.unwrap();
            assert_eq!(receipt.exact_request().causation_event_id.as_ref(),Some(&native.tool_call.event_id));
        }
        case
    }

    fn authority(&self) -> InvocationAuthority<'_> {
        InvocationAuthority {principal:&self.session.owner_ref,session:&self.session.agent_session_id,turn:&self.turn,operation:&self.operation,
            snapshot:&self.session.agent_binding.resolved_snapshot_ref,capability:&self.capability,action:&self.action}
    }

    fn resource_key(&self) -> String { format!("companion-memory:{}:{}",self.session.owner_ref.principal_id,self.companion) }

    fn owner_context(&self) -> memory_receipt_owner::CompanionMemoryWriteReceiptContext<'_> {
        memory_receipt_owner::CompanionMemoryWriteReceiptContext {owner_user_id:&self.session.owner_ref.principal_id,
            agent_session_id:self.session.agent_session_id.as_ref(),module_id:self.capability.as_ref(),action_id:self.action.as_ref(),
            idempotency_key:self.operation.as_ref(),target_companion_id:&self.companion}
    }

    async fn write_owner_once(&self, known: bool) -> Value {
        let memory_receipt_owner::ReceiptAdmission::Execute(mut guard) = self.ledger.admit(self.owner_context(),&StrictJsonValue(self.input.clone())).await.unwrap()
            else {panic!("the fresh owner command must win its durable receipt");};
        let memory = self.memories.add_agent_memory_with_sources("preference",self.input["content"].as_str().unwrap(),&[],&self.companion,
            self.session.agent_session_id.as_ref(),self.operation.as_ref(),&[nomifun_companion::store::CompanionMemoryMessageSource {
                session_id:self.session.agent_session_id.as_ref().into(),message_id:self.root.event_id.as_ref().into()}]).await.unwrap();
        let value = serde_json::to_value(memory).unwrap();
        let output = if known {Ok(StrictJsonValue(value.clone()))} else {Err(nomifun_agent_domain_wave1::Wave1HostPortError::new(
            memory_receipt_owner::ACTION_OUTCOME_UNKNOWN,"the memory mutation outcome needs explicit reconciliation"))};
        self.ledger.settle(&mut guard,&output).await.unwrap(); value
    }

    async fn owner_replay(&self) -> Result<StrictJsonValue,nomifun_agent_domain_wave1::Wave1HostPortError> {
        match self.ledger.admit(self.owner_context(),&StrictJsonValue(self.input.clone())).await.unwrap() {
            memory_receipt_owner::ReceiptAdmission::Return(result) => result,
            memory_receipt_owner::ReceiptAdmission::Execute(_) => panic!("a durable owner receipt cannot dispatch again"),
        }
    }

    async fn cancel_parent(&self) -> nomifun_agent_session::TurnReceipt {
        self.store.cancel_exact_turn(&self.session.agent_session_id,&self.turn,"memory-parent-cancel".into(),"session_api".into()).await.unwrap();
        assert!(self.store.native_tool_call_causation(&self.session.owner_ref,&self.session.agent_session_id,&self.turn,&self.operation,&self.capability,&self.action).await.is_err());
        self.store.committed_tool_call_causation(&self.session.agent_session_id,&self.turn,&self.operation,&self.capability,&self.action).await.unwrap();
        self.store.read_turn_receipt(&self.session.agent_session_id,&self.turn).await.unwrap()
    }

    async fn rows(&self) -> Vec<nomifun_companion::store::CompanionMemory> {
        self.memories.list_memories(&nomifun_companion::store::MemoryFilter {companion_id:Some(self.companion.clone()),limit:10,..Default::default()}).await.unwrap()
    }
}

#[tokio::test]
async fn companion_memory_effect_durable_known_replay_repairs_only_the_pending_terminal_after_parent_closes() {
    let case = MemoryCase::new(true).await; let output = case.write_owner_once(true).await;
    let rows = case.rows().await; assert_eq!(rows.len(),1);
    let original = case.store.list_effects(&case.session.agent_session_id).await.unwrap().pop().unwrap(); assert_eq!(original.state,AgentEffectState::Pending);
    let parent = case.cancel_parent().await;
    assert_eq!(case.store.read_effect(&case.session.agent_session_id,&original.effect_id).await.unwrap().unwrap().state,AgentEffectState::Pending,
        "closing the parent never invents a settled memory effect");
    let before = case.store.current_cursor(&case.session.agent_session_id).await.unwrap();
    let replay = case.owner_replay().await.unwrap(); assert_eq!(replay.0,output);
    case.receipts.settle_memory_replay_known(&case.store,case.authority(),&case.resources,&case.input,true,&replay.0).await.unwrap();
    let settled = case.store.list_effects(&case.session.agent_session_id).await.unwrap(); assert_eq!(settled.len(),1); assert_eq!(settled[0].state,AgentEffectState::Returned);
    assert_eq!(settled[0].started_event_id,original.started_event_id); assert_eq!(settled[0].effect_id,original.effect_id);
    let events = case.store.read_events(&case.session.agent_session_id,None,100).await.unwrap().events;
    assert_eq!(events.iter().filter(|event|event.kind.0 == "effect/started").count(),1);
    let terminal = events.iter().find(|event|event.kind.0 == "effect/succeeded").unwrap(); assert_eq!(terminal.causation_event_id.as_ref(),Some(&original.started_event_id));
    assert_eq!(case.store.current_cursor(&case.session.agent_session_id).await.unwrap().seq,before.seq+1);
    let repaired = case.store.current_cursor(&case.session.agent_session_id).await.unwrap();
    case.receipts.settle_memory_replay_known(&case.store,case.authority(),&case.resources,&case.input,true,&case.owner_replay().await.unwrap().0).await.unwrap();
    assert_eq!(case.store.current_cursor(&case.session.agent_session_id).await.unwrap(),repaired);
    assert_eq!(case.rows().await,rows); assert_eq!(case.store.read_turn_receipt(&case.session.agent_session_id,&case.turn).await.unwrap(),parent);
    case.database.close().await;
}

#[tokio::test]
async fn companion_memory_effect_legacy_known_owner_receipt_never_fabricates_a_historical_start() {
    let case = MemoryCase::new(false).await; case.write_owner_once(true).await; let rows = case.rows().await;
    case.cancel_parent().await; let before = case.store.current_cursor(&case.session.agent_session_id).await.unwrap();
    let replay = case.owner_replay().await.unwrap();
    case.receipts.settle_memory_replay_known(&case.store,case.authority(),&case.resources,&case.input,true,&replay.0).await.unwrap();
    assert!(case.store.list_effects(&case.session.agent_session_id).await.unwrap().is_empty());
    assert_eq!(case.store.current_cursor(&case.session.agent_session_id).await.unwrap(),before); assert_eq!(case.rows().await,rows);
    case.database.close().await;
}

#[tokio::test]
async fn companion_memory_effect_repair_rejects_changed_snapshot_or_input_digest_without_mutating_the_owner() {
    let case = MemoryCase::new(true).await; case.write_owner_once(true).await; let rows = case.rows().await; case.cancel_parent().await;
    let before = case.store.current_cursor(&case.session.agent_session_id).await.unwrap(); let replay = case.owner_replay().await.unwrap();
    let changed = ResolvedSnapshotRef {snapshot_id:"changed-memory-snapshot".into(),snapshot_digest:"c".repeat(64).into()};
    assert!(case.receipts.settle_memory_replay_known(&case.store,InvocationAuthority {snapshot:&changed,..case.authority()},&case.resources,&case.input,true,&replay.0).await.is_err());
    let mut other_input = case.input.clone(); other_input["content"] = json!("a different correction cannot borrow the completed receipt");
    assert!(case.receipts.settle_memory_replay_known(&case.store,case.authority(),&case.resources,&other_input,true,&replay.0).await.is_err());
    assert_eq!(case.ledger.admit(case.owner_context(),&StrictJsonValue(other_input)).await.unwrap_err().code.as_ref(),memory_receipt_owner::IDEMPOTENCY_CONFLICT);
    assert_eq!(case.store.current_cursor(&case.session.agent_session_id).await.unwrap(),before); assert_eq!(case.rows().await,rows);
    let effect = case.store.list_effects(&case.session.agent_session_id).await.unwrap().pop().unwrap(); assert_eq!(effect.state,AgentEffectState::Pending); assert!(effect.terminal_event_id.is_none());
    case.database.close().await;
}

#[tokio::test]
async fn companion_memory_effect_unknown_canonical_or_owner_outcomes_remain_fenced() {
    let case = MemoryCase::new(true).await; case.write_owner_once(true).await; let rows = case.rows().await;
    let effect = case.store.list_effects(&case.session.agent_session_id).await.unwrap().pop().unwrap();
    let mut uncertain = EffectEventRequest {agent_session_id:effect.agent_session_id.clone(),effect_id:effect.effect_id.clone(),turn_id:effect.turn_id.clone(),
        operation_id:effect.operation_id.clone(),owner_domain:effect.owner_domain.clone(),capability_module:effect.capability_module.clone(),action_id:effect.action_id.clone(),
        resource_binding_id:effect.resource_binding_id.clone(),resource_key:effect.resource_key.clone(),input_digest:effect.input_digest.clone(),recorded_at:nomifun_common::now_ms(),
        event_id:"memory-effect-uncertain".into(),producer_id:"owning_plugin".into(),idempotency_key:format!("effect:{}",effect.effect_id).into(),
        correlation_id:effect.effect_id.clone().into(),strategy:effect.strategy,causation_event_id:Some(effect.started_event_id.clone()),
        payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"unconfirmed":true})))};
    uncertain.recorded_at = nomifun_common::now_ms();
    case.store.record_effect_terminal(uncertain,EffectTerminalState::Uncertain).await.unwrap(); case.cancel_parent().await;
    let before = case.store.current_cursor(&case.session.agent_session_id).await.unwrap(); let replay = case.owner_replay().await.unwrap();
    assert!(case.receipts.settle_memory_replay_known(&case.store,case.authority(),&case.resources,&case.input,true,&replay.0).await.is_err());
    assert_eq!(case.store.list_effects(&case.session.agent_session_id).await.unwrap()[0].state,AgentEffectState::Unknown);
    assert!(case.store.has_unsettled_effects(&case.session.agent_session_id).await.unwrap()); assert_eq!(case.rows().await,rows);
    assert_eq!(case.store.current_cursor(&case.session.agent_session_id).await.unwrap(),before); case.database.close().await;

    let owner_unknown = MemoryCase::new(true).await; owner_unknown.write_owner_once(false).await; let rows = owner_unknown.rows().await; owner_unknown.cancel_parent().await;
    let before = owner_unknown.store.current_cursor(&owner_unknown.session.agent_session_id).await.unwrap();
    let unknown = owner_unknown.owner_replay().await.unwrap_err(); assert_eq!(unknown.code.as_ref(),memory_receipt_owner::ACTION_OUTCOME_UNKNOWN);
    // The caller has no durable known owner result and therefore never invokes
    // the repair core. Its original pending Effect keeps execution fenced.
    let pending = owner_unknown.store.list_effects(&owner_unknown.session.agent_session_id).await.unwrap(); assert_eq!(pending.len(),1); assert_eq!(pending[0].state,AgentEffectState::Pending);
    assert!(owner_unknown.store.has_unsettled_effects(&owner_unknown.session.agent_session_id).await.unwrap());
    assert_eq!(owner_unknown.store.current_cursor(&owner_unknown.session.agent_session_id).await.unwrap(),before); assert_eq!(owner_unknown.rows().await,rows);
    owner_unknown.database.close().await;
}
