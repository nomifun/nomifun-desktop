#!/usr/bin/env node
// Finish only the three cleanup steps of the exact completed control process.
// No probe replay, original-UNC mutation or Effect reconciliation is permitted.
import assert from 'node:assert/strict';
import {nativeProcessStep} from './global-os-process-step.mjs';
import {createHash,randomUUID} from 'node:crypto';
import {createServer} from 'node:http';
import {lstat,readFile,writeFile} from 'node:fs/promises';
import {join,resolve,win32} from 'node:path';
import {fileURLToPath} from 'node:url';
import {assertWindowsFixture,readWindowsOwnedFixtureBytes,windowsLoginIdentity} from './global-os-windows-cases.mjs';
import {windowsSmbControlPlan,validateSmbDispositionControlReceipt} from './global-os-windows-smb-control-cases.mjs';
import {readWindowsAdditionalSessionFacts} from './global-os-windows-unsettled-preservation.mjs';
import {nativeCanonicalPathReconsideration,nativeCompletionArguments,nativeCompletionContract,nativeCompletionPlanControl,nativeContinuationSettlement} from './global-os-native.mjs';
const sourceSha='873d2a666d0bc516c8f02aa6901b2159eefbc8eb2b4c804754bcacea58b47935';
const receiptSha='d2d34c187f39688beb4266554d4bad9af750d301e5cf7c9e1da514d1cddd42d0';
const sessionId='01a148a8-9fbc-7d02-801c-5274e2227521',callId='native-61cc8498-0f70-42f2-87a3-aa33772503bb-smb-control-disposition-probe';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const stable=value=>JSON.stringify(value,(_,entry)=>entry&&typeof entry==='object'&&!Array.isArray(entry)?Object.fromEntries(Object.keys(entry).sort().map(key=>[key,entry[key]])):entry);
const wait=ms=>new Promise(done=>setTimeout(done,ms));
const same=(a,b)=>a.replace(/^\\\\\?\\/,'').toLowerCase()===b.replace(/^\\\\\?\\/,'').toLowerCase();
export function windowsSmbControlCleanupSource(source,receipt,{sourceSha256,receiptSha256}){
  assert.equal(sourceSha256,sourceSha);assert.equal(receiptSha256,receiptSha);
  assert.equal(source.schema,'nomifun.global-os-native-regression.v2');assert.equal(source.platform,'win32');assert.equal(source.status,'failed');assert.equal(source.scenario,'smb-disposition-control');assert.equal(source.administrator,false);
  assert.equal(source.session_id,sessionId);assert.equal(source.accepted.agent_session_id,sessionId);assert.equal(source.accepted.replayed,false);
  assert.equal(source.workspace,win32.join(source.fixture,'workspace'));assert.equal(source.fixture_directories_retained,true);
  const resources=source.native_case_resources,nonce=win32.basename(resources.root).slice('smb-disposition-cases-'.length);
  const plan=windowsSmbControlPlan({probeInput:{nonce,physicalRoot:resources.physical_root,shareName:resources.share_name,preservedPhysicalRoot:resources.preserved_physical_root,preservedShareName:resources.preserved_share_name},workspace:source.workspace,loginIdentity:source.login_identity});
  assert.equal(plan.root,resources.root);assert.equal(plan.marker,resources.marker);assert.equal(plan.probe_script_sha256,resources.probe_script_sha256);assert.equal(win32.dirname(plan.root),source.fixture);
  assert.equal(resources.preserved_session_id,'01a14893-93f7-7a90-b573-bc0b31116207');assert.equal(resources.preserved_unc_report_sha256,'be97ccdfcde70b3ea454cd2fb29a682a0698dc089205f626946649a1a027ea11');
  for(const step of ['smb-control-marker','smb-control-create-share'])assert(source.checks.some(row=>row.step===step&&row.status==='passed'));
  assert(!source.checks.some(row=>['smb-control-remove-share','smb-control-delete-local-root','smb-control-marker-absence'].includes(row.step)&&row.status==='passed'));
  assert.equal(receipt.schema,'nomifun.windows-smb-control-canonical-payload-readonly.v1');assert.equal(receipt.source_sha256,sourceSha);assert.equal(receipt.session_id,sessionId);assert.equal(receipt.original_operation_id,source.accepted.operation_id);assert.equal(receipt.original_call_id,callId);assert.equal(receipt.event_seq,531);assert.equal(receipt.payload_identity_verified,true);assert.equal(receipt.automatic_replays,0);
  const reference=receipt.reference;assert.deepEqual(reference,{pointer:'/output/output',kind:'tool_output',payload_id:`shared-tool:${sessionId}:output:3247f9f0d1dae629b480c317d97b14c5c9c55acb4d1805dd869085a5786e2dfd`,digest:'3247f9f0d1dae629b480c317d97b14c5c9c55acb4d1805dd869085a5786e2dfd',byte_len:31535});
  const event=receipt.events.find(event=>event.seq===531);assert.equal(event.event_id,receipt.event_id);assert.equal(event.kind,'tool/result-recorded');assert.equal(event.agent_session_id,sessionId);
  const recorded=event.payload.value;assert.equal(recorded.call_id,callId);assert.equal(recorded.output.is_error,false);assert.equal(recorded.error,null);assert(recorded._canonical_shared_content.references.some(item=>stable(item)===stable(reference)));
  const protocol=validateSmbDispositionControlReceipt(receipt.canonical_payload_value,plan.target);assert.deepEqual(protocol,receipt.protocol);
  const objects=new Set();for(const row of protocol.cases){assert(row.created_identity&&row.delete_identity);assert.deepEqual(row.delete_identity,row.created_identity);if(row.metadata_holder)assert.deepEqual(row.holder_identity,row.created_identity);assert(row.steps.filter(step=>step.operation==='FileIdInfo').every(step=>step.success));const id=stable(row.created_identity);assert(!objects.has(id));objects.add(id);}
  nativeContinuationSettlement({execution:source.failure_settlement.execution,head:source.failure_settlement.head,operationId:source.accepted.operation_id,sessionId});assert.deepEqual(receipt.execution,source.failure_settlement.execution);
  assert.equal(receipt.events.length,651);assert.equal(source.failure_settlement.head.last_seq,651);receipt.events.forEach((event,index)=>{assert.equal(event.seq,index+1);assert.equal(event.agent_session_id,sessionId);});
  const steps=plan.steps.slice(3).map(step=>({...step,args:{...step.args}}));assert.deepEqual(steps.map(step=>step.name),['start_process','delete_path','read_file']);
  // All share checks/removal use absolute identities; use the frozen default
  // cwd instead of redispatching an explicit workspace spelling preflight.
  delete steps[0].args.cwd;
  return{...plan,steps,originalEvents:receipt.events,sourceOperation:source.accepted.operation_id};
}
async function stableReport(path){const before=await lstat(path);assert(before.isFile()&&!before.isSymbolicLink()&&before.size<=16*1024*1024);const bytes=await readFile(path),after=await lstat(path);for(const key of ['ino','dev','size','mtimeMs'])assert.equal(after[key],before[key]);assert.equal(bytes.length,before.size);return{bytes,value:JSON.parse(bytes),sha256:hash(bytes)};}
export async function cleanupWindowsSmbControl({fixtureReport,sourceReport,canonicalReceipt}){
  assert.equal(process.platform,'win32');const ready=(await stableReport(fixtureReport)).value,source=await stableReport(sourceReport),receipt=await stableReport(canonicalReceipt),plan=windowsSmbControlCleanupSource(source.value,receipt.value,{sourceSha256:source.sha256,receiptSha256:receipt.sha256});
  plan.steps=plan.steps.map(nativeProcessStep);
  const login=await windowsLoginIdentity();assert.equal(login.sid,ready.login_identity.sid);assert.equal(login.sid,source.value.login_identity.sid);assert.equal(ready.status,'ready');assert.equal(ready.base_url,source.value.host);assert.equal(ready.native_data_root,source.value.native_dataset.root);await assertWindowsFixture(source.value.fixture,source.value.workspace);
  const announcement=JSON.parse((await readWindowsOwnedFixtureBytes(join(ready.native_data_root,'port.json'),4096)).toString('utf8'));assert.equal(announcement.pid,ready.app_pid);assert.equal(String(announcement.port),new URL(ready.base_url).port);
  assert.equal(await readFile(plan.marker,'utf8'),plan.target.markerText);
  const token=(await readWindowsOwnedFixtureBytes(ready.token_file,4096)).toString('utf8').trim(),claims=JSON.parse(Buffer.from(token.split('.')[1],'base64url'));
  const api=async(method,path,body)=>{const response=await fetch(new URL(path,ready.base_url),{method,headers:{authorization:`Bearer ${token}`,'content-type':'application/json'},body:body===undefined?undefined:JSON.stringify(body),signal:AbortSignal.timeout(30000)}),value=await response.json();assert(response.ok&&value.success!==false,`${method} ${path} rejected`);return value.data??value;};
  const get=path=>api('GET',path),allEvents=async()=>{const events=[];let after=0;for(let page=0;page<20;page++){const result=await get(`/api/agent-sessions/${sessionId}/events?after_seq=${after}&limit=500`);events.push(...result.events);if(!result.events.length||result.next_cursor.seq<=after)return events;after=result.next_cursor.seq;}throw Error('Bounded control history exceeded');};
  const caller=await get('/api/system/permissions');assert.equal(caller.platform,'windows');assert.equal(caller.process_id,ready.app_pid);
  const before=await get(`/api/agent-sessions/${sessionId}`);assert.deepEqual(before.session.owner_ref,{principal_kind:'user',principal_id:claims.user_id});nativeContinuationSettlement({execution:await get(`/api/agent-sessions/${sessionId}/execution`),head:before.head,operationId:plan.sourceOperation,sessionId});
  assert.deepEqual(await allEvents(),plan.originalEvents);const originalPrefix=hash(stable(plan.originalEvents)),uncBefore=await readWindowsAdditionalSessionFacts(get);assert.equal(uncBefore.events.length,336);assert.equal(uncBefore.execution.pending_effects,1);assert.equal(uncBefore.execution.unknown_effects,0);
  const run=randomUUID(),checks=[],proposals=new Map(),consumed=new Set(),observed=[];let accepted,failure,index=0,sequence=0,completionMode,completionCount=0,planControls=0;
  const summary='独立SMB control的16次原生实验已在原Turn完成；原driver因模型上下文excerpt JSON解析失败仍保持失败。本次同Session新Turn仅清理本control唯一share和localroot，并用正式read_file取得marker typedabsence。未再次运行probe、未修改或对账原34d UNC pending效果；本次完成仅支持control清理状态，不代表产品UNC patch或旧效果成功。';
  const server=createServer(async(request,response)=>{try{
    assert.equal(request.method,'POST');assert.equal(request.url,'/v1/chat/completions');const chunks=[];for await(const chunk of request)chunks.push(chunk);const body=JSON.parse(Buffer.concat(chunks));
    for(const message of body.messages.filter(message=>message.role==='tool')){const proposal=proposals.get(message.tool_call_id);if(!proposal||consumed.has(message.tool_call_id))continue;const raw=typeof message.content==='string'?message.content:JSON.stringify(message.content);
      if(proposal.name==='ToolSearch'){assert(!raw.startsWith('[tool error]'));assert(JSON.parse(raw).some(tool=>tool.name===proposal.args.query));}
      else if(proposal.name==='update_plan'){const result=JSON.parse(raw);assert.equal(result.needs_replan,false);assert(result.plan.every(step=>step.status==='completed'));}
      else if(proposal.name==='report_completion'){assert(raw.startsWith('Completion account recorded,')||raw.startsWith('Candidate account recorded,'));completionCount++;}
      else{const step=plan.steps[index];assert.equal(proposal.index,index);const redirect=!['start_process','poll_process'].includes(proposal.name)?nativeCanonicalPathReconsideration({raw,callId:message.tool_call_id,callArguments:proposal.args,events:await allEvents(),operationId:accepted.operation_id,sessionId}):undefined;
        if(redirect){assert(same(redirect.canonical,step.args.path));step.args.path=redirect.canonical;checks.push({kind:'canonical_pre_dispatch_path_notice',call_id:message.tool_call_id});}
        else{assert(!raw.startsWith('[tool error]'),'Unknown or rejected cleanup is not retried');let result=JSON.parse(raw);if(index===0){if(await step.verify(result,{callId:message.tool_call_id})===false){consumed.add(message.tool_call_id);continue;}result=step.processObservation();}
          if(index===0){assert.equal(result.state,'exited');assert.equal(result.exit_code,0);assert.equal(result.cleanup.reaped,true);const native=JSON.parse(result.output.text);assert.equal(native.name,plan.target.shareName);assert.equal(native.exists,false);assert(same(native.expected_path,plan.target.physicalRoot));}
          else if(index===1){await assert.rejects(lstat(plan.root),error=>error.code==='ENOENT');}
          else{assert.equal(result.kind,'workspace_file_absent');assert(same(result.path,plan.marker));await assert.rejects(lstat(plan.root),error=>error.code==='ENOENT');}
          observed.push(message.tool_call_id);checks.push({step:step.id,call_id:message.tool_call_id,status:'passed'});index++;}
      }consumed.add(message.tool_call_id);
    }
    let name,args,text;
    if(index<plan.steps.length){const next=plan.steps[index];name=body.tools.some(tool=>tool.function?.name===next.name)?next.name:'ToolSearch';args=name==='ToolSearch'?{query:next.name}:typeof next.args==='function'?await next.args():next.args;}
    else{const control=nativeCompletionPlanControl({tools:body.tools,events:await allEvents(),operationId:accepted.operation_id,sessionId});if(control){assert(++planControls<=2);name='update_plan';args=control.arguments;}else{completionMode=nativeCompletionContract({tools:body.tools,toolChoice:body.tool_choice});if(completionMode==='report_completion'){assert(completionCount<4);name='report_completion';args=nativeCompletionArguments({tool:body.tools.find(tool=>tool.function?.name===name),observedCallIds:observed,summary});}else text=summary;}}
    const id=`smb-control-cleanup-${run}-${++sequence}`;assert(sequence<=40,'Bounded cleanup proposal budget exceeded');if(name)proposals.set(id,{name,args:structuredClone(args),index});response.setHeader('content-type','application/json');response.end(JSON.stringify({id:`fixture-${randomUUID()}`,object:'chat.completion',created:Math.floor(Date.now()/1000),model:'global-os-smb-control-cleanup',choices:[{index:0,message:name?{role:'assistant',content:null,tool_calls:[{id,type:'function',function:{name,arguments:JSON.stringify(args)}}]}:{role:'assistant',content:text},finish_reason:name?'tool_calls':'stop'}],usage:{prompt_tokens:1,completion_tokens:1,total_tokens:2}}));
  }catch(error){failure=error;response.statusCode=500;response.end(JSON.stringify({error:{message:'Scoped control cleanup rejected a result'}}));}});
  await new Promise(done=>server.listen(0,'127.0.0.1',done));const report={schema:'nomifun.windows-smb-control-scoped-cleanup.v1',status:'running',session_id:sessionId,source_report:resolve(sourceReport),source_report_sha256:source.sha256,canonical_receipt:resolve(canonicalReceipt),canonical_receipt_sha256:receipt.sha256,original_651_event_prefix_sha256:originalPrefix,preserved_unc_336_prefix_sha256:hash(stable(uncBefore.events)),resources:source.value.native_case_resources,probe_replays:0,source_effect_reconciliations:0,checks,started_at:new Date().toISOString()};
  try{
    const provider=await api('POST','/api/providers',{platform:'custom',name:`Scoped SMB control cleanup ${run}`,base_url:`http://127.0.0.1:${server.address().port}/v1`,auth_scheme:'bearer',credentials:{api_keys:['local-fixture-only']},enabled:true,initial_model:{model:'global-os-smb-control-cleanup',enabled:true,capabilities:[{task:'chat',traits:[],protocol:'openai.chat_text',connection_role:'default',output_limit:4096}]}});await api('PUT',`/api/agent-sessions/${sessionId}/model`,{provider_id:provider.provider_id,model:'global-os-smb-control-cleanup'});
    accepted=await api('POST',`/api/agent-sessions/${sessionId}/turns`,{idempotency_key:run,input:{content:summary}});report.accepted=accepted;
    const until=Date.now()+180000;let execution;while(Date.now()<until){if(failure)throw failure;execution=await get(`/api/agent-sessions/${sessionId}/execution`);assert.equal(execution.operation_id,accepted.operation_id);if(execution.state==='completed')break;assert.equal(execution.state,'running');await wait(200);}assert.equal(execution.state,'completed');assert.equal(index,3);assert.equal(execution.pending_effects,0);assert.equal(execution.unknown_effects,0);
    const events=await allEvents(),own=events.filter(event=>event.correlation_id===accepted.operation_id);assert(own.some(event=>event.payload?.value?.event?.event==='host_cleanup_proven'));assert.equal(hash(stable(events.filter(event=>event.seq<=651))),originalPrefix);assert.deepEqual((await get(`/api/agent-sessions/${sessionId}`)).session.agent_binding.typed_resource_bindings,before.session.agent_binding.typed_resource_bindings);
    assert.deepEqual(await readWindowsAdditionalSessionFacts(get),uncBefore,'The original 34d Session/Event/Payload/Effect/execution facts must remain unchanged');assert.equal(hash(await readFile(sourceReport)),source.sha256);assert.equal(hash(await readFile(canonicalReceipt)),receipt.sha256);
    if(completionMode==='report_completion')assert(own.some(event=>event.payload?.value?.event?.event==='completion_delivered'));
    let outcome;for(let page=1;page<=20;page++){const history=await get(`/api/agent-sessions/${sessionId}/message-history?page=${page}&page_size=100`),item=history.items.find(item=>item.content?.turn_summary===true&&item.content.turn_id===accepted.message_id);if(item){assert.equal(item.content.turn_outcome.kind,'answered');outcome=item.content.turn_outcome;break;}if(!history.has_more)break;}assert(outcome);
    Object.assign(report,{status:'passed',execution,outcome,settlement_events:own,canonical_host_cleanup_proven:true,original_failed_report_unchanged:true,original_control_events_unchanged:true,original_unc_pending_facts_unchanged:true});
  }catch(error){report.status='failed';report.failure={name:error.name,message:error.message};if(accepted)report.cancel_request=await api('POST',`/api/agent-sessions/${sessionId}/turns/cancel`,{idempotency_key:randomUUID(),expected_turn_id:accepted.message_id}).catch(()=>({unproven:true}));throw error;}
  finally{server.closeAllConnections();await new Promise(done=>server.close(done));report.finished_at=new Date().toISOString();const path=join(ready.report_root,`smb-control-scoped-cleanup-${run}.json`);await writeFile(path,JSON.stringify(report,null,2)+'\n',{flag:'wx'});console.log(JSON.stringify({status:report.status,report:path,session_id:sessionId}));}return report;
}
if(process.argv[1]===fileURLToPath(import.meta.url)){const flag=name=>{const index=process.argv.indexOf(name);assert(index>=0&&process.argv[index+1]);return process.argv[index+1];};await cleanupWindowsSmbControl({fixtureReport:flag('--fixture-report'),sourceReport:flag('--source-report'),canonicalReceipt:flag('--canonical-receipt')});}
