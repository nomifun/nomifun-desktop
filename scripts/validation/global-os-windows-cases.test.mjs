import { expect, test } from 'bun:test';
import { execFile, spawn } from 'node:child_process';
import { mkdir, readFile, realpath, rmdir, writeFile, unlink, link } from 'node:fs/promises';
import { createHash, randomUUID } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join,resolve } from 'node:path';
import { promisify } from 'node:util';
import { assertWindowsFixture, initializeWindowsPrivateEmptyFixture, createWindowsAdministratorScenario, createWindowsFilesScenario, createWindowsSystemScenario, powershell, psArguments, psLiteral, windowsLoginIdentity, readWindowsOwnedFixtureBytes, windowsProcessObservation, windowsProcessGone } from './global-os-windows-cases.mjs';
import { windowsIntegrityObservationScript } from './global-os-windows-cases.mjs';

const nativeTest = test.skipIf(process.platform !== 'win32');
const execute = promisify(execFile);

nativeTest('the independent control fixture ACL is limited to its new empty login-owned root',async()=>{
  const fixture=join(await realpath(tmpdir()),`nomifun-global-os-product-${randomUUID()}`),identity=await windowsLoginIdentity();await mkdir(fixture);
  try{const receipt=await initializeWindowsPrivateEmptyFixture(fixture,identity);expect(receipt).toEqual({owner_sid:identity.sid,acl_private:true});
    const marker=join(fixture,'guard.txt');await writeFile(marker,'guard',{flag:'wx'});
    try{await expect(initializeWindowsPrivateEmptyFixture(fixture,identity)).rejects.toThrow('empty fixture');expect(await readFile(marker,'utf8')).toBe('guard');}finally{await unlink(marker);}
  }finally{await rmdir(fixture);}
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
    const scripts = [];
    for (const step of [...ordinary.steps, ...admin.steps, ...admin.cleanupSteps, ...system.steps, ...system.cleanupSteps]) {
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
