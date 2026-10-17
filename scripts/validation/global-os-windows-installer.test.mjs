import { expect, test } from 'bun:test';
import { randomUUID } from 'node:crypto';
import { canonicalInstallerCases, canonicalInstallerExecutionScript, canonicalUninstallerRawArguments, installerExecutionScript, installerInventoryScript, observeInstallerSignatures, parseRenderedAcceptanceInstaller, validateCanonicalInstallerReceipt, validateInstallerAcceptanceConfig, validateInstallerFixtureClose } from './global-os-windows-installer.mjs';
import { powershell, psLiteral, windowsObservation } from './global-os-windows-cases.mjs';
import { validateWindowsExitResult, windowsCloseProcessIdentity } from './global-os-windows-process-exit-observer.mjs';

const fixture = () => {
  const nonce = randomUUID().replaceAll('-', '');
  return { productName: `NomiFun OS Acceptance ${nonce}`, identifier: `com.nomifun.desktop.acceptance.${nonce}`, mainBinaryName: `nomifun-os-acceptance-${nonce}`, plugins: { 'deep-link': { desktop: { schemes: [`nomifun-gos-${nonce}`] } } } };
};
const rendered = config => [
  `!define PRODUCTNAME "${config.productName}"`, `!define BUNDLEID "${config.identifier}"`, `!define MAINBINARYNAME "${config.mainBinaryName}"`, '!define INSTALLMODE "currentUser"', '!define STARTMENUFOLDER ""', '!define MANUFACTURER "NomiFun"', '!define VERSION "0.8.12"', `!define MAINBINARYSRCPATH "C:\\build\\${config.mainBinaryName}.exe"`, '!define OUTFILE "C:\\build\\fixture-setup.exe"',
  'RequestExecutionLevel user', 'StrCpy $INSTDIR "$LOCALAPPDATA\\Programs\\${PRODUCTNAME}"', '!insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"', `WriteRegStr SHCTX "Software\\Classes\\\\${config.plugins['deep-link'].desktop.schemes[0]}" "URL Protocol" ""`,
  ...['LICENSE', 'NOTICE', 'component-notices\\npm-10.9.4-LICENSE.txt'].map(target => `File /a "/oname=${target}" "C:\\source\\notice.txt"`),
].join('\n');

test('installer fixture cannot share a product, binary, protocol or identifier with the daily App', () => {
  const config = fixture(); expect(validateInstallerAcceptanceConfig(config).protocol).toBe(config.plugins['deep-link'].desktop.schemes[0]);
  for (const alter of [c => c.productName = 'NomiFun', c => c.mainBinaryName = 'nomifun-desktop', c => c.identifier = 'com.nomifun.desktop', c => c.plugins['deep-link'].desktop.schemes = ['nomifun'], c => c.bundle = { windows: { nsis: { installMode: 'perMachine' } } }]) {
    const changed = structuredClone(config); alter(changed); expect(() => validateInstallerAcceptanceConfig(changed)).toThrow();
  }
});
test('rendered NSIS checks actual variant identity, all notices, current-user paths and data preservation', () => {
  const config = fixture(), source = rendered(config); expect(parseRenderedAcceptanceInstaller(source.replaceAll('\n', '\r\n'), config).resources).toHaveLength(3);
  for (const changed of [source.replace(config.mainBinaryName, 'nomifun-desktop'), source.replace('currentUser', 'perMachine'), source.replace('LICENSE', 'missing-license'), source + '\nRmDir /r "$LOCALAPPDATA\\NomiFun"', source + '\n{{unrendered}}', source.replace('/oname=NOTICE', '/oname=..\\NOTICE')]) expect(() => parseRenderedAcceptanceInstaller(changed, config)).toThrow();
});
test('native installer script contains only silent install and no run, remap, kill or app-data deletion', () => {
  const script = installerExecutionScript(String.raw`C:\fixture with spaces\fixture-setup.exe`);
  expect(script).toContain("$start.Arguments='/S'"); expect(script).toContain('$start.UseShellExecute=$false'); expect(script).toContain('NomiFunInstallerToken]::Elevated($handle)');
  for (const forbidden of ['/R', '/D=', 'Kill(', 'Stop-Process', 'Remove-Item', '-Verb RunAs']) expect(script).not.toContain(forbidden);
});

