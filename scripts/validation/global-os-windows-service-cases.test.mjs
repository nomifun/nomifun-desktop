import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdir, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join,win32 } from 'node:path';
import test from 'node:test';
import { createWindowsServiceScenario, windowsServiceCompileScript, windowsServiceFixtureSource,windowsServiceProtectedCopyScript } from './global-os-windows-service-cases.mjs';
import { psLiteral, windowsLoginIdentity, windowsObservation } from './global-os-windows-cases.mjs';
import {windowsServiceBuildCleanupSource} from './global-os-windows-service-cleanup.mjs';

test('SCM fixture source rejects non-unique service identity before any native action', () => {
  assert.throws(() => windowsServiceFixtureSource({ serviceName: 'Spooler', nonce: randomUUID(), startMarker: 'a', stopMarker: 'b' }));
});
test('SCM ordinary build remains separate from its protected service directory and ACL is supplied at creation',()=>{
  const input={root:'C:\\fixture\\scm-own',compiledExecutable:'C:\\fixture\\build-own\\service.exe',executable:'C:\\fixture\\scm-own\\service.exe',expectedSha256:'a'.repeat(64)},script=windowsServiceProtectedCopyScript(input);
  assert(script.includes('[IO.Directory]::CreateDirectory($root,$acl)'));assert(script.indexOf('[IO.Directory]::CreateDirectory($root,$acl)')<script.indexOf('[IO.File]::Copy($source,$exe,$false)'));assert(script.includes('file_owner_sid'));assert(script.includes('Compiled service source hash changed'));
  assert.throws(()=>windowsServiceProtectedCopyScript({...input,compiledExecutable:input.executable}));assert.throws(()=>windowsServiceProtectedCopyScript({...input,expectedSha256:'invalid'}));
});
test('SCM residual compilation cleanup rejects any other namespace or unresolved source operation',()=>{
  assert.throws(()=>windowsServiceBuildCleanupSource({schema:'nomifun.global-os-native-regression.v2',platform:'win32',status:'failed',scenario:'browser',administrator:false}));
  assert.throws(()=>windowsServiceBuildCleanupSource({schema:'nomifun.global-os-native-regression.v2',platform:'win32',status:'failed',scenario:'service',administrator:false,fixture:'C:\\daily',workspace:'C:\\daily\\workspace'}));
});
test('SCM read-only completion continues only exact settled business and never accepts missing lifecycle proof',()=>{
  const fixture=join('C:\\Temp',`nomifun-global-os-product-${randomUUID()}`),nonce=randomUUID(),root=join(fixture,`scm-${nonce}`),buildRoot=join(fixture,`scm-build-${nonce}`),operation='original',session='session';
  const source={schema:'nomifun.global-os-native-regression.v2',platform:'win32',status:'failed',scenario:'service',administrator:false,fixture,workspace:join(fixture,'workspace'),session_id:session,native_case_resources:{root,buildRoot,serviceName:`NomiFunAcceptance_${nonce.replaceAll('-','')}`,compiledExecutable:join(buildRoot,'service.exe'),executable:join(root,'service.exe'),startMarker:join(root,'started.json'),stopMarker:join(root,'stopped.json'),executable_sha256:'a'.repeat(64),pid:1234,process_creation_filetime:'1340000000000'},checks:['scm-create','scm-configure','scm-start','scm-process-identity','scm-system-marker','scm-query','scm-stop','scm-process-exited','scm-stop-marker','scm-delete','scm-cleanup'].map(step=>({step,status:'passed'})),native_case_evidence:[{phase:'scm_deleted_files_removed',exact_pid_gone:true}],failure_settlement:{execution:{operation_id:operation,state:'cancelled',turn_state:'cancelled',producer_lease_live:false,checkpoint_retained:false,recovery_blocked:false,pause_requested:false,automatic_replay_authorized:false,pending_effects:0,unknown_effects:0,checkpoint_seq:null,checkpoint_digest:null,pause:null},head:{session_id:session,status:'ready'}}};
  assert.equal(windowsServiceBuildCleanupSource(source,{verifyOnly:true}).pid,1234);
  const missing=structuredClone(source);missing.checks.pop();assert.throws(()=>windowsServiceBuildCleanupSource(missing,{verifyOnly:true}));
  const unknown=structuredClone(source);unknown.failure_settlement.execution.unknown_effects=1;assert.throws(()=>windowsServiceBuildCleanupSource(unknown,{verifyOnly:true}));
  const foreign=structuredClone(source);foreign.native_case_resources.startMarker='C:\\daily\\private';assert.throws(()=>windowsServiceBuildCleanupSource(foreign,{verifyOnly:true}));
});

