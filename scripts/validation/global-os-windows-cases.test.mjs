import { expect, test } from 'bun:test';
import { execFile, spawn } from 'node:child_process';
import { mkdir, readFile, realpath, rmdir, writeFile, unlink, link } from 'node:fs/promises';
import { createHash, randomUUID } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join,resolve } from 'node:path';
import { promisify } from 'node:util';
import { assertWindowsFixture, initializeWindowsPrivateEmptyFixture, createWindowsAdministratorScenario, createWindowsFilesScenario, createWindowsSystemScenario, powershell, psArguments, psLiteral, windowsLoginIdentity, readWindowsOwnedFixtureBytes, windowsProcessObservation, windowsProcessGone } from './global-os-windows-cases.mjs';
import { createWindowsHighDesktopScenario, windowsCalculatorDisplayHasExactValue, windowsIntegrityObservationScript, windowsUiaEntries, windowsOwnedApplicationWindows,sanitizeWindowsDesktopReport } from './global-os-windows-desktop-cases.mjs';
import {assertWindowsUpgradePreservation,windowsPreservationValidationEvents} from './global-os-windows-fixture.mjs';

const nativeTest = test.skipIf(process.platform !== 'win32');
const execute = promisify(execFile);

nativeTest('the independent control fixture ACL is limited to its new empty login-owned root',async()=>{
  const fixture=join(await realpath(tmpdir()),`nomifun-global-os-product-${randomUUID()}`),identity=await windowsLoginIdentity();await mkdir(fixture);
  try{const receipt=await initializeWindowsPrivateEmptyFixture(fixture,identity);expect(receipt).toEqual({owner_sid:identity.sid,acl_private:true});
    const marker=join(fixture,'guard.txt');await writeFile(marker,'guard',{flag:'wx'});
    try{await expect(initializeWindowsPrivateEmptyFixture(fixture,identity)).rejects.toThrow('empty fixture');expect(await readFile(marker,'utf8')).toBe('guard');}finally{await unlink(marker);}
  }finally{await rmdir(fixture);}
});
test('explicit same-dataset App upgrade cannot erase or rewrite the original unknown Effect',()=>{
  const effect={seq:74,event_id:'effect:uncertain',agent_session_id:'original-session',kind:'effect/uncertain',payload:{storage:'inline_json',value:{outcome:'unknown',input_digest:'original-digest'}}};
  const execution={agent_session_id:'original-session',unknown_effects:1,pending_effects:0,operation_id:'original-operation',state:'cancelled'};
  const before={schema:'nomifun.windows-desktop-unknown-observation.v1',session_id:'original-session',execution,effect_events:[effect]};
  expect(assertWindowsUpgradePreservation({before,execution,events:[effect]}).all_original_effect_events_unchanged).toBe(true);
  for(const changed of [{...execution,unknown_effects:0},{...execution,operation_id:'different-operation'}])expect(()=>assertWindowsUpgradePreservation({before,execution:changed,events:[effect]})).toThrow();
  expect(()=>assertWindowsUpgradePreservation({before,execution,events:[]})).toThrow();
  expect(()=>assertWindowsUpgradePreservation({before,execution,events:[{...effect,payload:{...effect.payload,value:{...effect.payload.value,input_digest:'changed'}}}]})).toThrow();
  const ordinary={seq:73,event_id:'original-payload-reference',kind:'runtime/progress-recorded',payload:{storage:'payload_ref',payload_id:'original-payload',digest:'original-reference'}};
  const prefix=[ordinary,effect],stable=value=>JSON.stringify(value,(_,entry)=>entry&&typeof entry==='object'&&!Array.isArray(entry)?Object.fromEntries(Object.keys(entry).sort().map(key=>[key,entry[key]])):entry);
  const frozen={...before,canonical_prefix:{last_seq:74,event_count:2,sha256:createHash('sha256').update(stable(prefix)).digest('hex')}};
  expect(assertWindowsUpgradePreservation({before:frozen,execution,events:prefix}).all_original_effect_events_unchanged).toBe(true);
  expect(()=>assertWindowsUpgradePreservation({before:frozen,execution,events:[{...ordinary,payload:{...ordinary.payload,payload_id:'replacement-payload'}},effect]})).toThrow();
});
test('upgrade after human reconciliation requires exact canonical attestation while retaining the original unknown events',()=>{
  const session='original',effect_id='original-effect',input_digest='a'.repeat(64),evidence_digest='b'.repeat(64);
  const event=(seq,event_id,kind,value)=>({seq,event_id,agent_session_id:session,kind,payload:{storage:'inline_json',value}});
  const uncertain=event(1,'uncertain','effect/uncertain',{effect_id,input_digest,outcome:'unknown'}),attestation=event(2,'human-attestation','runtime/effect-reconciliation-attested',{effect_id,input_digest,authority:'session_owner_verified',outcome:'confirmed_failed',evidence:{verified:true,evidence_digest}}),reconciled=event(3,'human-attestation:effect','effect/reconciled',{effect_id,input_digest,outcome:'confirmed_failed'});
  const execution={unknown_effects:0,pending_effects:0,operation_id:'original-turn',state:'cancelled'},before={schema:'nomifun.windows-desktop-upgrade-preservation.v2',session_id:session,execution,effect_events:[uncertain,reconciled],owner_reconciliation:{effect_id,input_digest,outcome:'confirmed_failed',human_verification_sha256:evidence_digest,attestation_event:attestation,reconciled_event:reconciled}},events=[uncertain,attestation,reconciled];
  expect(assertWindowsUpgradePreservation({before,execution,events}).all_original_effect_events_unchanged).toBe(true);
  const complete={...reconciled,producer_id:'native_owner_reconciliation',kind_version:1,causation_event_id:'uncertain',idempotency_key:'original-effect:lifecycle'},compacted={...before,owner_reconciliation:{...before.owner_reconciliation,reconciled_event:complete}};
  const validation=windowsPreservationValidationEvents(compacted);expect(validation.find(event=>event.event_id===complete.event_id)).toEqual(complete);expect(assertWindowsUpgradePreservation({before:compacted,execution,events:validation}).all_original_effect_events_unchanged).toBe(true);
  expect(()=>windowsPreservationValidationEvents({...compacted,owner_reconciliation:{...compacted.owner_reconciliation,reconciled_event:{...complete,seq:99}}})).toThrow();
  for(const change of [value=>{value.before.owner_reconciliation=undefined;},value=>{value.before.owner_reconciliation.attestation_event.payload.value.evidence.verified=false;},value=>{value.before.owner_reconciliation.human_verification_sha256='c'.repeat(64);},value=>{value.before.effect_events=value.before.effect_events.filter(event=>event.kind!=='effect/uncertain');},value=>{value.before.owner_reconciliation.outcome='confirmed_succeeded';}]){const invalid=structuredClone({before,execution,events});change(invalid);expect(()=>assertWindowsUpgradePreservation(invalid)).toThrow();}
});
nativeTest('exact process observation requests only native query/synchronize rights and preserves creation identity',async()=>{
  const own=windowsProcessObservation(process.pid,{includeImagePath:true});expect(own.state).toBe('running');expect(own.access).toBe('QUERY_LIMITED_INFORMATION|SYNCHRONIZE');expect(resolve(own.image_path).toLowerCase()).toBe(resolve(process.execPath).toLowerCase());expect(windowsProcessGone(process.pid)).toBe(false);
  const child=spawn(process.execPath,['-e','process.stdin.resume();process.stdin.on("end",()=>process.exit(23))'],{windowsHide:true,stdio:['pipe','ignore','ignore']});
  const exit=new Promise((done,fail)=>{child.once('error',fail);child.once('exit',(code,signal)=>done({code,signal}));});
  const live=windowsProcessObservation(child.pid);expect(live.state).toBe('running');expect(live.creation_filetime).toMatch(/^[1-9][0-9]+$/);
  child.stdin.end();expect(await exit).toEqual({code:23,signal:null});
  const final=windowsProcessObservation(child.pid);expect(['absent','exited']).toContain(final.state);if(final.state==='exited'){expect(final.creation_filetime).toBe(live.creation_filetime);expect(final.exit_code).toBe('23');}expect(windowsProcessGone(child.pid)).toBe(true);
},30000);
nativeTest('native PowerShell encoded commands preserve literal whitespace, Unicode, quotes and metacharacters', async () => {
  const expected = ['', '中文 空格', `single' and "double"`, '% ! & | < > ^ $ $(throw "NOT_LITERAL")', 'back\\slash', 'line\nend'];
  const result = await execute(powershell, psArguments(`ConvertTo-Json -InputObject @(${expected.map(psLiteral).join(',')}) -Compress`), { encoding: 'utf8', windowsHide: true });
  expect(JSON.parse(result.stdout)).toEqual(expected);
});