test('canonical uninstaller uses the exact unique root as the last unquoted NSIS raw tail', () => {
  const config=fixture(), root=`C:\\Users\\Fixture Owner\\AppData\\Local\\Programs\\${config.productName}`;
  const raw=canonicalUninstallerRawArguments(root,config);
  expect(raw).toBe(`/S _?=${root}`); expect(raw).not.toContain('"');
  for(const target of ['C:\\Windows',root+'\\other',`C:\\bad"name\\${config.productName}`]) expect(()=>canonicalUninstallerRawArguments(target,config)).toThrow();
  const script=canonicalInstallerExecutionScript({phase:'uninstall',path:'C:\\private fixture\\copy.exe',sha256:'a'.repeat(64),rawArguments:raw});
  expect(script).toContain('$start.UseShellExecute=$false'); expect(script).toContain("$start.Arguments='/S _?=");
  expect(script).toContain('$p.WaitForExit()'); expect(script).toContain('[Console]::Out.Flush()');
  for(const forbidden of ['Stop-Process','Kill(', '-Verb RunAs','ExecutionPolicy','Remove-Item','ASFW_ANY']) expect(script).not.toContain(forbidden);
});

test('canonical installer requires a complete native receipt and original supervisor reap, never unknown replay', () => {
  const expected={phase:'install',path:'C:\\private fixture\\setup.exe',sha256:'a'.repeat(64),rawArguments:'/S'};
  const receipt={schema:'nomifun.canonical-installer-process.v1',...expected,raw_arguments:expected.rawArguments,pid:123,elevated:false,actual_exit:true,exit_code:0};
  const value={state:'exited',exit_code:0,cleanup:{reaped:true,errors:[]},output:{text:JSON.stringify(receipt)}};
  expect(validateCanonicalInstallerReceipt(value,expected).pid).toBe(123);
  for(const mutate of [v=>v.state='running',v=>v.state='unknown',v=>v.exit_code=74,v=>v.cleanup.reaped=false,v=>v.output.text='',v=>v.output.text=JSON.stringify({...receipt,actual_exit:false}),v=>v.output.text=JSON.stringify({...receipt,elevated:true}),v=>v.output.text=JSON.stringify({...receipt,raw_arguments:'/S /R'})]){
    const changed=structuredClone(value);mutate(changed);expect(()=>validateCanonicalInstallerReceipt(changed,expected)).toThrow();
  }
});

test('bootstrap uninstall requires the original retained App exit and the observer to finish successfully', () => {
  const fixtureReport = String.raw`C:\private fixture\ready.json`, creation = '134366923802594258';
  const app = { app_pid: 123, actual_app_identity: { native_process: { pid: 123, state: 'running', creation_filetime: creation } } };
  const close = { schema: 'nomifun.windows-global-os-fixture-close.v1', status: 'exited', path: 'existing_tauri_process_exit',
    actual_pid_gone: true, process_exit_proven: true, exit_code: 0, app_pid: app.app_pid, fixture_report: fixtureReport,
    native_process_exit: { schema: 'nomifun.windows-retained-process-exit.v1', stage: 'result', pid: app.app_pid, creation_filetime: creation,
      access: 'QUERY_LIMITED_INFORMATION|SYNCHRONIZE', ready_emitted: true, receipt: { status: 'exited', wait_status: 0, exit_code: 0, raw_os_error: null },
      handle_closed: true, close_raw_os_error: 0, observer_reaped: true, observer_exit_code: 0 } };
  expect(validateInstallerFixtureClose(app, close, fixtureReport).exit_code).toBe(0);
  // A signaled handle with code 23 proves a real exit, but cannot satisfy the
  // installer's stricter successful-shutdown prerequisite.
  const nonzero = structuredClone(close); nonzero.native_process_exit.receipt.exit_code = 23;
  expect(validateWindowsExitResult(nonzero.native_process_exit, windowsCloseProcessIdentity(app)).exit_code).toBe(23);
  expect(() => validateInstallerFixtureClose(app, nonzero, fixtureReport)).toThrow();
  for (const mutate of [
    value => { delete value.native_process_exit; },
    value => { value.native_process_exit.creation_filetime = '134366923802594259'; },
    value => { value.native_process_exit.pid = 456; },
    value => { value.native_process_exit.receipt.status = 'timeout'; value.native_process_exit.receipt.wait_status = 258; value.native_process_exit.receipt.exit_code = null; },
    value => { value.native_process_exit.handle_closed = false; },
    value => { value.native_process_exit.observer_reaped = false; },
    value => { value.native_process_exit.observer_exit_code = 74; },
    value => { value.exit_code = null; },
    value => { value.fixture_report = String.raw`C:\another fixture\ready.json`; },
  ]) {
    const changed = structuredClone(close); mutate(changed);
    expect(() => validateInstallerFixtureClose(app, changed, fixtureReport)).toThrow();
  }
});