test('actual Windows parser and .NET compiler accept the unique SCM fixture without registering it', { skip: process.platform !== 'win32' }, async () => {
  const fixture = join(await realpath(tmpdir()), `nomifun-global-os-product-${randomUUID()}`);
  await mkdir(fixture);
  try {
    const loginIdentity = await windowsLoginIdentity();
    const scenario = await createWindowsServiceScenario({ fixture, workspace: join(fixture, 'workspace'), loginIdentity });
    assert.equal(scenario.steps[0].args.path,fixture);assert.equal(scenario.steps[0].args.recursive,false);assert.notEqual(scenario.resources.root,scenario.resources.buildRoot);assert.equal(win32.dirname(scenario.resources.source),scenario.resources.buildRoot);assert.equal(win32.dirname(scenario.resources.compiledExecutable),scenario.resources.buildRoot);
    for (const step of [...scenario.steps, ...scenario.cleanupSteps]) {
      const args = typeof step.args==='function'?(step.id==='scm-cleanup'?step.args():undefined):step.args;
      if (!Array.isArray(args?.args)) continue;
      const at = args.args.indexOf('-EncodedCommand');
      if (at < 0) continue;
      const script = Buffer.from(args.args[at + 1], 'base64').toString('utf16le');
      const parsed = await windowsObservation(`$t=$null;$e=$null;[Management.Automation.Language.Parser]::ParseInput(${psLiteral(script)},[ref]$t,[ref]$e)|Out-Null;ConvertTo-Json -InputObject @($e|ForEach-Object{$_.Message}) -Compress`);
      assert.deepEqual(parsed, [], step.id);
    }
    const sourceStep = scenario.steps.find(step => step.id === 'scm-source');
    const protectedScript=windowsServiceProtectedCopyScript({root:scenario.resources.root,compiledExecutable:scenario.resources.compiledExecutable,executable:scenario.resources.executable,expectedSha256:'a'.repeat(64)});
    assert.deepEqual(await windowsObservation(`$t=$null;$e=$null;[Management.Automation.Language.Parser]::ParseInput(${psLiteral(protectedScript)},[ref]$t,[ref]$e)|Out-Null;ConvertTo-Json -InputObject @($e|ForEach-Object{$_.Message}) -Compress`),[],'Protected-copy fixture must parse without executing elevated mutations');
    await mkdir(scenario.resources.buildRoot);
    await writeFile(sourceStep.args.path, sourceStep.args.content);
    const result = await windowsObservation(windowsServiceCompileScript(sourceStep.args.path, scenario.resources.compiledExecutable));
    assert.match(result.sha256, /^[0-9a-f]{64}$/);
    const bytes = await readFile(scenario.resources.compiledExecutable);
    assert.equal(bytes.subarray(0, 2).toString('ascii'), 'MZ');
    const absent = await windowsObservation(`ConvertTo-Json -InputObject @(Get-CimInstance Win32_Service -Filter ${psLiteral(`Name='${scenario.resources.serviceName}'`)}) -Compress`);
    assert.deepEqual(absent, [], 'Compiler smoke must not register or start any service');
  } finally {
    assert.equal((await realpath(fixture)).toLowerCase(), fixture.toLowerCase());
    assert(fixture.startsWith(`${await realpath(tmpdir())}\\nomifun-global-os-product-`));
    await rm(fixture, { recursive: true });
  }
});
