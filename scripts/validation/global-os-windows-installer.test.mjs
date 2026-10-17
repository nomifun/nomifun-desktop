import { expect, test } from 'bun:test';
import { createHash, randomUUID } from 'node:crypto';
import { canonicalInstallerCases, canonicalInstallerExecutionScript, canonicalUninstallerRawArguments, installerExecutionScript, installerInventoryScript, locateRenderedAcceptanceInstallerOutput, observeInstallerSignatures, parseRenderedAcceptanceInstaller, predictUnsignedNsisExecutable, validateCanonicalInstallerReceipt, validateInstallerAcceptanceConfig, validateInstallerFixtureClose } from './global-os-windows-installer.mjs';
import { powershell, psLiteral, windowsObservation } from './global-os-windows-cases.mjs';
import { validateWindowsExitResult, windowsCloseProcessIdentity } from './global-os-windows-process-exit-observer.mjs';

const fixture = () => {
  const nonce = randomUUID().replaceAll('-', '');
  return { productName: `NomiFun OS Acceptance ${nonce}`, identifier: `com.nomifun.desktop.acceptance.${nonce}`, mainBinaryName: `nomifun-os-acceptance-${nonce}`, plugins: { 'deep-link': { desktop: { schemes: [`nomifun-gos-${nonce}`] } } } };
};
const rendered = config => [
  `!define PRODUCTNAME "${config.productName}"`, `!define BUNDLEID "${config.identifier}"`, `!define MAINBINARYNAME "${config.mainBinaryName}"`, '!define INSTALLMODE "currentUser"', '!define STARTMENUFOLDER ""', '!define MANUFACTURER "NomiFun"', '!define VERSION "0.8.12"', '!define ARCH "x64"', `!define MAINBINARYSRCPATH "C:\\build\\${config.mainBinaryName}.exe"`, '!define OUTFILE "C:\\build\\fixture-setup.exe"',
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

test('an absolute rendered NSIS output retains its exact supplied path without assuming a bundler layout', () => {
  const config=fixture(),parsed=parseRenderedAcceptanceInstaller(rendered(config),config);
  for(const outfile of [String.raw`C:\Explicit output\Acceptance-Setup.EXE`,String.raw`\\server\share\Explicit output\Acceptance-Setup.EXE`]){
    const result=locateRenderedAcceptanceInstallerOutput({config,rendered:{...parsed,outfile},renderedNsi:String.raw`C:\other location\explicit-installer.nsi`});
    expect(result).toEqual({raw_outfile:outfile,outfile,outfile_resolution:{rule:'absolute_rendered_outfile'}});
  }
  for(const outfile of [String.raw`C:\build\..\Acceptance-Setup.EXE`,String.raw`\\server\share\.\Acceptance-Setup.EXE`])expect(()=>locateRenderedAcceptanceInstallerOutput({config,rendered:{...parsed,outfile}})).toThrow();
});

function relativeInstallerOutputFixture(){
  const config=fixture(),release=String.raw`C:\build workspace\target\x86_64-pc-windows-msvc\release`;
  return{config,rendered:{...parseRenderedAcceptanceInstaller(rendered(config),config),outfile:'nsis-output.exe',sourceExe:`${release}\\${config.mainBinaryName}.exe`},renderedNsi:`${release}\\nsis\\x64\\installer.nsi`,release};
}

test('the actual Tauri temporary output maps only to its same-release unique final NSIS bundle', () => {
  const input=relativeInstallerOutputFixture(),before=structuredClone(input),actual=locateRenderedAcceptanceInstallerOutput(input);
  expect(actual.raw_outfile).toBe('nsis-output.exe');
  expect(actual.outfile).toBe(`${input.release}\\bundle\\nsis\\${input.config.productName}_0.8.12_x64-setup.exe`);
  expect(actual.outfile_resolution.rule).toBe('tauri_nsis_project_bundle_output.v1');
  expect(actual.outfile_resolution.nsis_working_directory).toBe(`${input.release}\\nsis\\x64`);
  expect(input).toEqual(before);
  const unc=structuredClone(input);unc.release=String.raw`\\build-host\private-share\repo\target\x86_64-pc-windows-msvc\release`;
  unc.rendered.sourceExe=`${unc.release}\\${unc.config.mainBinaryName}.exe`;unc.renderedNsi=`${unc.release}\\nsis\\x64\\installer.nsi`;
  expect(locateRenderedAcceptanceInstallerOutput(unc).outfile).toBe(`${unc.release}\\bundle\\nsis\\${unc.config.productName}_0.8.12_x64-setup.exe`);
});

test('relative output resolution rejects arbitrary paths traversal wrong architecture source version or variant', () => {
  for(const outfile of ['setup.exe','.\\nsis-output.exe','..\\nsis-output.exe','sub\\nsis-output.exe','NSIS-output.exe','nsis-output.exe ','C:nsis-output.exe','\\nsis-output.exe','/nsis-output.exe','nsis-output.exe\0']){
    const input=relativeInstallerOutputFixture();input.rendered.outfile=outfile;expect(()=>locateRenderedAcceptanceInstallerOutput(input)).toThrow();
  }
  for(const change of [
    value=>{value.renderedNsi='nsis\\x64\\installer.nsi';},
    value=>{value.renderedNsi=value.renderedNsi.replace('installer.nsi','other.nsi');},
    value=>{value.renderedNsi=value.renderedNsi.replace('\\nsis\\x64\\','\\other\\x64\\');},
    value=>{value.renderedNsi=value.renderedNsi.replace('\\nsis\\x64\\','\\nsis\\x86\\');},
    value=>{value.renderedNsi=value.renderedNsi.replace('\\release\\','\\debug\\');},
    value=>{value.renderedNsi=value.renderedNsi.replace('\\nsis\\','\\unused\\..\\nsis\\');},
    value=>{value.renderedNsi=value.renderedNsi.replace('\\nsis\\','\\.\\nsis\\');},
    value=>{value.rendered.sourceExe=`C:\\other\\release\\${value.config.mainBinaryName}.exe`;},
    value=>{value.rendered.sourceExe=value.rendered.sourceExe.replace('\\release\\','\\release\\unused\\..\\');},
    value=>{value.rendered.sourceExe=value.rendered.sourceExe.replace(value.config.mainBinaryName,'nomifun-desktop');},
    value=>{value.rendered.arch='arm64';},value=>{value.rendered.arch=undefined;},
    value=>{value.rendered.version='0.8.12/../other';},value=>{value.rendered.version='0.8.12\\other';},value=>{value.rendered.version='0.8.12_x64-other';},
    value=>{value.config.productName='NomiFun';},value=>{value.config.identifier='com.nomifun.desktop';},
  ]){const input=relativeInstallerOutputFixture();change(input);expect(()=>locateRenderedAcceptanceInstallerOutput(input)).toThrow();}
});

const executableHash = bytes => createHash('sha256').update(bytes).digest('hex');
const unsignedSignature = { status: 'NotSigned', has_signer: false };
test('unsigned NSIS prediction hashes the complete exact first-token replacement and preserves the original source', () => {
  const config=fixture(),prefix=Buffer.from('MZ\0native header\0'),from=Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK'),to=Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS'),tail=Buffer.from('\0unchanged native tail\0');
  const sourceBytes=Buffer.concat([prefix,from,tail]),before=Buffer.from(sourceBytes),expected=Buffer.concat([prefix,to,tail]);
  const prediction=predictUnsignedNsisExecutable({config,sourceBytes,sourceSignature:unsignedSignature});
  expect(prediction.source_executable_sha256).toBe(executableHash(sourceBytes));
  expect(prediction.expected_packaged_executable_sha256).toBe(executableHash(expected));
  expect(prediction.expected_packaged_executable_sha256).not.toBe(prediction.source_executable_sha256);
  expect(sourceBytes.equals(before)).toBe(true);expect(prediction.bundle_type_patch.offset).toBe(prefix.length);
  expect(prediction.bundle_type_patch.length).toBe(27);expect(prediction.bundle_type_patch.first_match).toBe(true);expect(prediction.bundle_type_patch.unsigned_only).toBe(true);
  // Any other PE byte, omitted patch or different marker produces a different
  // whole-file digest; no range is masked or ignored during installation.
  for(const mutate of [bytes=>{bytes[2]^=1;},bytes=>{bytes[bytes.length-1]^=1;},bytes=>{bytes[prefix.length+24]='U'.charCodeAt(0);}]){
    const changed=Buffer.from(expected);mutate(changed);expect(executableHash(changed)).not.toBe(prediction.expected_packaged_executable_sha256);
  }
});

