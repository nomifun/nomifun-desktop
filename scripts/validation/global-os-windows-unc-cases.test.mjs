// Scenario/protocol tests only; these do not create a share or claim UNC I/O.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { mkdir, realpath, rmdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';
import test from 'node:test';
import { createWindowsUncScenario, ownedUncFixturePath } from './global-os-windows-unc-cases.mjs';
import { powershell, psArguments, psLiteral } from './global-os-windows-cases.mjs';

test('UNC canonical-fixture gate requires verified exact server/share identity', () => {
  const state = { verified: true, shareName: `NomiFun_GlobalOS_${randomUUID()}`, hosts: ['localhost', 'DESKTOP-FIXTURE'] };
  for (const path of [`\\\\localhost\\${state.shareName}`, `\\\\localhost\\${state.shareName}\\file.txt`, `\\\\?\\UNC\\DESKTOP-FIXTURE\\${state.shareName}\\nested\\file.txt`]) assert(ownedUncFixturePath(path, state), path);
  for (const path of [`\\\\foreign\\${state.shareName}\\file.txt`, `\\\\localhost\\${state.shareName}-other\\file.txt`, `\\\\localhost\\${state.shareName}\\..\\other`, `\\\\localhost\\${state.shareName}\\a\\\\b`, `C:\\${state.shareName}\\file.txt`, `\\\\user@localhost\\${state.shareName}\\file.txt`, `\\\\localhost\\${state.shareName}\\bad\0name`]) assert(!ownedUncFixturePath(path, state), path);
  assert(!ownedUncFixturePath(`\\\\localhost\\${state.shareName}\\file.txt`, { ...state, verified: false }));
});

test('UNC scenario preserves unavailable facilities and rejects foreign share before path admission', { skip: process.platform !== 'win32' }, async () => {
  const parent = await realpath(tmpdir()), fixture = join(parent, `nomifun-global-os-product-${randomUUID()}`); await mkdir(fixture);
  try {
    const scenario = await createWindowsUncScenario({ fixture, workspace: join(fixture, 'workspace'), loginIdentity: { administrator: false, name: 'TEST\\fixture', sid: 'S-1-5-21-1-2-3-1000' } });
    assert(scenario.resources.unc_root.startsWith('\\\\localhost\\NomiFun_GlobalOS_'));
    assert.equal(scenario.steps.length, 16);
    assert.equal(scenario.steps.find(step => step.id === 'unc-search').args.path,`${scenario.resources.unc_root}\\`);
    for(const id of ['unc-native-exe-cwd','unc-native-ps1-literal-cwd'])assert.equal(scenario.steps.find(step=>step.id===id).args.cwd,`${scenario.resources.unc_root}\\`);
    assert.equal(scenario.allowsCanonicalFixturePath(scenario.resources.unc_root), false);
    const tool = value => ({ state: 'exited', exit_code: 0, cleanup: { reaped: true }, output: { text: JSON.stringify(value) } });
    await assert.rejects(scenario.steps[0].verify(tool({ service_exists: true, service_status: 'Stopped', commands: [] })), /not running/);
    await assert.rejects(scenario.steps.find(step => step.id === 'unc-create-share').verify(tool({ name: 'foreign', path: scenario.resources.physical_root })), /Expected values to be strictly equal/);
    assert.equal(scenario.allowsCanonicalFixturePath(scenario.resources.unc_root), false);
    const sources = [];
    for (const step of scenario.steps.filter(step => step.name === 'start_process')) {
      const args = typeof step.args === 'function' ? await step.args() : step.args;
      if (args.command !== powershell) continue;
      const source = Buffer.from(args.args.at(-1), 'base64').toString('utf16le'); sources.push(source);
      assert(!/\b(?:Start-Service|Stop-Service|Set-Service|Set-Smb\w+|Set-Net\w+|New-NetFirewallRule|netsh)\b/i.test(source));
      assert(!source.includes('-FullAccess Everyone'));
    }
    for (const step of scenario.cleanupSteps.filter(step => step.name === 'start_process')) sources.push(Buffer.from(step.args.args.at(-1), 'base64').toString('utf16le'));
    const parse = `$sources=ConvertFrom-Json ${psLiteral(JSON.stringify(sources))};$count=0;foreach($source in $sources){$tokens=$null;$errors=$null;[Management.Automation.Language.Parser]::ParseInput($source,[ref]$tokens,[ref]$errors)|Out-Null;if($errors.Count){throw ($errors|ForEach-Object{$_.Message}|Out-String)};$count++};[Console]::Write($count)`;
    const result = await promisify(execFile)(powershell, psArguments(parse), { encoding: 'utf8', windowsHide: true, timeout: 10000 });
    assert.equal(Number(result.stdout.trim()), sources.length);
  } finally { await rmdir(fixture); }
});