nativeTest('all Windows canonical scenario PowerShell commands parse natively without executing their mutations', async () => {
  const fixture = join(await realpath(tmpdir()), `nomifun-global-os-product-${randomUUID()}`), workspace = join(fixture, 'workspace');
  await mkdir(fixture); await mkdir(workspace);
  try {
    const loginIdentity = await windowsLoginIdentity();
    const ordinary = createWindowsFilesScenario({ fixture, workspace, outside: join(fixture, 'outside'), initial: 'before\n', changed: 'after 中文\n' });
    const admin = createWindowsAdministratorScenario({ fixture, root: join(fixture, 'admin-root'), instructions: join(fixture, 'admin-root', 'AGENTS.md'), instructionContent: 'isolated fixture\n', marker: join(fixture, 'admin-root', 'marker.txt'), markerValue: 'marker', instructionEvidence: {}, verifyInstructions: () => { throw new Error('not a product run'); } });
    const system = await createWindowsSystemScenario({ fixture, workspace, loginIdentity });
    const high = await createWindowsHighDesktopScenario({ fixture, workspace, loginIdentity });
    const scripts = [];
    for (const step of [...ordinary.steps, ...admin.steps, ...admin.cleanupSteps, ...system.steps, ...system.cleanupSteps, ...high.steps]) {
      const args = typeof step.args === 'function' ? await step.args() : step.args;
      if (args.command === powershell && args.args?.includes('-EncodedCommand')) scripts.push(Buffer.from(args.args.at(-1), 'base64').toString('utf16le'));
      if (step.name === 'write_file' && step.args.path.endsWith('.ps1')) scripts.push(step.args.content);
    }
    const code = '$scripts=[Console]::In.ReadToEnd()|ConvertFrom-Json;$failures=@();foreach($script in $scripts){$tokens=$null;$errors=$null;[Management.Automation.Language.Parser]::ParseInput($script,[ref]$tokens,[ref]$errors)|Out-Null;foreach($e in $errors){$failures+=$e.Message}};ConvertTo-Json -InputObject @($failures) -Compress';
    const stdout = await new Promise((done, fail) => {
      const child = execFile(powershell, psArguments(code), { encoding: 'utf8', windowsHide: true, maxBuffer: 1024 * 1024 }, (error, output) => error ? fail(error) : done(output));
      child.stdin.end(JSON.stringify(scripts));
    });
    expect(JSON.parse(stdout)).toEqual([]);
    expect(scripts.length).toBeGreaterThan(12);
    expect(system.phases.map(phase => phase.role)).toEqual(['system-install-config', 'system-service', 'system-network', 'system-finalize']);
    expect(system.steps.filter(step => step.name === 'start_process')).toHaveLength(1);
    await expect(assertWindowsFixture(join(fixture, '..'), workspace)).rejects.toThrow();
  } finally { await rmdir(workspace); await rmdir(fixture); }
});

