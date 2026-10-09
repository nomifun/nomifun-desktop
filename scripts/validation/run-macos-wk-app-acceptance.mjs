#!/usr/bin/env node
// Drives the REAL, separately launched Desktop App through its authenticated
// canonical API. The fixture serves only a page and a scripted local model.
// This is deterministic tool-chain evidence, never real-model or visual proof.
//
// 1. cargo run -p nomifun-app --example browser_gui_fixture --features browser-use -- /NEW/ROOT --wk-actions
// 2. NOMIFUN_DATA_DIR=/NEW/ROOT /path/NomiFun.app/Contents/MacOS/nomifun-desktop
// 3. node scripts/validation/run-macos-wk-app-acceptance.mjs --data-dir /NEW/ROOT
//    Keep the real App's prepared Session selected for visible-page acceptance.
//    Use --stop for canonical Stop instead of normal completion. With
//    --hold-terminal, inspect the real visible App after OUTPUT.ready appears,
//    then create OUTPUT.release within five minutes to continue the driver.
//    Use --expect-stopped when the reviewer presses the real App Stop button
//    during that hold; the driver then verifies cancellation without resending it.
//    Native Browser snapshots/commands are desktop-local and remain protected.
//    Perform user toolbar, native input-lock and profile navigation/clear in the
//    real GUI; this external driver uses only canonical JWT APIs and witnesses.
//    Profile case labels are listed in the phase branches below. Append them as
//    ?case=LABEL (plus &seed=alpha|beta only for seed URLs) in the appropriate GUI
//    Session, then run --phase seed|verify|clear|verify-clear to verify witnesses.
//    verify/verify-clear require a different App PID than the preceding phase.
//    --phase witness --witness-case LABEL --expected-profile alpha|beta|empty
//    checks one exact user-navigated case. --case-prefix prefixes standard labels.
//    --phase pending-dialog --hold-terminal --expect-stopped retains one Agent
//    Confirm callback for real GUI Stop; do not answer the webpage dialog.
//    Each Agent scenario runs once per prepared root. Repeat or failed Agent
//    attempts require a fresh root, preserving the original evidence.
//    If Stop aborts only the page's callback-witness HTTP request, retain that
//    failed report, then --phase verify-pending --source-report FAILED.json
//    --gui-evidence RECEIPT.json verifies the exact old Operation without replay.
//    Receipt: operation_id, app_pid, observed_text="Confirm 已取消",
//    stop_clicked=true, observed_at=ISO timestamp; include screenshot/AX paths.
import { readFile, writeFile, mkdir, stat } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { randomUUID, createHash } from 'node:crypto';

