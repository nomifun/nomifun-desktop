#!/usr/bin/env node
// A new scoped cleanup Turn in the original failed Browser Session. This never
// repeats upload/navigation, settles unknown effects or edits canonical storage.
import assert from 'node:assert/strict';
import {createHash,randomUUID} from 'node:crypto';
import {createServer} from 'node:http';
import {readFile,writeFile,lstat,realpath} from 'node:fs/promises';
import {basename,dirname,join,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {assertWindowsFixture,readWindowsOwnedFixtureBytes,windowsLoginIdentity,powershell,psArguments,psLiteral} from './global-os-windows-cases.mjs';
import {nativeCanonicalPathReconsideration,nativeCompletionArguments,nativeCompletionContract,nativeContinuationSettlement} from './global-os-native.mjs';
const wait=ms=>new Promise(done=>setTimeout(done,ms));
const stable=value=>JSON.stringify(value,(_,entry)=>entry&&typeof entry==='object'&&!Array.isArray(entry)?Object.fromEntries(Object.keys(entry).sort().map(key=>[key,entry[key]])):entry);
const digest=value=>createHash('sha256').update(stable(value)).digest('hex');
export function windowsBrowserCleanupSource(source){
  assert.equal(source.schema,'nomifun.global-os-native-regression.v2');assert.equal(source.platform,'win32');assert.equal(source.status,'failed');assert.equal(source.scenario,'browser');assert.equal(source.administrator,false);
  assert.match(basename(source.fixture),/^nomifun-global-os-product-[0-9a-f-]{36}$/);assert.equal(source.workspace,join(source.fixture,'workspace'));
  const root=source.native_case_resources?.root;assert.equal(dirname(root),source.fixture);assert.match(basename(root),/^browser-cases-[0-9a-f-]{36}$/);
  assert.equal(source.cleanup_intent.path,root);assert.equal(source.scenario_host_resources_closed,true);
  const prior=source.recovery_failure_settlement??source.failure_settlement;assert.equal(prior.cleanup_proven,true);
  nativeContinuationSettlement({execution:prior.execution,head:prior.head,operationId:prior.execution.operation_id,sessionId:source.session_id});
  const nonce=basename(root).slice('browser-cases-'.length),disposed=Object.entries(source.observations).findLast(([id,value])=>(id.includes('browser-recovery-disposed-')||id.includes('browser-confirm-disposed'))&&value.content?.includes(`Disposable upload document released ${nonce}`));
  assert(disposed&&disposed[1].target.document_generation>0,'The original File page must already be replaced by its native disposal document');
  return{root,prior,disposed_target:disposed[1].target};
}
export async function cleanupWindowsBrowserFixture({fixtureReport,sourceReport,retryReport}){
  assert.equal(process.platform,'win32');const login=await windowsLoginIdentity();
  const ready=JSON.parse(await readFile(fixtureReport,'utf8')),sourceBytes=await readFile(sourceReport),source=JSON.parse(sourceBytes),pin=windowsBrowserCleanupSource(source);
  assert.equal(ready.status,'ready');assert.equal(source.host,ready.base_url);assert.equal(login.sid,ready.login_identity.sid);await assertWindowsFixture(source.fixture,source.workspace);
  const port=JSON.parse(await readFile(join(ready.native_data_root,'port.json'),'utf8'));assert.equal(port.pid,ready.app_pid);assert.equal(new URL(ready.base_url).port,String(port.port));
  const token=(await readWindowsOwnedFixtureBytes(ready.token_file,4096)).toString('utf8').trim(),claims=JSON.parse(Buffer.from(token.split('.')[1],'base64url'));
  const api=async(method,path,body)=>{const response=await fetch(new URL(path,ready.base_url),{method,headers:{authorization:`Bearer ${token}`,'content-type':'application/json'},body:body===undefined?undefined:JSON.stringify(body),signal:AbortSignal.timeout(30000)});const value=await response.json();assert(response.ok&&value.success!==false);return value.data??value;};
  const allEvents=async()=>{const events=[];let after=0;for(let page=0;page<20;page++){const value=await api('GET',`/api/agent-sessions/${source.session_id}/events?after_seq=${after}&limit=500`);events.push(...value.events);if(!value.events.length||value.next_cursor.seq<=after)return events;after=value.next_cursor.seq;}throw new Error('Bounded canonical pagination exceeded');};
  const before=await api('GET',`/api/agent-sessions/${source.session_id}`);assert.deepEqual(before.session.owner_ref,{principal_kind:'user',principal_id:claims.user_id});
  const originalEvents=(await allEvents()).filter(event=>event.seq<=before.head.last_seq),originalDigest=digest(originalEvents);
  let priorOperation=pin.prior.execution.operation_id,retry;
  if(retryReport){retry=JSON.parse(await readFile(retryReport,'utf8'));assert.equal(retry.schema,'nomifun.windows-browser-scoped-cleanup.v1');assert.equal(retry.status,'failed');assert.equal(retry.source_report_sha256,createHash('sha256').update(sourceBytes).digest('hex'));assert.equal(retry.original_session_id,source.session_id);assert.equal(retry.root,pin.root);priorOperation=retry.accepted.operation_id;
    const failed=originalEvents.filter(event=>event.correlation_id===priorOperation);assert(!failed.some(event=>event.kind.startsWith('effect/')));assert(!failed.some(event=>event.payload?.value?.event?.event==='tool_call_completed'&&event.payload.value.event.step>0),'A dispatched proposal cannot use the provider-setup retry');
  }
  nativeContinuationSettlement({execution:await api('GET',`/api/agent-sessions/${source.session_id}/execution`),head:before.head,operationId:priorOperation,sessionId:source.session_id});
  const link=join(pin.root,'linked-directory'),sources=join(pin.root,'sources');assert((await lstat(link)).isSymbolicLink());assert.equal((await realpath(link)).toLowerCase(),(await realpath(sources)).toLowerCase());
  const sourceSnapshot=async()=>Promise.all(source.native_case_resources.source_paths.map(async path=>{assert.equal(dirname(path),sources);try{const item=await lstat(path);assert(item.isFile()&&!item.isSymbolicLink()&&item.size<=8192&&item.nlink===1);return{path,size:item.size,mtime:item.mtimeMs,sha256:createHash('sha256').update(await readFile(path)).digest('hex')};}catch(error){if(error.code==='ENOENT')return{path,absent:true};throw error;}}));
  const sourceBefore=await sourceSnapshot();
  const run=randomUUID(),proposals=new Map(),consumed=new Set(),observed=[],checks=[];let accepted,failure,linkRemoved=false,deleted=false,attempt=0,completionCount=0,completionMode,completed=false;
  const summary=`原Browser上传验收失败保持失败。本次同Session新Turn仅通过正式exec非递归删除唯一夹具junction ${link} 并核对sources完全保留，再delete_path删除唯一测试目录 ${pin.root} 并核对不存在；不重做上传、导航、Computer输入或未知效果对账。这不证明filetool的junction unlink预检已修复。`;
  const server=createServer(async(request,response)=>{try{
    assert.equal(request.method,'POST');assert.equal(request.url,'/v1/chat/completions');const chunks=[];for await(const chunk of request)chunks.push(chunk);const body=JSON.parse(Buffer.concat(chunks));
    for(const message of body.messages.filter(message=>message.role==='tool')){const proposal=proposals.get(message.tool_call_id);if(!proposal||consumed.has(message.tool_call_id))continue;const raw=typeof message.content==='string'?message.content:JSON.stringify(message.content);
      if(proposal.name==='exec_command'){
        assert(!raw.startsWith('[tool error]'));const value=JSON.parse(raw);assert.equal(value.state,'exited');assert.equal(value.exit_code,0);assert.equal(value.cleanup.reaped,true);assert.equal(value.output.text.trim(),'JUNCTION_REMOVED');await assert.rejects(lstat(link),error=>error.code==='ENOENT');assert.deepEqual(await sourceSnapshot(),sourceBefore);linkRemoved=true;observed.push(message.tool_call_id);checks.push({call_id:message.tool_call_id,kind:'nonrecursive_fixture_junction_removed',source_snapshot_unchanged:true});
      }else if(proposal.name==='delete_path'){
        const redirect=nativeCanonicalPathReconsideration({raw,callId:message.tool_call_id,callArguments:proposal.arguments,events:await allEvents(),operationId:accepted.operation_id,sessionId:source.session_id});
        if(redirect){assert([pin.root,`\\\\?\\${pin.root}`].includes(redirect.canonical));proposal.redirect=redirect.canonical;checks.push({call_id:message.tool_call_id,kind:'canonical_pre_dispatch_path_notice',canonical:redirect.canonical});}
        else{assert(!raw.startsWith('[tool error]'));JSON.parse(raw);await assert.rejects(lstat(pin.root),error=>error.code==='ENOENT');deleted=true;observed.push(message.tool_call_id);checks.push({call_id:message.tool_call_id,kind:'canonical_delete_and_native_absence'});}
      }else if(proposal.name==='ToolSearch'){assert(!raw.startsWith('[tool error]'));assert(JSON.parse(raw).some(tool=>tool.name===proposal.arguments.query));}
      else if(proposal.name==='report_completion'){assert(raw.startsWith('Completion account recorded,')||raw.startsWith('Candidate account recorded,'));completionCount++;}
      consumed.add(message.tool_call_id);
    }
    let name,args,text;
    if(!deleted){assert(attempt++<8,'Bounded cleanup proposals exhausted');const required=linkRemoved?'delete_path':'exec_command';if(body.tools.some(tool=>tool.function?.name===required)){name=required;args=linkRemoved?{path:[...proposals.values()].findLast(p=>p.redirect)?.redirect??pin.root}:{command:powershell,args:psArguments(`$p=${psLiteral(link)};if(-not([IO.File]::GetAttributes($p)-band[IO.FileAttributes]::ReparsePoint)){throw 'Expected only the exact fixture junction'};[IO.Directory]::Delete($p);[Console]::WriteLine('JUNCTION_REMOVED')`),timeout_ms:20000};}else{name='ToolSearch';args={query:required};}}
    else{completionMode=nativeCompletionContract({tools:body.tools,toolChoice:body.tool_choice});if(completionMode==='report_completion'){assert(completionCount<4);name='report_completion';args=nativeCompletionArguments({tool:body.tools.find(tool=>tool.function?.name===name),observedCallIds:observed,summary});}else{text=summary;completed=true;}}
    const id=`native-cleanup-${run}-${proposals.size+1}`;if(name)proposals.set(id,{name,arguments:args});response.setHeader('content-type','application/json');response.end(JSON.stringify({id:`fixture-${randomUUID()}`,object:'chat.completion',created:Math.floor(Date.now()/1000),model:'global-os-cleanup',choices:[{index:0,message:name?{role:'assistant',content:null,tool_calls:[{id,type:'function',function:{name,arguments:JSON.stringify(args)}}]}:{role:'assistant',content:text},finish_reason:name?'tool_calls':'stop'}],usage:{prompt_tokens:1,completion_tokens:1,total_tokens:2}}));
  }catch(error){failure=error;response.statusCode=500;response.end(JSON.stringify({error:{message:'Scoped cleanup driver rejected a result'}}));}});
  await new Promise(done=>server.listen(0,'127.0.0.1',done));const report={schema:'nomifun.windows-browser-scoped-cleanup.v1',source_report:resolve(sourceReport),source_report_sha256:createHash('sha256').update(sourceBytes).digest('hex'),original_session_id:source.session_id,original_canonical_prefix_sha256:originalDigest,root:pin.root,status:'running',checks,started_at:new Date().toISOString()};
  try{
    const provider=await api('POST','/api/providers',{platform:'custom',name:`Scoped failed Browser cleanup ${run}`,base_url:`http://127.0.0.1:${server.address().port}/v1`,auth_scheme:'bearer',credentials:{api_keys:['local-fixture-only']},enabled:true,initial_model:{model:'global-os-cleanup',enabled:true,capabilities:[{task:'chat',traits:[],protocol:'openai.chat_text',connection_role:'default',output_limit:4096}]}});
    await api('PUT',`/api/agent-sessions/${source.session_id}/model`,{provider_id:provider.provider_id,model:'global-os-cleanup'});
    accepted=await api('POST',`/api/agent-sessions/${source.session_id}/turns`,{idempotency_key:run,input:{content:summary.replace('并核对不存在','并真实核对不存在')}});report.accepted=accepted;
    const until=Date.now()+120000;let execution;
    while(Date.now()<until){if(failure)throw failure;execution=await api('GET',`/api/agent-sessions/${source.session_id}/execution`);assert.equal(execution.operation_id,accepted.operation_id);if(execution.state==='completed')break;assert.equal(execution.state,'running');await wait(200);}
    assert.equal(execution.state,'completed');assert(deleted);
    const after=await api('GET',`/api/agent-sessions/${source.session_id}`);assert(!after.head.active_turn_id);assert.equal(execution.unknown_effects,0);assert.equal(execution.pending_effects,0);
    const events=await allEvents();assert.equal(digest(events.filter(event=>event.seq<=before.head.last_seq)),originalDigest,'Original failure facts changed');const own=events.filter(event=>event.correlation_id===accepted.operation_id);assert(own.some(event=>event.payload?.value?.event?.event==='host_cleanup_proven'));
    if(completionMode==='report_completion'){const completion=own.findLast(event=>event.payload?.value?.event?.event==='completion_reported');assert(completion);assert(completion.payload.value.event.report.criteria.every(criterion=>criterion.disposition==='supported'));assert(own.some(event=>event.payload?.value?.event?.event==='completion_delivered'));}else assert(completed);
    let outcome;for(let page=1;page<=20;page++){const history=await api('GET',`/api/agent-sessions/${source.session_id}/message-history?page=${page}&page_size=100`);const item=history.items.find(item=>item.content?.turn_summary===true&&item.content.turn_id===accepted.message_id);if(item){assert.equal(item.content.turn_state,'completed');assert.equal(item.content.turn_outcome.kind,'answered');outcome=item.content.turn_outcome;break;}if(!history.has_more)break;}assert(outcome,'The cleanup needs its exact canonical answered Turn outcome');
    assert.equal(createHash('sha256').update(await readFile(sourceReport)).digest('hex'),report.source_report_sha256,'Original failed report changed');Object.assign(report,{status:'passed',execution,outcome,settlement_events:own,original_failure_events_unchanged:true,original_failed_report_unchanged:true,original_upload_replayed:false,original_unknown_reconciled:false});
  }catch(error){Object.assign(report,{status:'failed',failure:{name:error.name,message:error.message}});if(accepted){report.cancel_request=await api('POST',`/api/agent-sessions/${source.session_id}/turns/cancel`,{idempotency_key:randomUUID(),expected_turn_id:accepted.message_id}).catch(()=>({unproven:true}));}throw error;}
  finally{server.closeAllConnections();await new Promise(done=>server.close(done));report.finished_at=new Date().toISOString();const path=join(ready.report_root,`browser-scoped-cleanup-${run}.json`);await writeFile(path,JSON.stringify(report,null,2)+'\n',{flag:'wx'});console.log(JSON.stringify({status:report.status,report:path,session_id:source.session_id}));}
  return report;
}
if(process.argv[1]===fileURLToPath(import.meta.url)){const flag=name=>{const index=process.argv.indexOf(name);assert(index>=0&&process.argv[index+1]);return process.argv[index+1];};await cleanupWindowsBrowserFixture({fixtureReport:flag('--fixture-report'),sourceReport:flag('--source-report'),retryReport:process.argv.includes('--retry-report')?flag('--retry-report'):undefined});}