test('multiple unknown bundle markers change only the first exact match and do not mask later token changes', () => {
  const config=fixture(),prefix=Buffer.from('MZ first '),from=Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK'),to=Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS'),middle=Buffer.from(' second '),tail=Buffer.from(' tail');
  const sourceBytes=Buffer.concat([prefix,from,middle,from,tail]);
  const prediction=predictUnsignedNsisExecutable({config,sourceBytes,sourceSignature:unsignedSignature});
  expect(prediction.expected_packaged_executable_sha256).toBe(executableHash(Buffer.concat([prefix,to,middle,from,tail])));
  expect(prediction.expected_packaged_executable_sha256).not.toBe(executableHash(Buffer.concat([prefix,to,middle,to,tail])));
  expect(prediction.bundle_type_patch.offset).toBe(prefix.length);
});

test('bundle prediction rejects missing or wrong source tokens signed status and non-acceptance binaries', () => {
  const config=fixture(),sourceBytes=Buffer.from('MZ __TAURI_BUNDLE_TYPE_VAR_UNK tail');
  for(const bytes of [Buffer.from('MZ without a marker'),Buffer.from('MZ __TAURI_BUNDLE_TYPE_VAR_unk'),Buffer.from('MZ __TAURI_BUNDLE_TYPE_VAR_NSS'),Buffer.from('text __TAURI_BUNDLE_TYPE_VAR_UNK')])expect(()=>predictUnsignedNsisExecutable({config,sourceBytes:bytes,sourceSignature:unsignedSignature})).toThrow();
  for(const sourceSignature of [undefined,{status:'Valid',has_signer:true},{status:'UnknownError',has_signer:false},{status:'HashMismatch',has_signer:true},{status:'NotSigned',has_signer:true},{status:'NotSigned'}])expect(()=>predictUnsignedNsisExecutable({config,sourceBytes,sourceSignature})).toThrow();
  expect(()=>predictUnsignedNsisExecutable({config:{...config,productName:'NomiFun'},sourceBytes,sourceSignature:unsignedSignature})).toThrow();
  expect(()=>predictUnsignedNsisExecutable({config,sourceBytes:new Uint8Array(sourceBytes),sourceSignature:unsignedSignature})).toThrow();
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