const args=process.argv.slice(2);
const option=(name,fallback)=>{const index=args.indexOf(name);return index<0?fallback:args[index+1];};
const dataDir=resolve(option('--data-dir',''));
const phase=option('--phase','basic');
const allowed=['basic','seed','verify','clear','verify-clear','delete','witness','pending-dialog','verify-pending'];
if(!args.includes('--data-dir')||!allowed.includes(phase)) throw new Error('Use --data-dir <prepared new root> --phase basic|seed|verify|clear|verify-clear|delete|witness|pending-dialog|verify-pending');
if(phase==='pending-dialog'&&!args.includes('--stop')&&!(args.includes('--hold-terminal')&&args.includes('--expect-stopped')))throw new Error('Pending dialog requires --stop or --hold-terminal --expect-stopped');
if(process.platform!=='darwin') throw new Error('Actual macOS Desktop App required');
const fixture=JSON.parse(await readFile(join(dataDir,'wk-acceptance.json'),'utf8'));
if(fixture.mode!=='wk-actions'||resolve(fixture.data_dir)!==dataDir) throw new Error('Not the exact disposable WK fixture root');
const bound=JSON.parse(await readFile(join(dataDir,'port.json'),'utf8'));
const appAnnouncementMs=Math.floor((await stat(join(dataDir,'port.json'))).mtimeMs);
const api=`http://127.0.0.1:${bound.port}`;
const output=resolve(option('--output',join(dataDir,`wk-evidence-${phase}-${Date.now()}-${randomUUID()}.json`)));
const report={phase,app_pid:bound.pid,data_dir:dataDir,model:'local_scripted_protocol',real_model:false,visual_review:'separate_required',started_at:new Date().toISOString(),checks:{}};
let token;
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function request(base,path,method='GET',body,authenticate=false) {
  const response=await fetch(`${base}${path}`,{method,headers:{...(body===undefined?{}:{'content-type':'application/json'}),...(authenticate?{authorization:`Bearer ${token}`}:{})},body:body===undefined?undefined:JSON.stringify(body),signal:AbortSignal.timeout(30000)});
  const text=await response.text();
  let value; try {value=JSON.parse(text);} catch {value=text;}
  return {status:response.status,ok:response.ok,value};
}
async function product(path,method='GET',body) {
  const result=await request(api,path,method,body,true);
  if(!result.ok) throw new Error(`Product ${method} ${path} failed (${result.status}): ${JSON.stringify(result.value)}`);
  return result.value.data??result.value;
}
async function status() {const result=await request(fixture.control,'/status');if(!result.ok)throw new Error('Fixture control unavailable');return result.value;}
async function until(check,description,timeout=120000) {
  const deadline=Date.now()+timeout;
  do {const value=await check();if(value)return value;await sleep(250);} while(Date.now()<deadline);
  throw new Error(`Timed out: ${description}`);
}
const canonical=value=>Array.isArray(value)?value.map(canonical):value&&typeof value==='object'?Object.fromEntries(Object.keys(value).sort().map(key=>[key,canonical(value[key])])):value;
const digest=value=>createHash('sha256').update(JSON.stringify(canonical(value))).digest('hex');
async function preservation() {
  const before=JSON.parse(await readFile(join(dataDir,'wk-upgrade-preservation.json'),'utf8'));
  const providers=await product('/api/providers');
  const currentProvider=providers.find(provider=>provider.provider_id===before.provider_id);
  if(!currentProvider)throw new Error('Preserved provider is absent from authoritative list');
  const currentSession=await product(`/api/agent-sessions/${fixture.session_id}`);
  const currentMessages=await product(`/api/agent-sessions/${fixture.session_id}/messages?after_seq=0&limit=100`);
  const fixedSession=value=>({id:value.session.agent_session_id,owner:value.session.owner_ref,metadata:value.session.metadata,agent_binding:value.session.agent_binding});
  const checks={provider_configuration:[before.provider,currentProvider],session_identity_and_binding:[fixedSession(before.session),fixedSession(currentSession)]};
  for(const message of before.messages.messages) {
    const current=currentMessages.messages.find(item=>item.projection_id===message.projection_id);
    if(!current)throw new Error('Canonical upgrade seed message disappeared');
    checks[`canonical_message_${message.projection_id}`]=[message,current];
  }
  const actual=await readFile(join(dataDir,before.legacy_sentinel),'utf8');
  checks.legacy_cef_user_profile_preserved=[before.legacy_sentinel_text,actual];
  const evidence={};
  for(const [name,[oldValue,newValue]] of Object.entries(checks)) {
    const oldHash=digest(oldValue),newHash=digest(newValue);
    evidence[name]={before_sha256:oldHash,after_sha256:newHash};
    if(oldHash!==newHash)throw new Error(`Upgrade preservation mismatch: ${name}`);
  }
  return evidence;
}
async function profileWitness(label,expected) {
  const marker=`${option('--case-prefix','')}${label}`;
  const witness=await until(async()=>{const value=await status();return value.witnesses.findLast(w=>w.kind==='profile'&&w.case===marker&&w.received_at_ms>=appAnnouncementMs);},`current-App GUI Profile witness ${marker}`);
  if(witness.stored!==expected||witness.cookie!==expected)throw new Error(`Profile ${label} differs: ${JSON.stringify(witness)}`);
  report.checks[label]={case:marker,nonce:witness.nonce,received_at_ms:witness.received_at_ms,app_announcement_ms:appAnnouncementMs,local_storage:witness.stored,cookie:witness.cookie,session_selection:'separate_GUI_evidence'};
}
async function requireAppRestart(marker) {
  const prior=JSON.parse(await readFile(join(dataDir,`wk-profile-${marker}.json`),'utf8'));
  if(prior.app_pid===bound.pid||prior.session_id!==fixture.session_id||prior.secondary_session_id!==fixture.secondary_session_id)throw new Error(`Actual App restart required after ${marker} in this exact fixture root`);
  report.restart_evidence={previous_app_pid:prior.app_pid,current_app_pid:bound.pid,previous_report:prior.report};
}
try {
  // Credentials were randomly generated by the fixture preparer in this NEW
  // root. Never read user credentials, weaken App authentication, or print JWTs.
  const credentials=JSON.parse(await readFile(join(dataDir,'wk-acceptance-auth.json'),'utf8'));
  const login=await request(api,'/login','POST',credentials);
  if(!login.ok||typeof login.value.token!=='string')throw new Error(`Disposable fixture login failed (${login.status})`);
  token=login.value.token;
  report.upgrade_preservation_before=await preservation();
  if(phase==='basic'||phase==='pending-dialog') {
    const id=fixture.session_id;
    const pending=phase==='pending-dialog';
    const fixtureBefore=await status();
    const witnessOffset=fixtureBefore.witnesses.length;
    const beforeTurn=await product(`/api/agent-sessions/${id}`);
    const afterSeq=beforeTurn.head.last_seq;
    const requestId=randomUUID();
    await writeFile(join(dataDir,`wk-task-${phase}.started.json`),JSON.stringify({app_pid:bound.pid,request_id:requestId,report:output,started_at:new Date().toISOString()}),{flag:'wx'});
    const accepted=await product(`/api/agent-sessions/${id}/turns`,'POST',{input:{content:pending?'Verify WK pending dialog cancellation.':fixture.prompt},idempotency_key:requestId});
    report.operation_id=accepted.operation_id;
    const completed=await until(async()=>{
      const value=await status();
      if(value.failure)throw new Error(value.failure);
      if(pending) {
        const witnesses=value.witnesses.slice(witnessOffset);
        if(witnesses.some(w=>w.kind==='confirm'))throw new Error('Native Confirm returned before the reviewer could Stop it');
        const opened=witnesses.filter(w=>w.kind==='confirm-open');
        if(opened.length>1)throw new Error('Pending Confirm replayed before Stop');
        if(opened.length===1&&opened[0].trusted===false&&value.wk_evidence.pending_dialog_action_awaiting){report.pending_dialog=value.wk_evidence.pending_dialog;return value;}
        return null;
      }
      return value.wk_evidence.awaiting_terminal_release?value:null;
    },'canonical Browser tool sequence');
    report.checks=pending?{pending_dialog_action_awaiting:completed.wk_evidence.pending_dialog_action_awaiting}:{...completed.wk_evidence};
    report.model_calls=completed.model_calls-fixtureBefore.model_calls;
    report.model_calls_total=completed.model_calls;
    report.submit_witnesses=completed.witnesses.slice(witnessOffset).filter(w=>w.kind==='submit');
    if(!pending&&(report.submit_witnesses.length!==1||report.submit_witnesses[0].clickTrusted!==false||report.submit_witnesses[0].scrollTop<=0||report.submit_witnesses[0].name!=='WK 主应用表单 中文 café 🙂'||report.submit_witnesses[0].selected!=='Café 中文'))throw new Error('Expected exactly one semantic submit witness with exact entered text and selected option');
    if(!pending) {
      const clicks=completed.witnesses.slice(witnessOffset).filter(w=>w.kind==='user');
      if(clicks.length!==1||clicks[0].count!==1||clicks[0].trusted!==false)throw new Error('Element returned with screenshot was not used for exactly one semantic click');
      report.post_screenshot_click=clicks[0];
    }
    if(pending)report.checks.native_confirm_pending=true;
    const running=await product(`/api/agent-sessions/${id}`);
    if(running.head.status!=='running'||!running.head.active_turn_id)throw new Error('Canonical Turn ended before GUI hold');
    report.checks.canonical_running_during_hold=true;
    report.native_input_lock='requires_separate_GUI_evidence';
    if(args.includes('--hold-terminal')) {
      await mkdir(resolve(output,'..'),{recursive:true});
      await writeFile(`${output}.ready`,JSON.stringify({app_pid:bound.pid,session_id:id,operation_id:accepted.operation_id,output,release_file:`${output}.release`,canonical_head:running.head,witnesses:completed.witnesses.slice(witnessOffset),checks:report.checks},null,2),{flag:'wx'});
      console.log(JSON.stringify({status:'terminal_held',ready:`${output}.ready`,release:`${output}.release`}));
      await until(async()=>{try {await readFile(`${output}.release`);return true;}catch(error){if(error.code==='ENOENT')return false;throw error;}},'explicit acceptance driver release',300000);
      report.checks.terminal_hold_released=true;
    }
    if(args.includes('--stop')) {
      await product(`/api/agent-sessions/${id}/turns/cancel`,'POST',{idempotency_key:randomUUID()});
      report.checks.canonical_stop_requested=true;
    }
    // Cancellation already drops the product's provider request. Releasing a
    // canceled request can leave an unused permit that incorrectly completes
    // the following scenario. /shutdown cleans abandoned fixture waiters.
    if(!args.includes('--stop')&&!args.includes('--expect-stopped'))await request(fixture.control,'/finish','POST',{});
    const released=await until(async()=>{const value=await product(`/api/agent-sessions/${id}`);return !value.head.active_turn_id?value:null;},'canonical Turn idle after terminal cleanup');
    report.checks.canonical_idle_after_terminal=true;
    report.final_head=released.head;
    if(pending) {
      const settled=await until(async()=>{
        const value=await status(),witnesses=value.witnesses.slice(witnessOffset);
        const opened=witnesses.filter(item=>item.kind==='confirm-open'),closed=witnesses.filter(item=>item.kind==='confirm');
        if(opened.length>1||closed.length>1)throw new Error('Pending Confirm action replayed');
        return opened.length===1&&closed.length===1?{opened,closed}:null;
      },'pending native callback drained after Stop',10000);
      if(settled.opened[0].trusted!==false||settled.closed[0].accepted!==false)throw new Error('Pending semantic Confirm did not cancel honestly');
      report.dialog_witnesses=settled;
      report.checks.pending_callback_cancelled_once=true;
      report.checks.pending_action_not_replayed=true;
    }
    const events=await product(`/api/agent-sessions/${id}/events?after_seq=${afterSeq}&limit=1000`);
    const kinds=events.events.map(event=>event.kind);
    const expected=args.includes('--stop')||args.includes('--expect-stopped')?'turn/cancelled':'turn/completed';
    if(!events.events.some(event=>event.kind===expected&&event.correlation_id===accepted.operation_id))throw new Error(`Exact canonical Operation terminal missing ${expected}`);
    report.canonical_event_count=kinds.length;report.terminal=expected;
  } else if(phase==='verify-pending') {
    const sourcePath=option('--source-report'),guiPath=option('--gui-evidence');
    if(!sourcePath||!guiPath)throw new Error('Verification only requires --source-report and --gui-evidence');
    const prior=JSON.parse(await readFile(sourcePath,'utf8'));
    const ready=JSON.parse(await readFile(`${sourcePath}.ready`,'utf8'));
    const gui=JSON.parse(await readFile(guiPath,'utf8'));
    if(prior.phase!=='pending-dialog'||prior.passed!==false||prior.data_dir!==dataDir||!prior.error?.includes('pending native callback drained after Stop'))throw new Error('Source must be the preserved missing HTTP callback-witness failure from this root');
    if(gui.operation_id!==prior.operation_id||gui.app_pid!==prior.app_pid||gui.stop_clicked!==true||gui.observed_text!=='Confirm 已取消'||!Number.isFinite(Date.parse(gui.observed_at)))throw new Error('GUI receipt does not identify the same stopped Operation and actual DOM result');
    const original=ready.witnesses.filter(w=>w.kind==='confirm-open');
    if(original.length!==1||original[0].trusted!==false||Date.parse(gui.observed_at)<original[0].received_at_ms)throw new Error('GUI receipt precedes the original native Confirm');
    const witnesses=(await status()).witnesses.filter(w=>w.nonce===original[0].nonce);
    const opened=witnesses.filter(w=>w.kind==='confirm-open'),closed=witnesses.filter(w=>w.kind==='confirm');
    if(opened.length!==1||closed.length>1||closed.some(w=>w.accepted!==false))throw new Error('Original dialog was replayed or accepted');
    const events=(await product(`/api/agent-sessions/${fixture.session_id}/events?after_seq=0&limit=1000`)).events;
    const terminals=events.filter(e=>e.correlation_id===prior.operation_id&&['turn/completed','turn/failed','turn/cancelled'].includes(e.kind));
    if(terminals.length!==1||terminals[0].kind!=='turn/cancelled')throw new Error('Original Operation lacks a unique canonical cancelled terminal');
    const dispatches=events.filter(e=>e.kind==='tool/call-started'&&e.payload?.value?.call_id==='gui-wk-dialog-2'&&e.payload.value.operation_id===`${prior.operation_id}:tool:gui-wk-dialog-2`);
    if(dispatches.length!==1)throw new Error('Original Confirm tool did not dispatch exactly once');
    report.verification_only=true;report.source_failed_report=resolve(sourcePath);report.gui_evidence={path:resolve(guiPath),...gui};
    report.operation_id=prior.operation_id;report.terminal='turn/cancelled';report.callback_evidence_source='cua_dom';report.http_callback_witness=closed.length===1;
    report.checks={original_awaiting_dialog:ready.checks.pending_dialog_action_awaiting===true,canonical_cancelled_exact_operation:true,confirm_dispatched_once:true,actual_dom_cancel_result:true,no_model_or_browser_replay:true};
    if(!report.checks.original_awaiting_dialog)throw new Error('Original native awaiting-dialog receipt missing');
  } else if(phase==='seed') {
    await profileWitness('alpha-seeded','alpha');
    await profileWitness('beta-initial-empty',null);
    await profileWitness('beta-seeded','beta');
    await profileWitness('alpha-isolated','alpha');
  } else if(phase==='verify') {
    await requireAppRestart('seed');
    await profileWitness('alpha-persisted','alpha');
    await profileWitness('beta-persisted','beta');
  } else if(phase==='clear') {
    await profileWitness('alpha-cleared',null);
    await profileWitness('beta-preserved','beta');
  } else if(phase==='verify-clear') {
    await requireAppRestart('clear');
    await profileWitness('alpha-empty-after-restart',null);
    await profileWitness('beta-retained-after-restart','beta');
  } else if(phase==='witness') {
    const marker=option('--witness-case'),expected=option('--expected-profile');
    if(!marker||!['alpha','beta','empty'].includes(expected))throw new Error('Use --witness-case LABEL --expected-profile alpha|beta|empty');
    await profileWitness(marker,expected==='empty'?null:expected);
  } else if(phase==='delete') {
    await product(`/api/agent-sessions/${fixture.secondary_session_id}`,'DELETE');
    const deleted=await request(api,`/api/agent-sessions/${fixture.secondary_session_id}`,'GET',undefined,true);
    if(deleted.status!==404)throw new Error(`Deleted canonical Session still accessible (${deleted.status})`);
    report.checks.deleted_canonical_session_inaccessible=true;
    report.checks.native_store_removal_acknowledged=true;
  }
  report.upgrade_preservation_after=await preservation();
  if(['seed','clear'].includes(phase))await writeFile(join(dataDir,`wk-profile-${phase}.json`),JSON.stringify({app_pid:bound.pid,session_id:fixture.session_id,secondary_session_id:fixture.secondary_session_id,report:output}),{flag:'wx'});
  report.passed=true;
} catch(error) {report.passed=false;report.error=error.message;process.exitCode=1;}
finally {
  report.finished_at=new Date().toISOString();
  await mkdir(resolve(output,'..'),{recursive:true});
  await writeFile(output,JSON.stringify(report,null,2),{flag:'wx'});
  console.log(JSON.stringify({report:output,passed:report.passed,phase,error:report.error}));
}
