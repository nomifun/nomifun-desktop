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
import { windowsIntegrityObservationScript } from './global-os-windows-cases.mjs';
import {freezeWindowsAdditionalSession} from './global-os-windows-unsettled-preservation.mjs';
import {captureWindowsDailyUriSnapshot,windowsDailyUriLaunchObservation,windowsDailyUriCloseObservation} from './global-os-windows-uri-baseline.mjs';
import {beginWindowsProcessExitObservation,windowsCloseProcessIdentity} from './global-os-windows-process-exit-observer.mjs';

const delay=ms=>new Promise(done=>setTimeout(done,ms));
const flag=name=>{const i=process.argv.indexOf(name);return i<0?undefined:process.argv[i+1];};
async function executableDigest(path) {const hash=createHash('sha256');for await(const bytes of createReadStream(path))hash.update(bytes);return hash.digest('hex');}
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



export function assertWindowsPriorAppExited(source,observation) {
  assert.equal(observation.pid,source.app_pid);assert(['running','exited','absent'].includes(observation.state));
  if(observation.state!=='running')return observation;
  const original=source.actual_app_identity?.native_process;
  assert.equal(original?.pid,source.app_pid,'A reused PID requires the original exact native App identity');
  assert.match(original.creation_filetime,/^[1-9][0-9]+$/);assert.match(observation.creation_filetime,/^[1-9][0-9]+$/);
  assert.notEqual(observation.creation_filetime,original.creation_filetime,'The exact prior App must have exited before same-dataset restart');
  // PID reuse proves only that the original generation is gone. The old
  // retained-HANDLE exit receipt remains the separate source of its exit code.
  return{...observation,pid_reused:true,expected_prior_creation_filetime:original.creation_filetime,prior_app_running:false};
}
export async function restartWindowsAcceptanceFixture({fixtureReport,buildLabel}) {
  const source=JSON.parse(await readFile(fixtureReport,'utf8'));
  assert.equal(source.schema,'nomifun.windows-global-os-fixture.v1');assert.equal(source.status,'ready');const priorProcess=assertWindowsPriorAppExited(source,windowsProcessObservation(source.app_pid));
  assert.match(basename(source.parent),/^nomifun-global-os-owner-[0-9a-f-]{36}$/);assert.equal(source.native_data_root,join(source.parent,'native-data'));assert.equal(source.token_file,join(source.parent,'owner-jwt'));
  for(const path of [source.parent,source.native_data_root])await assertWindowsPrivatePath(path);
  assert.equal(await readFile(join(source.parent,'.nomifun-global-os-fixture'),'utf8'),'nomifun-global-os-native-acceptance-fixture.v1\n');
  assert.equal(await executableDigest(source.exe),source.exe_sha256,'Same-build persistence acceptance cannot silently replace the native executable');
  const exe=source.exe;

  const token=(await readWindowsOwnedFixtureBytes(source.token_file,4096)).toString('utf8').trim();assert.equal(token.split('.').length,3);
  const uriBefore=await captureWindowsDailyUriSnapshot({parent:source.parent,stage:'before'});
  const cdpPort=await allocateLoopbackPort(),suffix=randomUUID(),stdout=await open(join(source.parent,`app-restart-${suffix}.stdout.log`),'wx'),stderr=await open(join(source.parent,`app-restart-${suffix}.stderr.log`),'wx');
  const data=source.native_data_root,child=spawn(exe,[],{cwd:dirname(exe),windowsHide:true,detached:true,stdio:['ignore',stdout.fd,stderr.fd],env:{...process.env,NOMIFUN_DATA_DIR:data,NOMIFUN_WORK_DIR:join(data,'work'),NOMIFUN_CACHE_DIR:join(data,'cache'),NOMIFUN_LOG_DIR:join(data,'logs'),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${cdpPort}`}});let spawnError;child.once('error',error=>{spawnError=error;});child.unref();await stdout.close();await stderr.close();
  const report={...source,status:'restarting',exe,exe_sha256:source.exe_sha256,build_label:buildLabel??source.build_label,authenticated_native_caller:undefined,actual_app_identity:undefined,readonly_readiness_verification:undefined,ready_verified_at:undefined,app_pid:child.pid,cdp_port:cdpPort,restart_of:resolve(fixtureReport),prior_app_pid:source.app_pid,prior_process_observation:priorProcess,owner_jwt_reused:true,created_at:new Date().toISOString()};
  report.daily_uri_registration=windowsDailyUriLaunchObservation({kind:'restart',before:uriBefore,priorFixtureReport:fixtureReport});
  const reportPath=join(source.report_root,`windows-fixture-restart-${suffix}.json`);
  try{
    const deadline=Date.now()+180000;let announcement;
    while(Date.now()<deadline){
      if(spawnError)throw spawnError;if(child.exitCode!==null)throw new Error(`Restarted acceptance App exited before readiness (${child.exitCode})`);
      try{announcement=JSON.parse(await readFile(join(data,'port.json'),'utf8'));if(announcement.pid===child.pid){const baseUrl=`http://127.0.0.1:${announcement.port}`;const response=await fetch(`${baseUrl}/api/auth/user`,{headers:{Authorization:`Bearer ${token}`},signal:AbortSignal.timeout(1500)});if([401,403].includes(response.status))throw new Error('The original owner JWT was rejected after native restart');if(response.ok){Object.assign(report,{status:'ready',base_url:baseUrl,announcement});break;}}}catch(error){if(error.message==='The original owner JWT was rejected after native restart')throw error;}
      await delay(250);
    }
    assert.equal(report.status,'ready','Same-owner same-dataset native restart readiness was not proven');
    const target=await waitForRendererTarget(cdpPort,deadline);report.renderer=await rendererIdentity(target);report.actual_app_identity=await actualAppIdentity(child.pid,source.login_identity,exe);assert.equal(await executableDigest(exe),report.exe_sha256,'The immutable acceptance executable changed during native restart');
    const get=async path=>{const response=await fetch(`${report.base_url}${path}`,{headers:{Authorization:`Bearer ${token}`},signal:AbortSignal.timeout(15000)});assert(response.ok);const body=await response.json();assert.notEqual(body.success,false);return body.data??body;};
    const caller=await get('/api/system/permissions');assert.equal(caller.platform,'windows');assert.equal(caller.process_id,child.pid);
    report.authenticated_native_caller={platform:caller.platform,process_id:caller.process_id,process_identity_available:true};


    report.daily_uri_registration=windowsDailyUriLaunchObservation({kind:'restart',before:uriBefore,after:await captureWindowsDailyUriSnapshot({parent:source.parent,stage:'after-ready'}),priorFixtureReport:fixtureReport});
    report.ready_verified_at=new Date().toISOString();
    return {...report,report_path:reportPath};
  }catch(error){Object.assign(report,{status:'failed',failure:{name:error.name,message:error.message}});throw error;}
  finally{await writeFile(reportPath,`${JSON.stringify(report,null,2)}\n`,{flag:'wx'});console.log(JSON.stringify({status:report.status,report:reportPath,app_pid:child.pid,base_url:report.base_url,native_data_root:data}));}
}

if(process.argv[1]===fileURLToPath(import.meta.url)){
  if(process.argv.includes('--help'))console.log('node scripts/validation/global-os-windows-fixture.mjs --exe ABS_NATIVE_APP --report-root ABS_EVIDENCE [--build-label SOURCE]; --close-fixture-report ABS_FIXTURE requests coordinated product exit; --restart-fixture-report ABS_READY restarts the same executable/dataset after exact prior process exit. Fresh launch alone performs normal private owner setup/login.');
  else if(flag('--freeze-additional-session'))await freezeWindowsAdditionalSession({sessionId:flag('--freeze-additional-session'),fixtureReport:flag('--fixture-report'),sourceObservation:flag('--source-observation'),reportRoot:flag('--report-root')});
  else if(flag('--close-fixture-report'))await closeWindowsAcceptanceFixture({fixtureReport:flag('--close-fixture-report')});
  else if(flag('--restart-fixture-report'))await restartWindowsAcceptanceFixture({fixtureReport:flag('--restart-fixture-report')});
  else {assert(flag('--exe')&&flag('--report-root'),'--exe and --report-root are required');await launchWindowsAcceptanceFixture({exe:flag('--exe'),reportRoot:flag('--report-root'),buildLabel:flag('--build-label')});}
}