test('canonical case factory installs then uninstalls in distinct phases and deletes only owned copies', () => {
  const config=fixture(), root=`C:\\Users\\Fixture Owner\\AppData\\Local\\Programs\\${config.productName}`;
  const callbacks=Object.fromEntries(['verifyInstall','verifyCopy','verifyUninstall','verifyCopiedDeleted','verifySentinel','verifySentinelDeleted'].map(name=>[name,()=>{}]));
  const values={config,workspace:'C:\\private fixture\\workspace',installer:'C:\\build\\setup.exe',installerSha256:'a'.repeat(64),installRoot:root,
    uninstallerCopy:`C:\\private fixture\\canonical-uninstaller-${randomUUID()}.exe`,sentinel:`C:\\private data\\.installer-preservation-${config.productName.split(' ').at(-1)}`,sentinelContent:'fixture\n',getUninstallerSha256:()=> 'b'.repeat(64),...callbacks};
  const plan=canonicalInstallerCases(values);
  expect(plan.phases.map(phase=>phase.role)).toEqual(['installer-install','installer-uninstall']);expect(plan.steps).toHaveLength(6);
  expect(plan.steps.filter(step=>step.name==='delete_path').map(step=>step.args.path)).toEqual([values.uninstallerCopy,values.sentinel]);
  expect(plan.steps.every(step=>step.repeatable!==true)).toBe(true);
  const encoded=plan.steps.find(step=>step.id==='installer-canonical-uninstall').args().args.at(-1);
  const script=Buffer.from(encoded,'base64').toString('utf16le'); expect(script).toContain(`/S _?=${root}`);
  for(const changed of [{...values,installRoot:'C:\\Windows'},{...values,uninstallerCopy:'C:\\outside\\copy.exe'},{...values,sentinel:'C:\\private data\\other'}])expect(()=>canonicalInstallerCases(changed)).toThrow();
});
test.skipIf(process.platform !== 'win32')('actual Windows parser, token probe and registry/shortcut inventory are read-only', async () => {
  const config = validateInstallerAcceptanceConfig(fixture()), script = installerExecutionScript(String.raw`C:\fixture with spaces\fixture-setup.exe`);
  const parsed = await windowsObservation(`$tokens=$null;$errors=$null;[Management.Automation.Language.Parser]::ParseInput(${psLiteral(script)},[ref]$tokens,[ref]$errors)|Out-Null;[pscustomobject]@{errors=@($errors|ForEach-Object{$_.Message})}|ConvertTo-Json -Compress`); expect(parsed.errors).toEqual([]);
  const probe = script.slice(0, script.indexOf('$start=')); const token = await windowsObservation(`${probe}\n[pscustomobject]@{elevated=[NomiFunInstallerToken]::Elevated([Diagnostics.Process]::GetCurrentProcess().Handle)}|ConvertTo-Json -Compress`); expect(token.elevated).toBe(false);
  const inventory = await windowsObservation(installerInventoryScript(config, { manufacturer: 'NomiFun' })); expect(inventory.registry).toHaveLength(12); expect(inventory.registry.every(row => row.data === null)).toBe(true); expect(inventory.registry.every(row => !row.path.includes('${'))).toBe(true); expect(inventory.shortcuts).toHaveLength(2); expect(inventory.shortcuts.every(row => !row.exists)).toBe(true); expect(inventory.image_processes).toEqual([]);
  expect(inventory.autostart).toHaveLength(4); expect(inventory.autostart.filter(row => row.name === config.productName).every(row => !row.present)).toBe(true); expect(inventory.daily.every(row => /^[a-f0-9]{64}$/.test(row.data_sha256) && !Object.hasOwn(row, 'data'))).toBe(true);
  const signatures = await observeInstallerSignatures([powershell]); expect(signatures).toHaveLength(1); expect(signatures[0].status).toBe('Valid'); expect(signatures[0].has_signer).toBe(true);
  const root=`C:\\Users\\Fixture Owner\\AppData\\Local\\Programs\\${config.productName}`;
  const plan=canonicalInstallerCases({config:{productName:config.productName,identifier:config.identifier,mainBinaryName:config.mainBinaryName,plugins:config.plugins},workspace:'C:\\private fixture\\workspace',installer:'C:\\build\\setup.exe',installerSha256:'a'.repeat(64),installRoot:root,
    uninstallerCopy:`C:\\private fixture\\canonical-uninstaller-${randomUUID()}.exe`,sentinel:`C:\\private data\\.installer-preservation-${config.nonce}`,sentinelContent:'fixture\n',getUninstallerSha256:()=> 'b'.repeat(64)});
  for(const step of plan.steps.filter(step=>step.name==='exec_command')){
    const args=typeof step.args==='function'?step.args():step.args, source=Buffer.from(args.args.at(-1),'base64').toString('utf16le');
    const parsed=await windowsObservation(`$tokens=$null;$errors=$null;[Management.Automation.Language.Parser]::ParseInput(${psLiteral(source)},[ref]$tokens,[ref]$errors)|Out-Null;[pscustomobject]@{errors=@($errors|ForEach-Object{$_.Message})}|ConvertTo-Json -Compress`);
    expect(parsed.errors).toEqual([]);
  }
}, 20000);
