// Read-only upgrade evidence. This never calls a control, reconciliation,
// model, tool or database write endpoint and grants no replay authority.
import assert from 'node:assert/strict';
import {createHash,randomUUID} from 'node:crypto';
import {lstat,readFile,writeFile} from 'node:fs/promises';
import {basename,isAbsolute,join,resolve} from 'node:path';
import {assertWindowsPrivatePath,readWindowsOwnedFixtureBytes} from './global-os-windows-cases.mjs';

export const additionalUnsettledSession='01a14893-93f7-7a90-b573-bc0b31116207';
const effectId='wave2:44e7761f6e75732c4d73bf4ee17d9aa65c4cc4ad5a7673650f63510bb0f0a022';
const inputDigest='a4b6c5d764c35d56fa72bc3b23a0d41f920fb96d15267f669aa4163d78d35322';
const originalObservationSha='d203dc3939436a75ccf8189d1db075ddc56d112a230ccabc7ef53a97566ea67e';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const canonical=value=>JSON.stringify(value,(_,entry)=>entry&&typeof entry==='object'&&!Array.isArray(entry)?Object.fromEntries(Object.keys(entry).sort().map(key=>[key,entry[key]])):entry);
const digest=value=>hash(canonical(value));
const value=event=>{assert.equal(event.payload?.storage,'inline_json');return event.payload.value;};
const identityKeys=['effect_id','turn_id','operation_id','owner_domain','capability_module','action_id','input_digest','resource_binding_id','resource_key','strategy'];
function identity(event){const body=value(event);return Object.fromEntries(identityKeys.map(key=>{assert(key in body);return[key,body[key]];}));}
function sessionFacts(session){return Object.fromEntries(Object.entries(session).filter(([key])=>key!=='next_seq'));}
function checkedEvents(events){
  assert(Array.isArray(events)&&events.length>=336&&events.length<=10000);const ids=new Set();
  events.forEach((event,index)=>{assert.equal(event.agent_session_id,additionalUnsettledSession);assert.equal(event.seq,index+1);assert(!ids.has(event.event_id));ids.add(event.event_id);});return events;
}
function candidate(effects){
  assert.equal(effects.has_more,false);assert.equal(effects.automatic_replay_authorized,false);assert.deepEqual(effects.unresolved_invocations,[]);
  assert.equal(effects.items.length,1,'The original unsettled effect cannot disappear or be replaced');
  const item=effects.items[0];assert.equal(item.effect_id,effectId);assert.equal(item.input_digest,inputDigest);assert.equal(item.action_id,'workspace.files/patch');assert.equal(item.owner_domain,'workspace');return item;
}
function strictPending(observation){
  const {session,execution,effects}=observation,events=checkedEvents(observation.events);
  assert.equal(session.session.agent_session_id,additionalUnsettledSession);assert.equal(session.head.session_id,additionalUnsettledSession);
  assert.equal(session.head.status,'ready');assert(!session.head.active_turn_id);assert.equal(session.head.last_seq,336);assert.equal(events.length,336);
  assert.equal(execution.state,'cancelled');assert.equal(execution.turn_state,'cancelled');assert.equal(execution.pending_effects,1);assert.equal(execution.unknown_effects,0);
  for(const key of ['producer_lease_live','checkpoint_retained','automatic_replay_authorized','pause_requested','recovery_blocked'])assert.equal(execution[key],false);
  for(const key of ['checkpoint_seq','checkpoint_digest','pause'])assert.equal(execution[key],null);
  const start=events[296];assert.equal(start.kind,'effect/started');assert.equal(start.event_id,`${effectId}:started`);
  const frozenIdentity=identity(start);assert.equal(frozenIdentity.effect_id,effectId);assert.equal(frozenIdentity.input_digest,inputDigest);assert.equal(frozenIdentity.strategy,'managed_effect');assert.equal(frozenIdentity.turn_id,execution.operation_id);
  assert.equal(events[298].kind,'tool/result-recorded');assert.equal(value(events[298]).operation_id,frozenIdentity.operation_id);
  assert(value(events[298]).error.startsWith('Capability Kernel rejected tool (EFFECT_OUTCOME_UNKNOWN): '));
  assert.equal(events[334].kind,'turn/paused');assert.equal(value(events[334]).pause.reason,'EXECUTION_CLEANUP_UNPROVEN');assert.equal(value(events[334]).pause.cleanup_proven,false);
  assert.equal(events[335].kind,'turn/cancelled');assert.equal(value(events[335]).target_operation_id,execution.operation_id);
  const item=candidate(effects);assert.equal(item.state,'pending');assert.equal(item.operation_id,frozenIdentity.operation_id);
  assert(!events.some(event=>['effect/succeeded','effect/failed','effect/uncertain','effect/reconciled'].includes(event.kind)&&value(event).effect_id===effectId));
  return{events,start,frozenIdentity,item};
}
export function freezeWindowsAdditionalSessionFacts({fixture,tokenSha256,observation,originalObservation,sourceObservationPath,sourceObservationSha256}){
  assert.equal(sourceObservationSha256,originalObservationSha);assert.equal(originalObservation.session_id,additionalUnsettledSession);
  assert.equal(originalObservation.mutating_requests,0);const frozen=strictPending(observation),original=strictPending(originalObservation);
  assert.deepEqual(frozen.events,original.events,'The original 336 canonical Event/Payload-reference facts must already be unchanged');
  assert.deepEqual(sessionFacts(observation.session.session),sessionFacts(originalObservation.session.session));
  assert.deepEqual(observation.execution,originalObservation.execution);assert.deepEqual(observation.effects,originalObservation.effects);
  assert.match(tokenSha256,/^[a-f0-9]{64}$/);assert.equal(fixture.schema,'nomifun.windows-global-os-fixture.v1');assert.equal(fixture.status,'ready');
  assert.equal(fixture.app_pid,originalObservation.app_pid,'Freeze against the actual original native App before it exits');
  assert.equal(fixture.native_data_root,join(fixture.parent,'native-data'));assert.equal(fixture.token_file,join(fixture.parent,'owner-jwt'));
  return structuredClone({schema:'nomifun.windows-unsettled-upgrade-preservation.v1',session_id:additionalUnsettledSession,app_pid:fixture.app_pid,
    fixture:{parent:fixture.parent,native_data_root:fixture.native_data_root,token_file:fixture.token_file,owner_jwt_sha256:tokenSha256},
    source_observation:{path:sourceObservationPath,sha256:sourceObservationSha256},
    session:sessionFacts(observation.session.session),head:observation.session.head,execution:observation.execution,effects:observation.effects,
    effect_identity:frozen.frozenIdentity,effect_started_event:frozen.start,
    canonical_prefix:{event_count:336,last_seq:336,sha256:digest(frozen.events)},events:frozen.events,
    mutating_requests:0,automatic_replay_authorized:false});
}
export function assertWindowsAdditionalSessionPreservation({before,fixture,tokenSha256,observation}){
  assert.equal(before.schema,'nomifun.windows-unsettled-upgrade-preservation.v1');assert.equal(before.session_id,additionalUnsettledSession);
  assert.equal(before.mutating_requests,0);assert.equal(before.automatic_replay_authorized,false);
  assert.equal(before.source_observation.sha256,originalObservationSha);
  strictPending({session:{session:before.session,head:before.head},execution:before.execution,effects:before.effects,events:before.events});
  assert.deepEqual(identity(before.effect_started_event),before.effect_identity);assert.deepEqual(before.events[296],before.effect_started_event);
  for(const key of ['parent','native_data_root','token_file'])assert.equal(fixture[key],before.fixture[key],'Additional Session evidence must retain the same dataset and owner JWT path');
  assert.equal(tokenSha256,before.fixture.owner_jwt_sha256);
  const events=checkedEvents(observation.events),prefix=events.filter(event=>event.seq<=336);
  assert.equal(before.canonical_prefix.event_count,336);assert.equal(before.canonical_prefix.last_seq,336);
  assert.equal(digest(before.events),before.canonical_prefix.sha256);assert.deepEqual(prefix,before.events,'The complete original canonical prefix changed');
  assert.equal(digest(prefix),before.canonical_prefix.sha256);
  assert.deepEqual(sessionFacts(observation.session.session),before.session,'Owner, frozen binding, resources and Snapshot cannot change during upgrade');
  assert.equal(observation.session.head.session_id,additionalUnsettledSession);assert.equal(observation.session.head.active_set_generation,before.head.active_set_generation);
  const execution=observation.execution,item=candidate(observation.effects);assert.equal(execution.operation_id,before.execution.operation_id);
  assert.equal(item.operation_id,before.effect_identity.operation_id);assert.deepEqual(identity(events[296]),before.effect_identity);
  for(const key of ['producer_lease_live','checkpoint_retained','automatic_replay_authorized','pause_requested'])assert.equal(execution[key],false);
  for(const key of ['checkpoint_seq','checkpoint_digest'])assert.equal(execution[key],null);
  const additions=events.filter(event=>event.seq>336);let transition='pending_preserved',restartProof;
  if(item.state==='pending'){
    assert.equal(additions.length,0,'An upgrade must not dispatch or author new facts in the original cancelled Session');
    assert.deepEqual(execution,before.execution);assert.deepEqual(observation.effects,before.effects);assert.deepEqual(observation.session.head,before.head);
  }else{
    assert.equal(item.state,'unknown','An upgrade cannot settle the original effect');assert.equal(execution.pending_effects,0);assert.equal(execution.unknown_effects,1);
    const proofs=additions.filter(event=>event.kind==='effect/uncertain');assert.equal(proofs.length,1);const proof=proofs[0],body=value(proof);
    assert.equal(proof.producer_id,'runtime_supervisor');assert.deepEqual(identity(proof),before.effect_identity);
    assert.equal(proof.causation_event_id,before.effect_started_event.event_id);assert.equal(proof.idempotency_key,before.effect_started_event.idempotency_key);assert.equal(proof.correlation_id,effectId);
    assert.equal(body.outcome,'unknown');assert.equal(body.recovery,'process_restart_external_reconciliation_required');
    assert(Number.isSafeInteger(body.recorded_at)&&body.recorded_at>=value(before.effect_started_event).recorded_at);
    const fence=execution.execution_fence;assert(Number.isSafeInteger(fence)&&fence>before.execution.execution_fence);
    assert.equal(proof.event_id,`native-recovery-effect:${additionalUnsettledSession}:${effectId}:${fence}`);
    const blocked=additions.find(event=>event.event_id===`native-recovery-blocked:${additionalUnsettledSession}:${execution.operation_id}:${fence}`);
    assert(blocked);assert.equal(blocked.kind,'runtime/execution-recovery-blocked');assert.equal(blocked.producer_id,'runtime_supervisor');assert.equal(blocked.correlation_id,execution.operation_id);
    const diagnostic=value(blocked);assert.equal(diagnostic.code,'NATIVE_RECOVERY_RECONCILIATION_REQUIRED');assert.equal(diagnostic.execution_fence,fence);
    assert.equal(diagnostic.quarantined_effects,1);assert.equal(diagnostic.automatic_replay_allowed,false);assert.equal(diagnostic.next_action,'owner_reconciliation_required');
    assert(proof.seq<blocked.seq);assert(additions.every(event=>event===proof||event===blocked||(event.kind==='turn/paused'&&event.producer_id==='runtime_supervisor'&&event.correlation_id===execution.operation_id&&value(event).pause?.reason==='NATIVE_RECOVERY_RECONCILIATION_REQUIRED'&&value(event).pause?.cleanup_proven===false)),'Only matching native restart quarantine facts may be appended');
    assert(['cancelled','paused'].includes(execution.state));assert.equal(execution.turn_state,before.execution.turn_state);
    assert(['ready','paused','reconciliation'].includes(observation.session.head.status));assert(!observation.session.head.active_turn_id||observation.session.head.active_turn_id===execution.operation_id);
    assert.equal(execution.automatic_replay_authorized,false);transition='native_restart_unknown_preserved';restartProof={effect_event_id:proof.event_id,diagnostic_event_id:blocked.event_id};
  }
  return{session_id:before.session_id,effect_id:effectId,input_digest:inputDigest,transition,pending_effects:execution.pending_effects,unknown_effects:execution.unknown_effects,
    original_336_events_unchanged:true,owner_binding_resources_snapshot_unchanged:true,...(restartProof?{native_restart_proof:restartProof}:{}),driver_mutating_requests:0,driver_original_calls_dispatched:0,driver_reconciliation_requests:0};
}
async function stableJson(path){
  assert(isAbsolute(path));const before=await lstat(path);assert(before.isFile()&&!before.isSymbolicLink()&&before.size<=8*1024*1024);
  const bytes=await readFile(path),after=await lstat(path);assert.equal(bytes.length,before.size);for(const key of ['ino','dev','size','mtimeMs'])assert.equal(after[key],before[key]);
  return{path:resolve(path),sha256:hash(bytes),value:JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(bytes))};
}
async function reader(fixture){
  assert.equal(fixture.schema,'nomifun.windows-global-os-fixture.v1');assert.equal(fixture.status,'ready');
  assert.match(basename(fixture.parent),/^nomifun-global-os-owner-[0-9a-f-]{36}$/);
  assert.equal(fixture.native_data_root,join(fixture.parent,'native-data'));assert.equal(fixture.token_file,join(fixture.parent,'owner-jwt'));
  for(const path of [fixture.parent,fixture.native_data_root])await assertWindowsPrivatePath(path);
  assert.equal(await readFile(join(fixture.parent,'.nomifun-global-os-fixture'),'utf8'),'nomifun-global-os-native-acceptance-fixture.v1\n');
  const tokenBytes=await readWindowsOwnedFixtureBytes(fixture.token_file,4096),token=tokenBytes.toString('utf8').trim();assert.equal(token.split('.').length,3);
  const url=new URL(fixture.base_url);assert.equal(url.protocol,'http:');assert.equal(url.hostname,'127.0.0.1');
  const get=async path=>{const response=await fetch(`${fixture.base_url}${path}`,{headers:{Authorization:`Bearer ${token}`},signal:AbortSignal.timeout(15000)});assert(response.ok);const body=await response.json();assert.notEqual(body.success,false);return body.data??body;};
  const caller=await get('/api/system/permissions');assert.equal(caller.platform,'windows');assert.equal(caller.process_id,fixture.app_pid);
  return{get,tokenSha256:hash(tokenBytes)};
}
export async function readWindowsAdditionalSessionFacts(get){
  const path=`/api/agent-sessions/${additionalUnsettledSession}`,session=await get(path),execution=await get(`${path}/execution`);
  const effects=await get(`${path}/execution/effects?operation_id=${encodeURIComponent(execution.operation_id)}`),events=[];let after=0;
  for(let page=0;page<20;page++){const result=await get(`${path}/events?after_seq=${after}&limit=500`);assert(Array.isArray(result.events));events.push(...result.events);if(result.events.length===0||result.next_cursor.seq<=after)break;after=result.next_cursor.seq;assert(page<19,'Additional Session exceeds bounded pagination');}
  const end=await get(path);assert.deepEqual(end.head,session.head);assert.deepEqual(end.session,session.session,'Additional Session changed during read-only acquisition');
  return{session,execution,effects,events};
}
export async function freezeWindowsAdditionalSession({fixtureReport,sessionId,sourceObservation,reportRoot}){
  assert.equal(sessionId,additionalUnsettledSession);const fixture=(await stableJson(fixtureReport)).value,source=await stableJson(sourceObservation);
  assert.equal(source.sha256,originalObservationSha);const {get,tokenSha256}=await reader(fixture),observation=await readWindowsAdditionalSessionFacts(get);
  const frozen=freezeWindowsAdditionalSessionFacts({fixture,tokenSha256,observation,originalObservation:source.value,sourceObservationPath:source.path,sourceObservationSha256:source.sha256});
  const path=join(resolve(reportRoot),`windows-additional-unsettled-preservation-${randomUUID()}.json`);await writeFile(path,JSON.stringify({...frozen,observed_at:new Date().toISOString()},null,2)+'\n',{flag:'wx'});
  console.log(JSON.stringify({report:path,session_id:sessionId,event_count:336,pending_effects:1,unknown_effects:0,mutating_requests:0}));return{...frozen,report_path:path};
}
export async function loadWindowsAdditionalSessionReport(path,sourceFixture){
  const frozen=await stableJson(path);assert.equal(frozen.value.app_pid,sourceFixture.app_pid);assert.equal(frozen.value.schema,'nomifun.windows-unsettled-upgrade-preservation.v1');
  assertWindowsAdditionalSessionPreservation({before:frozen.value,fixture:sourceFixture,tokenSha256:frozen.value.fixture.owner_jwt_sha256,
    observation:{session:{session:frozen.value.session,head:frozen.value.head},execution:frozen.value.execution,effects:frozen.value.effects,events:frozen.value.events}});
  return frozen;
}
export async function verifyWindowsAdditionalSessionReport(frozen,fixture){
  const current=await stableJson(frozen.path);assert.equal(current.sha256,frozen.sha256);const {get,tokenSha256}=await reader(fixture);
  return assertWindowsAdditionalSessionPreservation({before:frozen.value,fixture,tokenSha256,observation:await readWindowsAdditionalSessionFacts(get)});
}
