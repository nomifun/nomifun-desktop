import{test,expect}from'bun:test';
import{join}from'node:path';
import{additionalUnsettledSession as sessionId,freezeWindowsAdditionalSessionFacts,assertWindowsAdditionalSessionPreservation}from'./global-os-windows-unsettled-preservation.mjs';
const effectId='wave2:44e7761f6e75732c4d73bf4ee17d9aa65c4cc4ad5a7673650f63510bb0f0a022',inputDigest='a4b6c5d764c35d56fa72bc3b23a0d41f920fb96d15267f669aa4163d78d35322';
const sourceSha='d203dc3939436a75ccf8189d1db075ddc56d112a230ccabc7ef53a97566ea67e',tokenSha='b'.repeat(64),operation='original-turn';
function sample(){
  const parent=join('fixture','nomifun-global-os-owner-11111111-1111-4111-8111-111111111111');
  const fixture={schema:'nomifun.windows-global-os-fixture.v1',status:'ready',parent,native_data_root:join(parent,'native-data'),token_file:join(parent,'owner-jwt'),app_pid:30552};
  const event=(seq,kind,body)=>({agent_session_id:sessionId,seq,event_id:`original-${seq}`,kind,producer_id:'runtime_supervisor',payload:{storage:'inline_json',value:body}});
  const events=Array.from({length:336},(_,index)=>event(index+1,'runtime/progress-recorded',{event:{event:'fixture'}}));
  const identity={effect_id:effectId,turn_id:operation,operation_id:operation+':tool',owner_domain:'workspace',capability_module:'workspace.files',action_id:'workspace.files/patch',input_digest:inputDigest,resource_binding_id:'original-binding',resource_key:'workspace:original-key',strategy:'managed_effect'};
  events[296]={...event(297,'effect/started',{...identity,recorded_at:123}),event_id:effectId+':started',producer_id:'capability_host',idempotency_key:effectId+':lifecycle',correlation_id:effectId};
  events[298]=event(299,'tool/result-recorded',{operation_id:identity.operation_id,error:'Capability Kernel rejected tool (EFFECT_OUTCOME_UNKNOWN): original receipt'});
  events[334]=event(335,'turn/paused',{pause:{reason:'EXECUTION_CLEANUP_UNPROVEN',cleanup_proven:false}});
  events[335]=event(336,'turn/cancelled',{target_operation_id:operation});
  const observation={session_id:sessionId,app_pid:30552,mutating_requests:0,events,
    session:{session:{agent_session_id:sessionId,owner_ref:{principal_kind:'user',principal_id:'original-owner'},agent_binding:{resolved_snapshot_ref:{snapshot_id:'original-snapshot',snapshot_digest:'a'.repeat(64)},typed_resource_bindings:[{resource_id:'original-resource',owner_id:'original-owner'}]},metadata:{title:'original'},next_seq:337},head:{session_id:sessionId,status:'ready',active_set_generation:0,last_seq:336,unread_count:0}},
    execution:{operation_id:operation,state:'cancelled',turn_state:'cancelled',execution_fence:1,pending_effects:1,unknown_effects:0,producer_lease_live:false,checkpoint_retained:false,automatic_replay_authorized:false,pause_requested:false,recovery_blocked:false,checkpoint_seq:null,checkpoint_digest:null,pause:null},
    effects:{items:[{effect_id:effectId,operation_id:identity.operation_id,owner_domain:'workspace',action_id:'workspace.files/patch',input_digest:inputDigest,state:'pending'}],has_more:false,automatic_replay_authorized:false,unresolved_invocations:[]}};
  const before=freezeWindowsAdditionalSessionFacts({fixture,tokenSha256:tokenSha,observation,originalObservation:structuredClone(observation),sourceObservationPath:'original.json',sourceObservationSha256:sourceSha});
  return{fixture,tokenSha256:tokenSha,before,observation};
}
test('additional cancelled Session freezes all 336 facts and retains the original pending owner/resource identity',()=>{
  const input=sample(),result=assertWindowsAdditionalSessionPreservation(input);
  expect(result.transition).toBe('pending_preserved');expect(result.original_336_events_unchanged).toBe(true);
  expect(result.pending_effects).toBe(1);expect(result.driver_mutating_requests).toBe(0);expect(result.driver_reconciliation_requests).toBe(0);
});
test('additional Session cannot lose a pending effect, edit old facts, replace resources or switch dataset/token',()=>{
  for(const mutate of [x=>{x.observation.execution.pending_effects=0;},x=>{x.observation.effects.items=[];},x=>{x.observation.events[298].payload.value.error='rewritten';},x=>{x.observation.events[296].payload.value.input_digest='changed';},x=>{x.observation.session.session.owner_ref.principal_id='other';},x=>{x.observation.session.session.agent_binding.typed_resource_bindings[0].resource_id='replacement';},x=>{x.fixture.native_data_root+='-new';},x=>{x.tokenSha256='c'.repeat(64);},x=>{x.observation.execution.automatic_replay_authorized=true;}]){
    const input=sample();mutate(input);expect(()=>assertWindowsAdditionalSessionPreservation(input)).toThrow();
  }
});
function restart(){
  const input=sample(),{before,observation}=input,body={...before.effect_identity,outcome:'unknown',recovery:'process_restart_external_reconciliation_required',recorded_at:456};
  const proof={agent_session_id:sessionId,seq:337,event_id:`native-recovery-effect:${sessionId}:${effectId}:2`,kind:'effect/uncertain',producer_id:'runtime_supervisor',causation_event_id:before.effect_started_event.event_id,idempotency_key:before.effect_started_event.idempotency_key,correlation_id:effectId,payload:{storage:'inline_json',value:body}};
  const blocked={agent_session_id:sessionId,seq:338,event_id:`native-recovery-blocked:${sessionId}:${operation}:2`,kind:'runtime/execution-recovery-blocked',producer_id:'runtime_supervisor',correlation_id:operation,payload:{storage:'inline_json',value:{code:'NATIVE_RECOVERY_RECONCILIATION_REQUIRED',execution_fence:2,quarantined_effects:1,automatic_replay_allowed:false,next_action:'owner_reconciliation_required'}}};
  observation.events.push(proof,blocked);observation.execution={...observation.execution,execution_fence:2,pending_effects:0,unknown_effects:1,state:'paused',recovery_blocked:true};observation.effects.items[0].state='unknown';observation.session.head.last_seq=338;
  return input;
}
test('only a complete matching typed native restart quarantine can turn pending into unknown',()=>{
  expect(assertWindowsAdditionalSessionPreservation(restart()).transition).toBe('native_restart_unknown_preserved');
  for(const mutate of [x=>{x.observation.events.pop();},x=>{x.observation.events[336].producer_id='owning_plugin';},x=>{x.observation.events[336].payload.value.recovery='text contains process_restart_external_reconciliation_required but is not proof';},x=>{x.observation.events[336].payload.value.resource_key='other';},x=>{x.observation.events[336].causation_event_id='other';},x=>{x.observation.events[337].payload.value.automatic_replay_allowed=true;},x=>{x.observation.execution.unknown_effects=0;}]){
    const input=restart();mutate(input);expect(()=>assertWindowsAdditionalSessionPreservation(input)).toThrow();
  }
});
test('a new call, manual attestation or reconciliation cannot satisfy upgrade preservation',()=>{
  for(const kind of ['effect/started','tool/call-started','turn/started','runtime/effect-reconciliation-attested','effect/reconciled']){
    const input=restart();input.observation.events.push({agent_session_id:sessionId,seq:339,event_id:'new-fact',kind,producer_id:'runtime_supervisor',payload:{storage:'inline_json',value:{effect_id:effectId}}});
    expect(()=>assertWindowsAdditionalSessionPreservation(input)).toThrow();
  }
});