nativeTest('integrity fixture queries actual ordinary process token independently of administrator membership', async () => {
  const result=await execute(powershell,psArguments(windowsIntegrityObservationScript(process.pid)),{encoding:'utf8',windowsHide:true});
  expect(JSON.parse(result.stdout)).toBe(8192);
  const owned=await windowsOwnedApplicationWindows(process.pid);expect(Array.isArray(owned)).toBe(true);
  expect(owned.every(window=>window.owner_pid===process.pid||window.owned_descendant===true)).toBe(true);
});

nativeTest('private Windows fixture reader checks actual ACL, size and hardlink identity', async () => {
  const root = join(await realpath(tmpdir()), `nomifun-global-os-product-${randomUUID()}`), path = join(root, 'metadata');
  await mkdir(root);
  try {
    const script = `$i=[Security.Principal.WindowsIdentity]::GetCurrent();$a=New-Object Security.AccessControl.DirectorySecurity;$a.SetOwner($i.User);$a.SetAccessRuleProtection($true,$false);foreach($s in @($i.User.Value,'S-1-5-18','S-1-5-32-544')){$r=New-Object Security.AccessControl.FileSystemAccessRule((New-Object Security.Principal.SecurityIdentifier($s)),'FullControl','ContainerInherit,ObjectInherit','None','Allow');$a.AddAccessRule($r)};[IO.Directory]::SetAccessControl(${psLiteral(root)},$a)`;
    await execute(powershell, psArguments(script), { windowsHide: true });
    await writeFile(path, 'bounded'); expect((await readWindowsOwnedFixtureBytes(path, 64)).toString()).toBe('bounded');
    await expect(readWindowsOwnedFixtureBytes(path, 2)).rejects.toThrow();
    await link(path, join(root, 'linked')); await expect(readWindowsOwnedFixtureBytes(path, 64)).rejects.toThrow(); await unlink(join(root, 'linked'));
    await execute(powershell, psArguments(`$p=${psLiteral(path)};$a=[IO.File]::GetAccessControl($p);$a.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule((New-Object Security.Principal.SecurityIdentifier('S-1-1-0')),'Read','Allow')));[IO.File]::SetAccessControl($p,$a)`), { windowsHide: true });
    await expect(readWindowsOwnedFixtureBytes(path, 64)).rejects.toThrow();
    expect(await readFile(path, 'utf8')).toBe('bounded');
  } finally { await unlink(path).catch(error => { if (error.code !== 'ENOENT') throw error; }); await rmdir(root); }
});

