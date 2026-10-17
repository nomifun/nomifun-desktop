#!/usr/bin/env node
// Launch one explicitly supplied native executable with a fresh private dataset.
// Owner setup/login runs in that actual WebView2 via normal public auth routes.
// A debug CDP port belongs only to this disposable fixture, never a daily App.
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { randomBytes, randomUUID, createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { open, mkdir, readFile, realpath, writeFile, lstat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { allocateLoopbackPort, assertWindowsPrivatePath, powershell, psArguments, psLiteral, readWindowsOwnedFixtureBytes, windowsLoginIdentity, windowsProcessGone, windowsProcessObservation,windowsObservation } from './global-os-windows-cases.mjs';
import { windowsIntegrityObservationScript } from './global-os-windows-desktop-cases.mjs';
import {freezeWindowsAdditionalSession,loadWindowsAdditionalSessionReport,verifyWindowsAdditionalSessionReport} from './global-os-windows-unsettled-preservation.mjs';
import {captureWindowsDailyUriSnapshot,windowsDailyUriLaunchObservation,windowsDailyUriCloseObservation} from './global-os-windows-uri-baseline.mjs';
import {beginWindowsProcessExitObservation,windowsCloseProcessIdentity} from './global-os-windows-process-exit-observer.mjs';

const delay=ms=>new Promise(done=>setTimeout(done,ms));
const flag=name=>{const i=process.argv.indexOf(name);return i<0?undefined:process.argv[i+1];};
async function executableDigest(path) {const hash=createHash('sha256');for await(const bytes of createReadStream(path))hash.update(bytes);return hash.digest('hex');}
const canonicalDigest=value=>createHash('sha256').update(JSON.stringify(value,(_,entry)=>entry&&typeof entry==='object'&&!Array.isArray(entry)?Object.fromEntries(Object.keys(entry).sort().map(key=>[key,entry[key]])):entry)).digest('hex');
async function actualAppIdentity(pid,login,exe){
  const integrity=await windowsObservation(windowsIntegrityObservationScript(pid));assert.equal(integrity,8192,'The GUI must retain ordinary Medium integrity');
  const observed=await windowsObservation(`$p=Get-CimInstance Win32_Process -Filter 'ProcessId=${pid}';if($null-eq$p){throw 'Exact App process is absent'};$o=Invoke-CimMethod -InputObject $p -MethodName GetOwnerSid;[pscustomobject]@{pid=$p.ProcessId;session_id=$p.SessionId;sid=$o.Sid;owner_query=$o.ReturnValue}|ConvertTo-Json -Compress`);
  assert.equal(observed.pid,pid);assert.equal(observed.session_id,login.session_id);assert.equal(observed.sid,login.sid);assert.equal(observed.owner_query,0);const native=windowsProcessObservation(pid,{includeImagePath:true});assert.equal(native.state,'running');assert.equal((await realpath(native.image_path)).toLowerCase(),exe.toLowerCase(),'The actual running App image must be the exact immutable acceptance executable');return{...observed,integrity_rid:integrity,native_process:native};
}
async function cdp(target, expression, {onDispatch,connectionTimeoutMs=30000}={}) {
  const socket=new WebSocket(target.webSocketDebuggerUrl);
  try {
    await new Promise((done,fail)=>{const timeout=setTimeout(()=>fail(new Error('Fixture CDP websocket readiness timed out')),connectionTimeoutMs);socket.addEventListener('open',()=>{clearTimeout(timeout);done();},{once:true});socket.addEventListener('error',()=>{clearTimeout(timeout);fail(new Error('Fixture CDP websocket failed'));},{once:true});});
    return await new Promise((done,fail)=>{
      const timeout=setTimeout(()=>fail(new Error('Fixture WebView2 operation timed out')),30000);
      socket.addEventListener('message',event=>{let message;try{message=JSON.parse(event.data);}catch{return;}
        if(message.id!==1)return;clearTimeout(timeout);if(message.error||message.result?.exceptionDetails)return fail(new Error('Fixture WebView2 operation failed; private response omitted'));
        done(message.result.result.value);
      });
      socket.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true}}));
      onDispatch?.();
    });
  } finally {socket.close();}
}
async function rendererIdentity(target) {
  const value=await cdp(target,"(async()=>{const until=Date.now()+20000;while(Date.now()<until){if(window.__TAURI_INTERNALS__&&document.getElementById('root')?.childElementCount>0){try{const version=await window.__TAURI_INTERNALS__.invoke('plugin:app|version');return {version,origin:location.origin,renderer_mounted:true,width:innerWidth,height:innerHeight}}catch{}}await new Promise(done=>setTimeout(done,100))}throw new Error('Actual product renderer/native IPC readiness was not proven')})()");
  assert.equal(value?.renderer_mounted,true,'The actual native product renderer must mount before acceptance');assert(typeof value.version==='string');
  assert(/^https?:\/\/(?:tauri\.localhost|localhost|127\.0\.0\.1)(?::\d+)?$/.test(value.origin),'Native product renderer origin was not proven');assert(value.width>=880&&value.height>=600,'Native acceptance requires the supported desktop viewport of at least 880x600');return value;
}
export function windowsRendererReadinessTransient(error){
  return ['ECONNREFUSED','ECONNRESET','UND_ERR_SOCKET'].includes(error?.cause?.code)
    ||(error?.name==='TypeError'&&error?.code==='ConnectionRefused')
    ||['TimeoutError','AbortError'].includes(error?.name);
}
async function waitForRendererTarget(cdpPort,deadline){
  while(Date.now()<deadline){
    try{const response=await fetch(`http://127.0.0.1:${cdpPort}/json/list`,{signal:AbortSignal.timeout(1500)});if(response.ok){const targets=await response.json(),target=targets.find(t=>t.type==='page'&&/^https?:\/\/(?:tauri\.localhost|localhost|127\.0\.0\.1)(?::\d+)?\//.test(t.url));if(target)return target;}}
    catch(error){if(!windowsRendererReadinessTransient(error))throw error;}
    await delay(250);
  }
  throw new Error('The exact acceptance App renderer did not become ready before its bounded deadline');
}
export async function launchWindowsAcceptanceFixture({exe,reportRoot,buildLabel='unverified-build'}) {
  assert.equal(process.platform,'win32');const identity=await windowsLoginIdentity();
  exe=await realpath(resolve(exe));const shape=await lstat(exe);assert(shape.isFile()&&!shape.isSymbolicLink());
  const parent=join(await realpath(tmpdir()),`nomifun-global-os-owner-${randomUUID()}`),data=join(parent,'native-data');
  await mkdir(parent);const current=psLiteral(identity.sid);
  await promisify(execFile)(powershell,psArguments(`$a=New-Object Security.AccessControl.DirectorySecurity;$a.SetOwner((New-Object Security.Principal.SecurityIdentifier(${current})));$a.SetAccessRuleProtection($true,$false);foreach($s in @(${current},'S-1-5-18','S-1-5-32-544')){$r=New-Object Security.AccessControl.FileSystemAccessRule((New-Object Security.Principal.SecurityIdentifier($s)),'FullControl','ContainerInherit,ObjectInherit','None','Allow');$a.AddAccessRule($r)};[IO.Directory]::SetAccessControl(${psLiteral(parent)},$a)`),{windowsHide:true,timeout:10000});
  await mkdir(data);await mkdir(reportRoot,{recursive:true});await writeFile(join(parent,'.nomifun-global-os-fixture'),'nomifun-global-os-native-acceptance-fixture.v1\n',{flag:'wx'});
  const uriBefore=await captureWindowsDailyUriSnapshot({parent,stage:'before'});
  const cdpPort=await allocateLoopbackPort(),stdout=await open(join(parent,'app.stdout.log'),'wx'),stderr=await open(join(parent,'app.stderr.log'),'wx');
  const child=spawn(exe,[],{cwd:dirname(exe),windowsHide:true,detached:true,stdio:['ignore',stdout.fd,stderr.fd],env:{...process.env,NOMIFUN_DATA_DIR:data,NOMIFUN_WORK_DIR:join(data,'work'),NOMIFUN_CACHE_DIR:join(data,'cache'),NOMIFUN_LOG_DIR:join(data,'logs'),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${cdpPort}`}});
  let spawnError;child.once('error',error=>{spawnError=error;});child.unref();await stdout.close();await stderr.close();
  const report={schema:'nomifun.windows-global-os-fixture.v1',status:'launching',build_label:buildLabel,exe,exe_sha256:await executableDigest(exe),native_data_root:data,parent,report_root:resolve(reportRoot),app_pid:child.pid,cdp_port:cdpPort,login_identity:identity,created_at:new Date().toISOString()};
  report.daily_uri_registration=windowsDailyUriLaunchObservation({kind:'fresh',before:uriBefore});
  const reportPath=join(reportRoot,`windows-fixture-${randomUUID()}.json`);
  try {
    let announcement,target;const deadline=Date.now()+180000;
    while(Date.now()<deadline){
      if(spawnError)throw spawnError;if(child.exitCode!==null)throw new Error(`Acceptance App exited before readiness (${child.exitCode})`);
      try{announcement=JSON.parse(await readFile(join(data,'port.json'),'utf8'));const response=await fetch(`http://127.0.0.1:${cdpPort}/json/list`,{signal:AbortSignal.timeout(1500)});const targets=await response.json();target=targets.find(t=>t.type==='page'&&/^https?:\/\/(?:tauri\.localhost|localhost|127\.0\.0\.1)(?::\d+)?\//.test(t.url));if(target&&announcement.pid===child.pid)break;}catch{}
      await delay(250);
    }
    assert(target&&announcement?.pid===child.pid,'Fresh native dataset/actual executable PID/WebView2 target readiness was not proven');
    report.renderer=await rendererIdentity(target);
    report.actual_app_identity=await actualAppIdentity(child.pid,identity,exe);assert.equal(await executableDigest(exe),report.exe_sha256,'The immutable acceptance executable changed during native startup');
    const baseUrl=`http://127.0.0.1:${announcement.port}`,password=`Gos-${randomBytes(32).toString('hex')}!`,username='global-os-owner';
    const login=await cdp(target,`(async()=>{const origin=${JSON.stringify(baseUrl)};const status=await (await fetch(origin+'/api/auth/status')).json();if(status.needs_setup!==true)throw new Error('Fresh fixture already has owner');const credentials=${JSON.stringify({username,password})};const setup=await fetch(origin+'/api/auth/setup',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(credentials)});if(!setup.ok)throw new Error('Normal owner setup rejected');const response=await fetch(origin+'/login',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(credentials)});if(!response.ok)throw new Error('Normal owner login rejected');const body=await response.json();return {token:body.token??body.data?.token};})()`);
    assert(typeof login?.token==='string'&&login.token.split('.').length===3,'Normal owner login returned no JWT');
    const tokenFile=join(parent,'owner-jwt');await writeFile(tokenFile,login.token,{flag:'wx'});
    Object.assign(report,{base_url:baseUrl,token_file:tokenFile,owner_setup:'normal_native_webview_route',owner_login:'normal_password_route',announcement});
    const callerResponse=await fetch(`${baseUrl}/api/system/permissions`,{headers:{Authorization:`Bearer ${login.token}`},signal:AbortSignal.timeout(15000)});
    assert(callerResponse.ok);const envelope=await callerResponse.json(),caller=envelope.data??envelope;assert.equal(caller.platform,'windows');
    if(caller.process_id!==undefined)assert.equal(caller.process_id,child.pid);
    Object.assign(report,{status:'ready',authenticated_native_caller:{platform:caller.platform,process_id:caller.process_id,process_identity_available:caller.process_id!==undefined}});
    report.daily_uri_registration=windowsDailyUriLaunchObservation({kind:'fresh',before:uriBefore,after:await captureWindowsDailyUriSnapshot({parent,stage:'after-ready'})});
    report.ready_verified_at=new Date().toISOString();
    return {...report,report_path:reportPath};
  } catch(error){Object.assign(report,{status:'failed',failure:{name:error.name,message:error.message}});throw error;}
  finally{await writeFile(reportPath,`${JSON.stringify(report,null,2)}\n`,{flag:'wx'});console.log(JSON.stringify({status:report.status,report:reportPath,native_data_root:data,app_pid:child.pid,base_url:report.base_url,token_file:report.token_file}));}
}
export async function closeWindowsAcceptanceFixture({fixtureReport}) {
  const report=JSON.parse(await readFile(fixtureReport,'utf8'));
  assert.equal(report.schema,'nomifun.windows-global-os-fixture.v1');assert.match(basename(report.parent),/^nomifun-global-os-owner-[0-9a-f-]{36}$/);
  assert.equal(report.native_data_root,join(report.parent,'native-data'));assert.equal(await realpath(report.parent),report.parent);
  assert.equal(await readFile(join(report.parent,'.nomifun-global-os-fixture'),'utf8'),'nomifun-global-os-native-acceptance-fixture.v1\n');
  const announcement=JSON.parse(await readFile(join(report.native_data_root,'port.json'),'utf8'));assert.equal(announcement.pid,report.app_pid);
  const close={schema:'nomifun.windows-global-os-fixture-close.v1',app_pid:report.app_pid,fixture_report:resolve(fixtureReport),requested_at:new Date().toISOString(),path:'existing_tauri_process_exit',request_dispatched:false,actual_pid_gone:false,process_exit_proven:false,exit_code:null,overall_cleanup_proven:false,cleanup_scope:'A retained process exit code does not prove every resource or snapshot was cleaned'};
  let observer;
  try{
    observer=await beginWindowsProcessExitObservation(windowsCloseProcessIdentity(report));
    close.retained_process_before_dispatch=observer.ready;
    const targets=await (await fetch(`http://127.0.0.1:${report.cdp_port}/json/list`,{signal:AbortSignal.timeout(5000)})).json();const target=targets.find(t=>t.type==='page'&&/^https?:\/\/(?:tauri\.localhost|localhost|127\.0\.0\.1)(?::\d+)?\//.test(t.url));assert(target,'Exact fixture WebView2 target is unavailable; no PID signal or global quit fallback');
    try {await cdp(target,"window.__TAURI_INTERNALS__.invoke('plugin:process|exit',{code:0})",{connectionTimeoutMs:5000,onDispatch:()=>{close.request_dispatched=true;close.dispatched_at=new Date().toISOString();}});}catch(error){close.invoke_disconnected=error.message;}
    close.native_process_exit=await observer.completion;
    close.actual_pid_gone=true;close.process_exit_proven=true;close.exit_code=close.native_process_exit.exit_code;
  }catch(error){
    close.process_exit_failure={name:error.name,message:error.message,native_observation:error.observation??null};
    if(observer&&!close.native_process_exit){try{close.native_process_exit=await observer.completion;close.actual_pid_gone=true;close.process_exit_proven=true;close.exit_code=close.native_process_exit.exit_code;}catch(failure){close.process_exit_failure.native_observation=failure.observation??close.process_exit_failure.native_observation;}}
  }
  close.status=close.process_exit_proven&&close.request_dispatched?'exited':'cleanup_not_proven';
  try{close.daily_uri_registration=await windowsDailyUriCloseObservation({parent:report.parent,launchObservation:report.daily_uri_registration,current:await captureWindowsDailyUriSnapshot({parent:report.parent,stage:'close-current'})});}
  catch{close.daily_uri_registration={status:'readonly_observation_unavailable',registry_restoration_performed:false,raw_values_recorded:false};}
  const path=join(report.report_root,`windows-fixture-close-${randomUUID()}.json`);await writeFile(path,`${JSON.stringify(close,null,2)}\n`,{flag:'wx'});console.log(JSON.stringify({status:close.status,report:path,app_pid:report.app_pid,exit_code:close.exit_code,process_exit_proven:close.process_exit_proven,overall_cleanup_proven:close.overall_cleanup_proven}));assert.equal(close.status,'exited','The coordinated request and exact retained App exit were not proven; retain the dataset and do not force terminate');return close;
}
export function assertWindowsUpgradePreservation({before,execution,events}) {
  assert(['nomifun.windows-desktop-unknown-observation.v1','nomifun.windows-desktop-upgrade-preservation.v2'].includes(before.schema));
  if(before.execution.unknown_effects===0){
    assert.equal(before.schema,'nomifun.windows-desktop-upgrade-preservation.v2');const proof=before.owner_reconciliation;assert(proof,'A cleared unknown requires its original canonical human owner reconciliation');assert.equal(proof.outcome,'confirmed_failed');assert.match(proof.human_verification_sha256,/^[a-f0-9]{64}$/);
    const attestation=proof.attestation_event,reconciled=proof.reconciled_event,value=attestation?.payload?.value;assert.equal(attestation?.kind,'runtime/effect-reconciliation-attested');assert.equal(value?.authority,'session_owner_verified');assert.equal(value?.outcome,'confirmed_failed');assert.equal(value?.effect_id,proof.effect_id);assert.equal(value?.input_digest,proof.input_digest);assert.equal(value?.evidence?.verified,true);assert.equal(value.evidence.evidence_digest,proof.human_verification_sha256);assert.equal(reconciled?.event_id,`${attestation.event_id}:effect`);assert.equal(reconciled?.kind,'effect/reconciled');assert.equal(reconciled?.payload?.value?.outcome,'confirmed_failed');assert.equal(reconciled.payload.value.effect_id,proof.effect_id);assert.equal(reconciled.payload.value.input_digest,proof.input_digest);
    const uncertain=before.effect_events.find(event=>event.kind==='effect/uncertain'&&event.payload?.value?.effect_id===proof.effect_id&&event.payload.value.outcome==='unknown');assert(uncertain);assert.equal(uncertain.payload.value.input_digest,proof.input_digest);assert(uncertain.seq<attestation.seq&&attestation.seq<reconciled.seq);
    assert.deepEqual(events.find(event=>event.event_id===attestation.event_id),attestation);assert.deepEqual(events.find(event=>event.event_id===reconciled.event_id),reconciled);
  }else assert(before.execution.unknown_effects>0,'Upgrade preservation requires the actual original uncertain outcome');
  assert.equal(execution.agent_session_id??before.session_id,before.session_id);
  assert.equal(execution.unknown_effects,before.execution.unknown_effects,'An App upgrade cannot erase or settle the original unknown');
  assert.equal(execution.pending_effects,before.execution.pending_effects);
  assert.equal(execution.operation_id,before.execution.operation_id);
  assert.equal(execution.state,before.execution.state);
  assert(before.effect_events.some(event=>event.kind==='effect/uncertain'&&event.payload?.value?.outcome==='unknown'));
  for(const original of before.effect_events){
    const current=events.find(event=>event.event_id===original.event_id&&event.seq===original.seq);
    assert(current,'An original canonical Effect event disappeared during upgrade');
    assert.deepEqual(Object.fromEntries(Object.keys(original).map(key=>[key,current[key]])),original,'An original canonical Effect fact changed during upgrade');
  }
  if(before.canonical_prefix){const prefix=events.filter(event=>event.seq<=before.canonical_prefix.last_seq);assert.equal(prefix.length,before.canonical_prefix.event_count);assert.equal(canonicalDigest(prefix),before.canonical_prefix.sha256,'Original Session Event/Payload-reference facts changed during upgrade');}
  if(before.original_unknown_prefix){const prefix=events.filter(event=>event.seq<=before.original_unknown_prefix.last_seq);assert.equal(prefix.length,before.original_unknown_prefix.event_count);assert.equal(canonicalDigest(prefix),before.original_unknown_prefix.sha256,'The original uncertain history must remain intact after owner reconciliation');}
  return{session_id:before.session_id,operation_id:execution.operation_id,unknown_effects:execution.unknown_effects,pending_effects:execution.pending_effects,original_effect_event_ids:before.effect_events.map(event=>event.event_id),all_original_effect_events_unchanged:true,driver_original_calls_dispatched:0,driver_reconciliation_requests:0};
}
export function windowsPreservationValidationEvents(preserved){
  const events=new Map(preserved.effect_events.map(event=>[event.event_id,event]));
  for(const event of [preserved.owner_reconciliation?.attestation_event,preserved.owner_reconciliation?.reconciled_event]){
    if(!event)continue;const compact=events.get(event.event_id);
    if(compact)assert.deepEqual(Object.fromEntries(Object.keys(compact).map(key=>[key,event[key]])),compact,'Compact Effect facts must exactly match their complete canonical owner event');
    events.set(event.event_id,event);
  }
  return[...events.values()];
}
export function assertWindowsDesktopPreservationSource({source,preserved,preservedBytes,preservedSessionReport}){
  assert.equal(source.status,'ready');
  const expected=assertWindowsUpgradePreservation({before:{...preserved,canonical_prefix:undefined,original_unknown_prefix:undefined},execution:preserved.execution,events:windowsPreservationValidationEvents(preserved)});
  if(preserved.app_pid===source.app_pid)return{kind:'direct_original_app',app_pid:source.app_pid};
  const prior=source.upgrade;assert(prior,'A different App PID requires the exact previous verified upgrade receipt');
  assert.equal(source.owner_jwt_reused,true);assert.equal(prior.preserved_session_report,resolve(preservedSessionReport));
  assert.equal(prior.preserved_session_report_sha256,createHash('sha256').update(preservedBytes).digest('hex'));
  assert.equal(prior.native_data_root_unchanged,true);assert.equal(prior.owner_jwt_unchanged,true);assert.equal(prior.new_dataset_created,false);
  assert.equal(source.exe_sha256,prior.new_exe_sha256);assert.equal(source.prior_app_pid,preserved.app_pid,'Every prior App hop must have a verified source receipt');
  assert.deepEqual(prior.canonical_preservation,expected,'The previous upgrade must have verified this exact canonical Session preservation');
  return{kind:'verified_prior_upgrade',app_pid:source.app_pid,original_guard_app_pid:preserved.app_pid,source_exe_sha256:source.exe_sha256};
}
export async function restartWindowsAcceptanceFixture({fixtureReport,upgradeExe,preservedSessionReport,additionalSessionReport,buildLabel}) {
  const source=JSON.parse(await readFile(fixtureReport,'utf8'));
  assert.equal(source.schema,'nomifun.windows-global-os-fixture.v1');assert.equal(source.status,'ready');const priorProcess=windowsProcessObservation(source.app_pid);assert.notEqual(priorProcess.state,'running','The exact prior App must have exited before same-dataset restart');
  assert.match(basename(source.parent),/^nomifun-global-os-owner-[0-9a-f-]{36}$/);assert.equal(source.native_data_root,join(source.parent,'native-data'));assert.equal(source.token_file,join(source.parent,'owner-jwt'));
  for(const path of [source.parent,source.native_data_root])await assertWindowsPrivatePath(path);
  assert.equal(await readFile(join(source.parent,'.nomifun-global-os-fixture'),'utf8'),'nomifun-global-os-native-acceptance-fixture.v1\n');
  assert.equal(await executableDigest(source.exe),source.exe_sha256,'Same-build persistence acceptance cannot silently replace the native executable');
  let exe=source.exe,upgrade,preserved,additional;
  if(upgradeExe){
    assert(preservedSessionReport,'An explicit App upgrade requires its original unknown Session evidence');
    exe=await realpath(resolve(upgradeExe));const item=await lstat(exe);assert(item.isFile()&&!item.isSymbolicLink());
    const newDigest=await executableDigest(exe);assert.notEqual(newDigest,source.exe_sha256,'An explicit upgrade must identify a distinct immutable native binary');
    const preservedBytes=await readFile(preservedSessionReport);preserved=JSON.parse(preservedBytes);
    assert(['nomifun.windows-desktop-unknown-observation.v1','nomifun.windows-desktop-upgrade-preservation.v2'].includes(preserved.schema));assert(preserved.canonical_prefix,'Upgrade needs a frozen digest of the complete original canonical event prefix');
    assert.match(preserved.canonical_prefix.sha256,/^[a-f0-9]{64}$/);assert(preserved.canonical_prefix.event_count>0&&preserved.canonical_prefix.last_seq>0);
    const preservationSource=assertWindowsDesktopPreservationSource({source,preserved,preservedBytes,preservedSessionReport});
    upgrade={old_exe:source.exe,old_exe_sha256:source.exe_sha256,new_exe:exe,new_exe_sha256:newDigest,preserved_session_report:resolve(preservedSessionReport),preserved_session_report_sha256:createHash('sha256').update(preservedBytes).digest('hex'),preserved_session_source:preservationSource,native_data_root_unchanged:true,owner_jwt_unchanged:true,new_dataset_created:false};
    if(additionalSessionReport){additional=await loadWindowsAdditionalSessionReport(resolve(additionalSessionReport),source);Object.assign(upgrade,{additional_session_report:additional.path,additional_session_report_sha256:additional.sha256});}
  }else {assert(!preservedSessionReport,'Session preservation is only an explicit upgrade argument');assert(!additionalSessionReport,'Additional Session preservation requires the existing explicit upgrade path');}
  const token=(await readWindowsOwnedFixtureBytes(source.token_file,4096)).toString('utf8').trim();assert.equal(token.split('.').length,3);
  const uriBefore=await captureWindowsDailyUriSnapshot({parent:source.parent,stage:'before'});
  const cdpPort=await allocateLoopbackPort(),suffix=randomUUID(),stdout=await open(join(source.parent,`app-restart-${suffix}.stdout.log`),'wx'),stderr=await open(join(source.parent,`app-restart-${suffix}.stderr.log`),'wx');
  const data=source.native_data_root,child=spawn(exe,[],{cwd:dirname(exe),windowsHide:true,detached:true,stdio:['ignore',stdout.fd,stderr.fd],env:{...process.env,NOMIFUN_DATA_DIR:data,NOMIFUN_WORK_DIR:join(data,'work'),NOMIFUN_CACHE_DIR:join(data,'cache'),NOMIFUN_LOG_DIR:join(data,'logs'),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${cdpPort}`}});let spawnError;child.once('error',error=>{spawnError=error;});child.unref();await stdout.close();await stderr.close();
  const report={...source,status:'restarting',exe,exe_sha256:upgrade?.new_exe_sha256??source.exe_sha256,build_label:buildLabel??source.build_label,upgrade,authenticated_native_caller:undefined,actual_app_identity:undefined,readonly_readiness_verification:undefined,ready_verified_at:undefined,app_pid:child.pid,cdp_port:cdpPort,restart_of:resolve(fixtureReport),prior_app_pid:source.app_pid,prior_process_observation:priorProcess,owner_jwt_reused:true,created_at:new Date().toISOString()};
  report.daily_uri_registration=windowsDailyUriLaunchObservation({kind:upgrade?'upgrade':'restart',before:uriBefore,priorFixtureReport:fixtureReport});
  const reportPath=join(source.report_root,`windows-fixture-${upgrade?'upgrade':'restart'}-${suffix}.json`);
  try{
    const deadline=Date.now()+180000;let announcement;
    while(Date.now()<deadline){
      if(spawnError)throw spawnError;if(child.exitCode!==null)throw new Error(`Restarted acceptance App exited before readiness (${child.exitCode})`);
      try{announcement=JSON.parse(await readFile(join(data,'port.json'),'utf8'));if(announcement.pid===child.pid){const baseUrl=`http://127.0.0.1:${announcement.port}`;const response=await fetch(`${baseUrl}/api/auth/user`,{headers:{Authorization:`Bearer ${token}`},signal:AbortSignal.timeout(1500)});if([401,403].includes(response.status))throw new Error('The original owner JWT was rejected after native restart');if(response.ok){Object.assign(report,{status:'ready',base_url:baseUrl,announcement});break;}}}catch(error){if(error.message==='The original owner JWT was rejected after native restart')throw error;}
      await delay(250);
    }
    assert.equal(report.status,'ready','Same-owner same-dataset native restart readiness was not proven');
    const target=await waitForRendererTarget(cdpPort,deadline);report.renderer=await rendererIdentity(target);report.actual_app_identity=await actualAppIdentity(child.pid,source.login_identity,exe);assert.equal(await executableDigest(exe),report.exe_sha256,'The immutable acceptance executable changed during native restart');
    if(preserved){
      const get=async path=>{const response=await fetch(`${report.base_url}${path}`,{headers:{Authorization:`Bearer ${token}`},signal:AbortSignal.timeout(15000)});assert(response.ok);const body=await response.json();return body.data??body;};
      const events=[];let after=0;for(let page=0;page<20;page++){const value=await get(`/api/agent-sessions/${preserved.session_id}/events?after_seq=${after}&limit=500`);events.push(...value.events);if(!value.events.length||value.next_cursor.seq<=after)break;after=value.next_cursor.seq;assert(page<19,'Original Session preservation exceeds bounded pagination');}
      report.upgrade.canonical_preservation=assertWindowsUpgradePreservation({before:preserved,execution:await get(`/api/agent-sessions/${preserved.session_id}/execution`),events});
    }
    if(additional)report.upgrade.additional_canonical_preservation=await verifyWindowsAdditionalSessionReport(additional,report);
    report.daily_uri_registration=windowsDailyUriLaunchObservation({kind:upgrade?'upgrade':'restart',before:uriBefore,after:await captureWindowsDailyUriSnapshot({parent:source.parent,stage:'after-ready'}),priorFixtureReport:fixtureReport});
    report.ready_verified_at=new Date().toISOString();
    return {...report,report_path:reportPath};
  }catch(error){Object.assign(report,{status:'failed',failure:{name:error.name,message:error.message}});throw error;}
  finally{await writeFile(reportPath,`${JSON.stringify(report,null,2)}\n`,{flag:'wx'});console.log(JSON.stringify({status:report.status,report:reportPath,app_pid:child.pid,base_url:report.base_url,native_data_root:data}));}
}
export async function verifyExistingWindowsUpgrade({fixtureReport}){
  const sourceBytes=await readFile(fixtureReport),source=JSON.parse(sourceBytes);assert.equal(source.schema,'nomifun.windows-global-os-fixture.v1');assert.equal(source.status,'failed');assert(source.upgrade&&source.restart_of);assert.equal(source.owner_jwt_reused,true);
  const original=JSON.parse(await readFile(source.restart_of,'utf8'));assert.equal(original.status,'ready');assert.equal(original.app_pid,source.prior_app_pid);assert.equal(source.native_data_root,original.native_data_root);assert.equal(source.token_file,original.token_file);assert.equal(source.parent,original.parent);assert(windowsProcessGone(original.app_pid));assert.equal(await executableDigest(source.exe),source.exe_sha256);assert.equal(source.exe_sha256,source.upgrade.new_exe_sha256);
  for(const path of [source.parent,source.native_data_root])await assertWindowsPrivatePath(path);assert.equal(await readFile(join(source.parent,'.nomifun-global-os-fixture'),'utf8'),'nomifun-global-os-native-acceptance-fixture.v1\n');
  const token=(await readWindowsOwnedFixtureBytes(source.token_file,4096)).toString('utf8').trim(),preservedBytes=await readFile(source.upgrade.preserved_session_report);assert.equal(createHash('sha256').update(preservedBytes).digest('hex'),source.upgrade.preserved_session_report_sha256);const preserved=JSON.parse(preservedBytes);
  const announcement=JSON.parse((await readWindowsOwnedFixtureBytes(join(source.native_data_root,'port.json'),4096)).toString('utf8'));assert.equal(announcement.pid,source.app_pid);assert.equal(announcement.port,Number(new URL(source.base_url).port));
  const get=async path=>{const response=await fetch(`${source.base_url}${path}`,{headers:{Authorization:`Bearer ${token}`},signal:AbortSignal.timeout(15000)});assert(response.ok);const value=await response.json();assert.notEqual(value.success,false);return value.data??value;};
  const caller=await get('/api/system/permissions');assert.equal(caller.platform,'windows');assert.equal(caller.process_id,source.app_pid);const target=await waitForRendererTarget(source.cdp_port,Date.now()+60000),renderer=await rendererIdentity(target),identity=await actualAppIdentity(source.app_pid,source.login_identity,source.exe);
  const events=[];let after=0;for(let page=0;page<20;page++){const value=await get(`/api/agent-sessions/${preserved.session_id}/events?after_seq=${after}&limit=500`);events.push(...value.events);if(!value.events.length||value.next_cursor.seq<=after)break;after=value.next_cursor.seq;assert(page<19);}
  const preservation=assertWindowsUpgradePreservation({before:preserved,execution:await get(`/api/agent-sessions/${preserved.session_id}/execution`),events});assert.equal(await executableDigest(source.exe),source.exe_sha256);assert.equal(createHash('sha256').update(await readFile(fixtureReport)).digest('hex'),createHash('sha256').update(sourceBytes).digest('hex'));
  let additionalPreservation;
  if(source.upgrade.additional_session_report){const additional=await loadWindowsAdditionalSessionReport(source.upgrade.additional_session_report,original);assert.equal(additional.sha256,source.upgrade.additional_session_report_sha256);additionalPreservation=await verifyWindowsAdditionalSessionReport(additional,{...source,status:'ready'});}
  const report={...source,status:'ready',failure:undefined,renderer,actual_app_identity:identity,authenticated_native_caller:{platform:caller.platform,process_id:caller.process_id,process_identity_available:true},upgrade:{...source.upgrade,canonical_preservation:preservation},readonly_readiness_verification:{original_failed_report:resolve(fixtureReport),original_failed_report_sha256:createHash('sha256').update(sourceBytes).digest('hex'),original_failed_report_unchanged:true,spawn_count:0,owner_setup_calls:0,owner_login_calls:0,agent_effect_calls:0,observed_at:new Date().toISOString()}};
  if(additionalPreservation)report.upgrade.additional_canonical_preservation=additionalPreservation;
  const path=join(source.report_root,`windows-fixture-upgrade-readback-${randomUUID()}.json`);await writeFile(path,JSON.stringify(report,null,2)+'\n',{flag:'wx'});console.log(JSON.stringify({status:'ready',report:path,app_pid:source.app_pid,base_url:source.base_url,original_canonical_prefix_event_count:preserved.canonical_prefix.event_count,original_canonical_prefix_unchanged:true,readonly_verification:true}));return{...report,report_path:path};
}
if(process.argv[1]===fileURLToPath(import.meta.url)){
  if(flag('--preserve-additional-session-report'))assert(flag('--upgrade-existing-fixture')&&flag('--preserve-session-report'),'Additional preservation requires the original Desktop preservation and explicit same-dataset upgrade');
  if(process.argv.includes('--help'))console.log('node scripts/validation/global-os-windows-fixture.mjs --exe C:\\ABS\\nomifun-desktop.exe --report-root C:\\ABS\\EVIDENCE [--build-label source-build]; --close-fixture-report C:\\ABS\\fixture.json requests existing coordinated product exit; --restart-fixture-report C:\\ABS\\ready-fixture.json restarts the same executable/dataset; --upgrade-existing-fixture C:\\ABS\\ready-fixture.json --exe C:\\ABS\\NEW.exe --preserve-session-report C:\\ABS\\unknown-observation.json explicitly upgrades after exact old PID exit. Both reuse original private owner JWT; upgrade verifies original unknown canonical Effect facts unchanged. Fresh launch alone performs normal owner setup/login. Never kills an App, changes an existing owner or deletes a dataset.');
  else if(flag('--freeze-additional-session'))await freezeWindowsAdditionalSession({sessionId:flag('--freeze-additional-session'),fixtureReport:flag('--fixture-report'),sourceObservation:flag('--source-observation'),reportRoot:flag('--report-root')});
  else if(flag('--close-fixture-report'))await closeWindowsAcceptanceFixture({fixtureReport:flag('--close-fixture-report')});
  else if(flag('--restart-fixture-report'))await restartWindowsAcceptanceFixture({fixtureReport:flag('--restart-fixture-report')});
  else if(flag('--verify-existing-upgrade-report'))await verifyExistingWindowsUpgrade({fixtureReport:flag('--verify-existing-upgrade-report')});
  else if(flag('--upgrade-existing-fixture')){assert(flag('--exe')&&flag('--preserve-session-report'),'Upgrade requires --exe and --preserve-session-report');await restartWindowsAcceptanceFixture({fixtureReport:flag('--upgrade-existing-fixture'),upgradeExe:flag('--exe'),preservedSessionReport:flag('--preserve-session-report'),additionalSessionReport:flag('--preserve-additional-session-report'),buildLabel:flag('--build-label')});}
  else {assert(flag('--exe')&&flag('--report-root'),'--exe and --report-root are required');await launchWindowsAcceptanceFixture({exe:flag('--exe'),reportRoot:flag('--report-root'),buildLabel:flag('--build-label')});}
  if(process.argv.includes('--help'))console.log('Read-only optional extra evidence: --freeze-additional-session 01a14893-93f7-7a90-b573-bc0b31116207 --fixture-report ABS_READY --source-observation ABS_ORIGINAL_UNC_JSON --report-root ABS_EVIDENCE. It does not close or restart an App. After review, append --preserve-additional-session-report ABS_SIDECAR to the existing upgrade command while retaining its mandatory Desktop --preserve-session-report. No reconciliation, replay, owner/dataset change or terminal settlement is performed.');
}