test('Windows Calculator display parser accepts only exact nonbutton values', () => {
  for (const label of ['46', 'Display is 46', '显示为 46', '显示为46']) expect(windowsCalculatorDisplayHasExactValue(`[8] text ${JSON.stringify(label)}`, '46')).toBe(true);
  for (const text of ['[8] button "46"', '[8] text "146"', '[8] text "Display is 46.0"', '[8] text "12+34=46"', '[8] text "显示为 46" = "47"']) expect(windowsCalculatorDisplayHasExactValue(text, '46')).toBe(false);
});
test('Windows desktop redaction retains stable failure code/step without private native bodies',()=>{
  const safe=sanitizeWindowsDesktopReport({failure:{name:'Error',code:'ROLE_HOST_PROVIDER_FAILURE',step:'desktop-focus',message:'private global UI text'},observations:{call:{result:{text:'private global UI text'}}}});
  expect(safe.windows_failure_metadata.failure).toEqual({code:'ROLE_HOST_PROVIDER_FAILURE',step:'desktop-focus'});expect(JSON.stringify(safe)).not.toContain('private global UI text');
});

test('native Windows semantic tree refs preserve their owning window and ignore another application display',()=>{
  const tree='desktop\n├── window "Calculator"\n│   ├── [1] text "Display is 46" (400,100)  [action: click]\n│   └── [2] button "One" (200,300)  [action: click]  [focused]\n└── window "Unrelated"\n    └── [3] text "47" (800,400)  [action: click]';
  expect(windowsUiaEntries(tree)).toEqual([{ref:1,role:'text',name:'Display is 46',value:undefined,window:'Calculator'},{ref:2,role:'button',name:'One',value:undefined,window:'Calculator'},{ref:3,role:'text',name:'47',value:undefined,window:'Unrelated'}]);
  expect(windowsCalculatorDisplayHasExactValue(tree,'46','Calculator')).toBe(true);
  expect(windowsCalculatorDisplayHasExactValue(tree,'47','Calculator')).toBe(false);
  expect(windowsCalculatorDisplayHasExactValue('[1] text "\\u{200e}46\\u{200e}"','46')).toBe(true);
});
