#!/usr/bin/env node
// Native NSIS acceptance for a single UUID-named current-user build. This
// driver never removes leftovers, signals applications or repairs registration.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { lstat, mkdir, open, readFile, realpath, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, isAbsolute, join, resolve, win32 } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { assertWindowsFixture, assertWindowsPrivatePath, powershell, psArguments, psLiteral, windowsLoginIdentity, windowsObservation, windowsProcessGone } from './global-os-windows-cases.mjs';
import { validateWindowsExitResult, windowsCloseProcessIdentity } from './global-os-windows-process-exit-observer.mjs';

const execute = promisify(execFile);
const repository = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const samePath = (a, b) => resolve(a).toLowerCase() === resolve(b).toLowerCase();
const absent = path => lstat(path).then(() => false, error => { if (error.code === 'ENOENT') return true; throw error; });
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
async function fileDigest(path) {
  const before = await lstat(path); assert(before.isFile() && !before.isSymbolicLink(), 'Expected a regular file');
  const hash = createHash('sha256'); for await (const bytes of createReadStream(path)) hash.update(bytes);
  const after = await lstat(path); assert.equal(after.ino, before.ino); assert.equal(after.dev, before.dev); assert.equal(after.size, before.size); assert.equal(after.mtimeMs, before.mtimeMs);
  return hash.digest('hex');
}
async function stableText(path, maximum = 1024 * 1024) {
  const before = await lstat(path); assert(before.isFile() && !before.isSymbolicLink() && before.size <= maximum);
  const bytes = await readFile(path); assert.equal(bytes.length, before.size); const after = await lstat(path);
  assert.equal(after.ino, before.ino); assert.equal(after.dev, before.dev); assert.equal(after.mtimeMs, before.mtimeMs);
  return { text: new TextDecoder('utf-8', { fatal: true }).decode(bytes), sha256: digest(bytes) };
}
export function validateInstallerAcceptanceConfig(config) {
  assert.deepEqual(Object.keys(config).sort(), ['identifier', 'mainBinaryName', 'plugins', 'productName']);
  const nonce = /^NomiFun OS Acceptance ([0-9a-f]{32})$/.exec(config.productName)?.[1];
  assert(nonce && nonce[12] === '4' && /^[89ab]$/.test(nonce[16]), 'Only a fresh UUID-v4 acceptance identity is allowed');
  assert.equal(config.identifier, `com.nomifun.desktop.acceptance.${nonce}`);
  assert.equal(config.mainBinaryName, `nomifun-os-acceptance-${nonce}`);
  assert.deepEqual(config.plugins, { 'deep-link': { desktop: { schemes: [`nomifun-gos-${nonce}`] } } });
  return { ...config, nonce, protocol: `nomifun-gos-${nonce}` };
}
export function parseRenderedAcceptanceInstaller(text, config) {
  text = text.replaceAll('\r\n', '\n');
  config = validateInstallerAcceptanceConfig(config);
  assert(!text.includes('{{'), 'NSIS must be actually rendered by Tauri');
  const definition = name => { const matches = [...text.matchAll(new RegExp(`^!define ${name} "([^"\\r\\n]*)"$`, 'gm'))]; assert.equal(matches.length, 1, `Expected one NSIS ${name}`); return matches[0][1]; };
  assert.equal(definition('PRODUCTNAME'), config.productName); assert.equal(definition('BUNDLEID'), config.identifier);
  assert.equal(definition('MAINBINARYNAME'), config.mainBinaryName); assert.equal(definition('INSTALLMODE'), 'currentUser');
  assert.equal(definition('STARTMENUFOLDER'), '', 'This fixture accepts the unmodified default shortcut layout');
  assert(text.includes('RequestExecutionLevel user')); assert(text.includes('"$LOCALAPPDATA\\Programs\\${PRODUCTNAME}"'));
  assert(text.includes('!insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"'));
  for (const forbidden of ['DeleteAppDataCheckbox', '$(deleteAppData)', 'RmDir /r "$LOCALAPPDATA', 'RmDir /r "$APPDATA']) assert(!text.includes(forbidden), `Forbidden data deletion: ${forbidden}`);
  const protocols = [...text.matchAll(/^\s*WriteRegStr SHCTX "Software\\Classes\\+([^"\\]+)" "URL Protocol" ""\s*$/gm)].map(match => match[1]);
  assert.deepEqual(protocols, [config.protocol], 'Only the unique acceptance protocol can be registered');
  const resources = [...text.matchAll(/^\s*File \/a "\/oname=([^"\r\n]+)" "([^"\r\n]+)"\s*$/gm)].map(match => ({ target: match[1].replaceAll('\\', '/'), source: match[2] }));
  for (const resource of resources) assert(!resource.target.startsWith('/') && !resource.target.includes(':') && resource.target.split('/').every(part => part && part !== '.' && part !== '..') && !resource.target.includes('$'));
  for (const required of ['LICENSE', 'NOTICE', 'component-notices/npm-10.9.4-LICENSE.txt']) assert.equal(resources.filter(entry => entry.target === required).length, 1, `Missing full bundled ${required}`);
  const sourceExe = definition('MAINBINARYSRCPATH'); assert(win32.isAbsolute(sourceExe)); assert.equal(win32.basename(sourceExe).toLowerCase(), `${config.mainBinaryName}.exe`);
  return { sourceExe, resources, manufacturer: definition('MANUFACTURER'), version: definition('VERSION'), arch: definition('ARCH'), outfile: definition('OUTFILE') };
}
export function locateRenderedAcceptanceInstallerOutput({ config, rendered, renderedNsi }) {
  config = validateInstallerAcceptanceConfig(config);
  const raw_outfile = rendered.outfile;
  const fullyAbsolute = path => typeof path === 'string' && win32.isAbsolute(path) && !['\\', '/'].includes(win32.parse(path).root) && !path.includes('\0');
  const withoutTraversal = path => !path.split(/[\\/]/).some(part => part === '.' || part === '..');
  if (fullyAbsolute(raw_outfile)) {
    assert(withoutTraversal(raw_outfile), 'The absolute rendered output must not contain path traversal aliases');
    return { raw_outfile, outfile: raw_outfile, outfile_resolution: { rule: 'absolute_rendered_outfile' } };
  }
  assert.equal(raw_outfile, 'nsis-output.exe', 'Only the exact Tauri NSIS temporary output name can be resolved relatively');
  const cleanAbsolute = path => fullyAbsolute(path) && withoutTraversal(path);
  assert(cleanAbsolute(renderedNsi) && cleanAbsolute(rendered.sourceExe), 'The rendered NSIS and source executable must use explicit absolute paths without parent aliases');
  const nsi = win32.normalize(renderedNsi), workingDirectory = win32.dirname(nsi);
  assert.equal(win32.basename(nsi), 'installer.nsi'); assert.equal(win32.basename(workingDirectory), 'x64');
  const nsisDirectory = win32.dirname(workingDirectory), releaseDirectory = win32.dirname(nsisDirectory);
  assert.equal(win32.basename(nsisDirectory), 'nsis'); assert.equal(win32.basename(releaseDirectory), 'release');
  assert.equal(rendered.arch, 'x64', 'This acceptance locator requires the actual x64 NSIS layout');
  assert.match(rendered.version, /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?(?:\+[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/, 'The rendered version must be a filename-safe package version');
  const source = win32.normalize(rendered.sourceExe);
  assert.equal(win32.dirname(source).toLowerCase(), releaseDirectory.toLowerCase(), 'The source executable must belong to this exact NSIS release directory');
  assert.equal(win32.basename(source).toLowerCase(), `${config.mainBinaryName}.exe`);
  return { raw_outfile, outfile: win32.join(releaseDirectory, 'bundle', 'nsis', `${config.productName}_${rendered.version}_${rendered.arch}-setup.exe`),
    outfile_resolution: { rule: 'tauri_nsis_project_bundle_output.v1', nsis_working_directory: workingDirectory,
      source: 'https://github.com/tauri-apps/tauri/blob/499df79be65ef8c0670abc0207cd9e37b55d8491/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs',
      output_folder_source: 'https://github.com/tauri-apps/tauri/blob/499df79be65ef8c0670abc0207cd9e37b55d8491/crates/tauri-bundler/src/bundle/windows/util.rs' } };
}
export function predictUnsignedNsisExecutable({ config, sourceBytes, sourceSignature }) {
  validateInstallerAcceptanceConfig(config);
  assert.equal(sourceSignature?.status, 'NotSigned', 'This exact bundle-type prediction supports only an unsigned acceptance executable');
  assert.equal(sourceSignature.has_signer, false, 'Signed executables require their own packaging verification');
  assert(Buffer.isBuffer(sourceBytes) && sourceBytes.subarray(0, 2).equals(Buffer.from('MZ')), 'Expected the complete native acceptance executable bytes');
  const from = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK', 'ascii'), to = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS', 'ascii');
  const offset = sourceBytes.indexOf(from); assert(offset >= 0, 'The original executable has no exact unpatched Tauri bundle-type token');
  const patched = createHash('sha256').update(sourceBytes.subarray(0, offset)).update(to).update(sourceBytes.subarray(offset + from.length)).digest('hex');
  return { source_executable_sha256: digest(sourceBytes), expected_packaged_executable_sha256: patched,
    bundle_type_patch: { offset, length: from.length, from: from.toString('ascii'), to: to.toString('ascii'), first_match: true, unsigned_only: true,
      source: 'https://github.com/tauri-apps/tauri/blob/499df79be65ef8c0670abc0207cd9e37b55d8491/crates/tauri-bundler/src/bundle.rs' } };
}
async function unsignedNsisExecutableExpectation(sourceExe, config) {
  const signatures = await observeInstallerSignatures([sourceExe]); assert.equal(signatures.length, 1); assert(samePath(signatures[0].path, sourceExe));
  const before = await lstat(sourceExe); assert(before.isFile() && !before.isSymbolicLink(), 'Expected the original regular executable');
  const file = await open(sourceExe, 'r'); let bytes;
  try {
    const opened = await file.stat(); assert.equal(opened.ino, before.ino); assert.equal(opened.dev, before.dev);
    bytes = await file.readFile(); const after = await file.stat(); assert.equal(bytes.length, before.size);
    assert.equal(after.size, before.size); assert.equal(after.mtimeMs, before.mtimeMs);
  } finally { await file.close(); }
  const expected = predictUnsignedNsisExecutable({ config, sourceBytes: bytes, sourceSignature: signatures[0] });
  assert.equal(await fileDigest(sourceExe), expected.source_executable_sha256, 'The original source executable changed during packaging prediction');
  return { ...expected, source_authenticode: signatures[0] };
}
const elevationProbe = `Add-Type -TypeDefinition @'
using System; using System.ComponentModel; using System.Runtime.InteropServices;
public static class NomiFunInstallerToken {
  [DllImport("advapi32.dll", SetLastError=true)] static extern bool OpenProcessToken(IntPtr process, uint rights, out IntPtr token);
  [DllImport("advapi32.dll", SetLastError=true)] static extern bool GetTokenInformation(IntPtr token, int kind, out int value, int size, out int needed);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr value);
  public static bool Elevated(IntPtr process) { IntPtr token; if(!OpenProcessToken(process,8,out token)) throw new Win32Exception(Marshal.GetLastWin32Error()); try { int value, needed; if(!GetTokenInformation(token,20,out value,4,out needed)) throw new Win32Exception(Marshal.GetLastWin32Error()); return value!=0; } finally { CloseHandle(token); } }
}
'@`;
export const installerExecutionScript = path => `${elevationProbe}
$start=New-Object Diagnostics.ProcessStartInfo; $start.FileName=${psLiteral(path)}; $start.Arguments='/S'; $start.WorkingDirectory=${psLiteral(dirname(path))}; $start.UseShellExecute=$false; $start.CreateNoWindow=$true;
$p=[Diagnostics.Process]::Start($start); $receipt=[ordered]@{pid=$p.Id;elevated=$null;actual_exit=$false;exit_code=$null;arguments=@('/S');process_signals_sent=0}; try {$handle=$p.Handle;$receipt.elevated=[NomiFunInstallerToken]::Elevated($handle);$receipt.actual_exit=$p.WaitForExit(180000);if($receipt.actual_exit){$receipt.exit_code=$p.ExitCode}}catch{$receipt.error=$_.Exception.Message};[pscustomobject]$receipt|ConvertTo-Json -Compress`;
async function runNativeInstaller(path) {
  const result = await execute(powershell, psArguments(installerExecutionScript(path)), { encoding: 'utf8', windowsHide: true, maxBuffer: 1024 * 1024 });
  return JSON.parse(result.stdout.replace(/^\uFEFF/, '').trim());
}
function verifyNativeExit(receipt) { assert.equal(receipt.elevated, false, 'Actual installer token must remain ordinary'); assert.equal(receipt.actual_exit, true, 'Installer result unknown: retain the fixture and do not replay'); assert.equal(receipt.exit_code, 0); }
export function installerInventoryScript(config, rendered) {
  const paths = ['Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\' + config.productName, `Software\\${rendered.manufacturer}\\${config.productName}`, `Software\\Classes\\${config.protocol}`];
  const daily = [String.raw`Software\Microsoft\Windows\CurrentVersion\Uninstall\NomiFun`, `Software\\${rendered.manufacturer}\\NomiFun`, String.raw`Software\Classes\nomifun`];
  return `function Snapshot-Hash($value){$json=if($null -eq $value){'null'}else{ConvertTo-Json -InputObject $value -Compress -Depth 8};$hash=[Security.Cryptography.SHA256]::Create();try{return ([BitConverter]::ToString($hash.ComputeHash([Text.Encoding]::UTF8.GetBytes($json)))).Replace('-','').ToLowerInvariant()}finally{$hash.Dispose()}};function Read-Key($root,$view,$path){$base=[Microsoft.Win32.RegistryKey]::OpenBaseKey($root,$view);try{$key=$base.OpenSubKey($path);if($null -eq $key){return $null};try{$values=[ordered]@{};foreach($name in ($key.GetValueNames()|Sort-Object)){$values[$name]=$key.GetValue($name)};$command=$key.OpenSubKey('shell\\open\\command');$openCommand=$null;if($null -ne $command){try{$openCommand=$command.GetValue('')}finally{$command.Dispose()}};return [pscustomobject]@{values=$values;children=@($key.GetSubKeyNames()|Sort-Object);open_command=$openCommand}}finally{$key.Dispose()}}finally{$base.Dispose()}};
$rows=@();foreach($root in @([Microsoft.Win32.RegistryHive]::CurrentUser,[Microsoft.Win32.RegistryHive]::LocalMachine)){foreach($view in @([Microsoft.Win32.RegistryView]::Registry64,[Microsoft.Win32.RegistryView]::Registry32)){foreach($path in @(${paths.map(psLiteral).join(',')})){$rows+= [pscustomobject]@{root=[string]$root;view=[string]$view;path=$path;data=(Read-Key $root $view $path)}}}};
$daily=@();foreach($root in @([Microsoft.Win32.RegistryHive]::CurrentUser,[Microsoft.Win32.RegistryHive]::LocalMachine)){foreach($view in @([Microsoft.Win32.RegistryView]::Registry64,[Microsoft.Win32.RegistryView]::Registry32)){foreach($path in @(${daily.map(psLiteral).join(',')})){$daily += [pscustomobject]@{root=[string]$root;view=[string]$view;path=$path;data_sha256=(Snapshot-Hash (Read-Key $root $view $path))}}}};
$run=@();foreach($view in @([Microsoft.Win32.RegistryView]::Registry64,[Microsoft.Win32.RegistryView]::Registry32)){$base=[Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser,$view);try{$key=$base.OpenSubKey('Software\\Microsoft\\Windows\\CurrentVersion\\Run');try{foreach($name in @(${psLiteral(config.productName)},'NomiFun')){$value=$null;if($null -ne $key){$value=$key.GetValue($name)};$run += [pscustomobject]@{view=[string]$view;name=$name;present=($null -ne $value);value_sha256=(Snapshot-Hash $value)}}}finally{if($null -ne $key){$key.Dispose()}}}finally{$base.Dispose()}};
$programs=[Environment]::GetFolderPath('Programs');$desktop=[Environment]::GetFolderPath('DesktopDirectory');$shortcuts=@();$shell=New-Object -ComObject WScript.Shell;foreach($path in @((Join-Path $programs ${psLiteral(`${config.productName}.lnk`)}),(Join-Path $desktop ${psLiteral(`${config.productName}.lnk`)}))){$exists=[IO.File]::Exists($path);$target=$null;if($exists){$target=$shell.CreateShortcut($path).TargetPath};$shortcuts += [pscustomobject]@{path=$path;exists=$exists;target=$target}};
[pscustomobject]@{registry=$rows;daily=$daily;autostart=$run;shortcuts=$shortcuts;local_appdata=[Environment]::GetFolderPath('LocalApplicationData');image_processes=@(Get-Process -Name ${psLiteral(config.mainBinaryName)} -ErrorAction SilentlyContinue|ForEach-Object{$_.Id})}|ConvertTo-Json -Compress -Depth 9`;
}
const inventory = (config, rendered) => windowsObservation(installerInventoryScript(config, rendered));
export async function observeInstallerSignatures(paths) {
  assert(paths.length > 0 && paths.every(isAbsolute));
  return windowsObservation(`Import-Module (Join-Path $PSHOME 'Modules\\Microsoft.PowerShell.Security\\Microsoft.PowerShell.Security.psd1') -ErrorAction Stop;$command=Get-Command 'Microsoft.PowerShell.Security\\Get-AuthenticodeSignature' -ErrorAction Stop;ConvertTo-Json -InputObject @(@(${paths.map(psLiteral).join(',')})|ForEach-Object{$s=& $command -LiteralPath $_;[pscustomobject]@{path=$_;status=[string]$s.Status;has_signer=($null -ne $s.SignerCertificate)}}) -Compress`);
}
function validateRegistered(inventory, config, installRoot, exe, manufacturer) {
  const current = inventory.registry.filter(row => row.root === 'CurrentUser' && row.data);
  const uninstall = current.find(row => row.path.endsWith(`Uninstall\\${config.productName}`)); assert(uninstall, 'Native HKCU uninstall record is missing');
  const values = uninstall.data.values; assert.equal(values.DisplayName, config.productName); assert.equal(values.MainBinaryName, `${config.mainBinaryName}.exe`);
  assert(samePath(values.InstallLocation.replace(/^"|"$/g, ''), installRoot)); assert.equal(values.UninstallString.toLowerCase(), `"${join(installRoot, 'uninstall.exe')}"`.toLowerCase());
  const product = current.find(row => row.path === `Software\\${manufacturer}\\${config.productName}`); assert(product && samePath(product.data.values[''], installRoot));
  const protocol = current.find(row => row.path === `Software\\Classes\\${config.protocol}`); assert(protocol); assert.equal(protocol.data.values[''], `URL:${config.identifier} protocol`); assert(Object.hasOwn(protocol.data.values, 'URL Protocol')); assert.equal(protocol.data.open_command.toLowerCase(), `"${exe}" "%1"`.toLowerCase()); assert(inventory.registry.filter(row => row.root === 'LocalMachine').every(row => row.data === null));
  assert.equal(inventory.shortcuts.length, 2); for (const shortcut of inventory.shortcuts) { assert(shortcut.exists, `Missing unique shortcut ${shortcut.path}`); assert(samePath(shortcut.target, exe)); }
}
async function prepare(configPath, nsiPath) {
  assert.equal(process.platform, 'win32'); const login = await windowsLoginIdentity();
  assert.equal(await fileDigest(join(repository, 'apps/desktop/nsis/installer.nsi')), '646077ad9b18482820aab29237248a2585fbc1944525ff96393ad8c316415311', 'Acceptance must use the current preserved-data NSIS source contract');
  const configFile = await stableText(resolve(configPath)), nsi = await stableText(resolve(nsiPath));
  const sourceConfig = JSON.parse(configFile.text), config = validateInstallerAcceptanceConfig(sourceConfig), rendered = parseRenderedAcceptanceInstaller(nsi.text, sourceConfig);
  Object.assign(rendered, locateRenderedAcceptanceInstallerOutput({ config: sourceConfig, rendered, renderedNsi: nsiPath }));
  const packagedExecutable = await unsignedNsisExecutableExpectation(rendered.sourceExe, sourceConfig);
  for (const [target, source] of [['LICENSE', 'LICENSE'], ['NOTICE', 'NOTICE'], ['component-notices/npm-10.9.4-LICENSE.txt', 'crates/backend/nomifun-runtime/notices/npm-10.9.4-LICENSE.txt']]) assert.equal(await fileDigest(rendered.resources.find(entry => entry.target === target).source), await fileDigest(join(repository, source)), `Full source notice mismatch: ${target}`);
  const before = await inventory(config, rendered), installRoot = join(before.local_appdata, 'Programs', config.productName), exe = join(installRoot, `${config.mainBinaryName}.exe`);
  return { config, rendered, packagedExecutable, login, before, installRoot, exe, config_sha256: configFile.sha256, rendered_nsi_sha256: nsi.sha256 };
}
async function installedFiles(state) {
  assert(samePath(await realpath(state.installRoot), state.installRoot), 'Install root must not be a junction');
  assert.equal(await fileDigest(state.rendered.sourceExe), state.packagedExecutable.source_executable_sha256);
  const executableSha256 = await fileDigest(state.exe);
  assert.equal(executableSha256, state.packagedExecutable.expected_packaged_executable_sha256, 'The installed executable must match the complete exact first-token NSIS prediction');
  const signatures = await observeInstallerSignatures([state.exe]); assert.equal(signatures.length, 1); assert(samePath(signatures[0].path, state.exe));
  assert.equal(signatures[0].status, 'NotSigned'); assert.equal(signatures[0].has_signer, false, 'The acceptance prediction cannot mask signed executable changes');
  const resources = []; for (const entry of state.rendered.resources) { const path = join(state.installRoot, ...entry.target.split('/')); const sha256 = await fileDigest(path); assert.equal(sha256, await fileDigest(entry.source), `Installed resource mismatch: ${entry.target}`); resources.push({ target: entry.target, sha256 }); }
  assert.equal(await fileDigest(state.exe), executableSha256, 'The installed executable changed during verification');
  return { executable_sha256: executableSha256, uninstaller_sha256: await fileDigest(join(state.installRoot, 'uninstall.exe')), resources, authenticode: signatures[0] };
}
async function withReport(reportRoot, mode, work) {
  await mkdir(reportRoot, { recursive: true }); const path = join(resolve(reportRoot), `windows-installer-${mode}-${randomUUID()}.json`);
  const report = { schema: 'nomifun.windows-installer-acceptance.v1', mode, status: 'preparing', created_at: new Date().toISOString(), evidence_scope: 'unique current-user unsigned acceptance variant; not production signing certification' };
  await writeFile(path, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
  try { await work(report, () => writeFile(path, `${JSON.stringify(report, null, 2)}\n`)); return { ...report, report_path: path }; }
  catch (error) { report.status = 'failed_or_result_unknown'; report.failure = { name: error.name, message: error.message }; throw error; }
  finally { await writeFile(path, `${JSON.stringify(report, null, 2)}\n`); console.log(JSON.stringify({ mode, status: report.status, report: path, install_root: report.install_root, exe: report.exe })); }
}
export async function installWindowsAcceptance({ config: configPath, renderedNsi, installer, reportRoot }) {
  return withReport(reportRoot, 'install', async (report, flush) => {
    const state = await prepare(configPath, renderedNsi); const artifact = await realpath(resolve(installer)); assert.equal(artifact.toLowerCase(), resolve(state.rendered.outfile).toLowerCase(), 'Actual installer must match rendered NSIS OUTFILE');
    assert(await absent(state.installRoot), 'A preexisting install root cannot be replaced'); assert(state.before.registry.every(row => row.data === null)); assert(state.before.autostart.filter(row => row.name === state.config.productName).every(row => !row.present)); assert(state.before.shortcuts.every(row => !row.exists)); assert.deepEqual(state.before.image_processes, []);
    Object.assign(report, { status: 'prepared', config_path: resolve(configPath), config_sha256: state.config_sha256, rendered_nsi: resolve(renderedNsi), rendered_nsi_sha256: state.rendered_nsi_sha256, raw_outfile: state.rendered.raw_outfile, outfile_resolution: state.rendered.outfile_resolution, ...state.packagedExecutable, config: state.config, install_root: state.installRoot, exe: state.exe, login_identity: state.login, installer: artifact, installer_sha256: await fileDigest(artifact), before: state.before, signature_probe_before_dispatch: await observeInstallerSignatures([artifact, state.rendered.sourceExe]) });
    report.status = 'native_installer_dispatched_result_unknown'; await flush();
    report.native_execution = await runNativeInstaller(artifact); await flush(); verifyNativeExit(report.native_execution);
    assert(samePath(await realpath(state.installRoot), state.installRoot));
    report.actual_uninstaller_after_known_exit = { path: join(state.installRoot, 'uninstall.exe'), sha256: await fileDigest(join(state.installRoot, 'uninstall.exe')) }; await flush();
    report.after = await inventory(state.config, state.rendered); await flush(); report.files = await installedFiles(state);
    validateRegistered(report.after, state.config, state.installRoot, state.exe, state.rendered.manufacturer); assert.deepEqual(report.after.daily, state.before.daily, 'Daily application registration changed'); assert.deepEqual(report.after.autostart, state.before.autostart); assert.deepEqual(report.after.image_processes, [], 'Silent install must not launch the App');
    report.authenticode = await observeInstallerSignatures([artifact, state.exe, join(state.installRoot, 'uninstall.exe')]);
    report.status = 'installed';
  });
}

// This read-only evidence describes the exact built artifact. It grants no
// product permission and records no operation/retry state.
export async function prepareWindowsInstallerArtifact({ config: configPath, renderedNsi, installer, reportRoot }) {
  return withReport(reportRoot, 'prepare', async report => {
    const state = await prepare(configPath, renderedNsi), artifact = await realpath(resolve(installer));
    const { nonce: _nonce, protocol: _protocol, ...sourceConfig } = state.config;
    assert(samePath(artifact, state.rendered.outfile)); assert.equal(state.login.administrator, false);
    assert(await absent(state.installRoot)); assert(state.before.registry.every(row => row.data === null));
    assert(state.before.shortcuts.every(row => !row.exists)); assert.deepEqual(state.before.image_processes, []);
    assert(state.before.autostart.filter(row => row.name === state.config.productName).every(row => !row.present));
    Object.assign(report, { schema: 'nomifun.windows-installer-artifact.v1', status: 'artifact_verified_absent',
      config_path: resolve(configPath), config_sha256: state.config_sha256,
      rendered_nsi: resolve(renderedNsi), rendered_nsi_sha256: state.rendered_nsi_sha256,
      raw_outfile: state.rendered.raw_outfile, outfile_resolution: state.rendered.outfile_resolution,
      config: sourceConfig, install_root: state.installRoot, exe: state.exe,
      installer: artifact, installer_sha256: await fileDigest(artifact),
      ...state.packagedExecutable,
      resources: await Promise.all(state.rendered.resources.map(async entry => ({ target: entry.target, sha256: await fileDigest(entry.source) }))),
      login_identity: state.login, before: state.before,
      signature_probe_before_dispatch: await observeInstallerSignatures([artifact, state.rendered.sourceExe]),
      native_installer_dispatched: false });
  });
}

export async function inspectWindowsInstalledAcceptance({ artifactReport }) {
  assert(isAbsolute(artifactReport));
  const reportSource = await stableText(artifactReport), artifact = JSON.parse(reportSource.text);
  assert.equal(artifact.schema, 'nomifun.windows-installer-artifact.v1'); assert.equal(artifact.status, 'artifact_verified_absent');
  assert.equal(artifact.native_installer_dispatched, false);
  const state = await prepare(artifact.config_path, artifact.rendered_nsi);
  assert.equal(state.login.administrator, false); assert.equal(state.login.sid, artifact.login_identity.sid);
  assert.equal(state.config_sha256, artifact.config_sha256); assert.equal(state.rendered_nsi_sha256, artifact.rendered_nsi_sha256);
  assert(samePath(state.installRoot, artifact.install_root)); assert(samePath(state.exe, artifact.exe));
  assert(isAbsolute(artifact.installer)); assert(samePath(await realpath(artifact.installer), state.rendered.outfile));
  assert.equal(await fileDigest(artifact.installer), artifact.installer_sha256);
  assert.equal(state.packagedExecutable.source_executable_sha256, artifact.source_executable_sha256);
  if (Object.hasOwn(artifact, 'expected_packaged_executable_sha256')) {
    assert.equal(state.packagedExecutable.expected_packaged_executable_sha256, artifact.expected_packaged_executable_sha256);
    assert.deepEqual(state.packagedExecutable.bundle_type_patch, artifact.bundle_type_patch);
  } else assert(!Object.hasOwn(artifact, 'bundle_type_patch'), 'A partial source prediction cannot authorize an inspection');
  assert.deepEqual(await Promise.all(state.rendered.resources.map(async entry => ({ target: entry.target, sha256: await fileDigest(entry.source) }))), artifact.resources);
  validateRegistered(state.before, state.config, state.installRoot, state.exe, state.rendered.manufacturer);
  assert.deepEqual(state.before.image_processes, [], 'The unique installed acceptance App must be absent before scoped cleanup');
  assert.deepEqual(state.before.daily, artifact.before.daily, 'Daily registration changed since the original artifact preparation');
  assert.deepEqual(state.before.autostart, artifact.before.autostart);
  const files = await installedFiles(state);
  assert.deepEqual(files.resources, artifact.resources);
  const signatures = await observeInstallerSignatures([artifact.installer, state.rendered.sourceExe, state.exe, join(state.installRoot, 'uninstall.exe')]);
  assert.equal(signatures.length, 4);
  for (const signature of signatures) { assert.equal(signature.status, 'NotSigned'); assert.equal(signature.has_signer, false); }
  assert.equal(await fileDigest(artifactReport), reportSource.sha256, 'The original artifact report changed during read-only inspection');
  return { schema: 'nomifun.windows-installed-acceptance-inspection.v1', read_only: true, artifact_report: artifactReport, artifact_report_sha256: reportSource.sha256,
    config: artifact.config, config_sha256: state.config_sha256, rendered_nsi: artifact.rendered_nsi, rendered_nsi_sha256: state.rendered_nsi_sha256,
    install_root: state.installRoot, exe: state.exe, login_identity: state.login, before: state.before, artifact_before: artifact.before,
    source_executable: state.rendered.sourceExe, ...state.packagedExecutable, files, authenticode: signatures,
    prediction_recorded_in_original_artifact: Object.hasOwn(artifact, 'expected_packaged_executable_sha256'), original_artifact_report_unchanged: true,
    installer_executions: 0, filesystem_mutations: 0, canonical_mutations: 0 };
}

export function canonicalUninstallerRawArguments(installRoot, config) {
  const selected = validateInstallerAcceptanceConfig(config);
  assert(win32.isAbsolute(installRoot) && win32.basename(installRoot) === selected.productName);
  assert(!/["\x00-\x1f]/.test(installRoot), 'NSIS raw directory tail must have native unquoted spelling');
  return `/S _?=${installRoot}`;
}
const nativeBinaryGuard = (path, sha256) => {
  assert(win32.isAbsolute(path)); assert.match(sha256, /^[a-f0-9]{64}$/);
  return `$binary=${psLiteral(path)};$item=Get-Item -Force -LiteralPath $binary;if($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)){throw 'Expected exact regular binary'};$hash=(Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant();if($hash -cne ${psLiteral(sha256)}){throw 'Exact native binary SHA mismatch'};`;
};
export function canonicalInstallerExecutionScript({ phase, path, sha256, rawArguments }) {
  assert(['install', 'uninstall'].includes(phase)); assert.equal(typeof rawArguments, 'string');
  assert(phase === 'install' ? rawArguments === '/S' : rawArguments.startsWith('/S _?=') && !/["\x00-\x1f]/.test(rawArguments));
  return `${elevationProbe}\n${nativeBinaryGuard(path, sha256)}
$start=New-Object Diagnostics.ProcessStartInfo;$start.FileName=$binary;$start.Arguments=${psLiteral(rawArguments)};$start.UseShellExecute=$false;$start.CreateNoWindow=$true;
$p=[Diagnostics.Process]::Start($start);$receipt=[ordered]@{schema='nomifun.canonical-installer-process.v1';phase=${psLiteral(phase)};path=$binary;sha256=$hash;raw_arguments=$start.Arguments;pid=$p.Id;elevated=$null;actual_exit=$false;exit_code=$null};
try{$receipt.elevated=[NomiFunInstallerToken]::Elevated($p.Handle);$p.WaitForExit();$receipt.actual_exit=$true;$receipt.exit_code=$p.ExitCode;[Console]::Write(($receipt|ConvertTo-Json -Compress));[Console]::Out.Flush();if($receipt.elevated -or $receipt.exit_code -ne 0){exit 74}}finally{$p.Dispose()}`;
}
export function validateCanonicalInstallerReceipt(value, expected) {
  assert.equal(value.state, 'exited', 'Unknown/running/cancelled native installer is never replayed');
  assert.equal(value.exit_code, 0); assert.equal(value.cleanup?.reaped, true);
  assert.deepEqual(value.cleanup?.errors ?? [], []);
  const receipt = JSON.parse(value.output.text.trim());
  assert.equal(receipt.schema, 'nomifun.canonical-installer-process.v1');
  assert.equal(receipt.phase, expected.phase); assert(samePath(receipt.path, expected.path));
  assert.equal(receipt.sha256, expected.sha256); assert.equal(receipt.raw_arguments, expected.rawArguments);
  assert(Number.isSafeInteger(receipt.pid) && receipt.pid > 1); assert.equal(receipt.elevated, false);
  assert.equal(receipt.actual_exit, true); assert.equal(receipt.exit_code, 0);
  return receipt;
}

export function canonicalInstallerCases({ config, workspace, installer, installerSha256, installRoot,
  uninstallerCopy, sentinel, sentinelContent, getUninstallerSha256, verifyInstall, verifyCopy,
  verifyUninstall, verifyCopiedDeleted, verifySentinel, verifySentinelDeleted }) {
  const rawArguments = canonicalUninstallerRawArguments(installRoot, config);
  config = validateInstallerAcceptanceConfig(config);
  assert(win32.isAbsolute(workspace) && win32.isAbsolute(uninstallerCopy) && win32.isAbsolute(sentinel));
  assert.equal(win32.dirname(uninstallerCopy), win32.dirname(workspace));
  assert.match(win32.basename(uninstallerCopy), /^canonical-uninstaller-[0-9a-f-]{36}\.exe$/);
  assert.equal(win32.basename(sentinel), `.installer-preservation-${config.nonce}`);
  const launch = (phase, path, sha256, rawArguments) => ({ command: powershell,
    args: psArguments(canonicalInstallerExecutionScript({ phase, path, sha256, rawArguments })), cwd: workspace, timeout_ms: 180000 });
  const install = { id: 'installer-canonical-install', name: 'start_process',
    args: launch('install', installer, installerSha256, '/S'), verify: verifyInstall };
  const copy = { id: 'installer-canonical-copy-uninstaller', name: 'start_process', args: () => {
    const source = win32.join(installRoot, 'uninstall.exe');
    return { command: powershell, cwd: workspace, timeout_ms: 20000, args: psArguments(`${nativeBinaryGuard(source, getUninstallerSha256())}if([IO.File]::Exists(${psLiteral(uninstallerCopy)}) -or [IO.Directory]::Exists(${psLiteral(uninstallerCopy)})){throw 'Copied uninstaller target collision'};[IO.File]::Copy($binary,${psLiteral(uninstallerCopy)},$false);[Console]::Write('COPIED_VERIFIED_UNINSTALLER')`) };
  }, verify: verifyCopy };
  const uninstall = { id: 'installer-canonical-uninstall', name: 'start_process', args: () =>
    launch('uninstall', uninstallerCopy, getUninstallerSha256(), rawArguments), verify: verifyUninstall };
  const preserve = { id: 'installer-data-preservation-write', name: 'write_file', args: { path: sentinel, content: sentinelContent }, verify: verifySentinel };
  const deleteCopy = { id: 'installer-canonical-delete-copied-uninstaller', name: 'delete_path', args: { path: uninstallerCopy }, verify: verifyCopiedDeleted };
  const deleteSentinel = { id: 'installer-data-preservation-delete', name: 'delete_path', args: { path: sentinel }, verify: verifySentinelDeleted };
  const phases = [
    { role: 'installer-install', prompt: `仅用现役普通用户exec和文件工具完成本阶段：在实际App数据根写本次唯一无敏感sentinel，再验证并静默安装唯一NSIS ${installer}（SHA ${installerSha256}），保持默认安装目录 ${installRoot}。不启动目标App、不重装、不卸载、不修改其他注册。此阶段不代表卸载或正式签名认证。`, steps: [preserve, install] },
    { role: 'installer-uninstall', prompt: `这是同Session的新Turn。仅复制已验SHA的唯一 ${join(installRoot, 'uninstall.exe')} 到本次私有 ${uninstallerCopy}，通过固定显式WinPS及ProcessStartInfo UseShellExecute=false/raw最后参数同步卸载 ${installRoot}，取得真实PID/exit及原Supervisor回收证明，再只删除本次复制品和sentinel。不得杀App、重放未知操作、直接删安装目录或修改日常NomiFun。`, steps: [copy, uninstall, deleteCopy, deleteSentinel] },
  ];
  return { phases, steps: phases.flatMap(phase => phase.steps) };
}

export async function createWindowsInstallerScenario({ fixture, workspace, loginIdentity, installerReport, nativeDataRoot }) {
  await assertWindowsFixture(fixture, workspace); await assertWindowsPrivatePath(fixture);
  assert.equal(loginIdentity.administrator, false); assert(isAbsolute(installerReport));
  const reportSource = await stableText(installerReport), artifact = JSON.parse(reportSource.text);
  assert.equal(artifact.schema, 'nomifun.windows-installer-artifact.v1'); assert.equal(artifact.status, 'artifact_verified_absent');
  assert.equal(artifact.native_installer_dispatched, false); assert.equal(artifact.login_identity.sid, loginIdentity.sid);
  const state = await prepare(artifact.config_path, artifact.rendered_nsi);
  assert.equal(state.config_sha256, artifact.config_sha256); assert.equal(state.rendered_nsi_sha256, artifact.rendered_nsi_sha256);
  assert(samePath(state.installRoot, artifact.install_root)); assert(samePath(state.exe, artifact.exe));
  assert(samePath(await realpath(artifact.installer), state.rendered.outfile));
  assert.equal(await fileDigest(artifact.installer), artifact.installer_sha256);
  assert.equal(await fileDigest(state.rendered.sourceExe), artifact.source_executable_sha256);
  assert.deepEqual(await Promise.all(state.rendered.resources.map(async entry => ({ target: entry.target, sha256: await fileDigest(entry.source) }))), artifact.resources);
  assert(await absent(state.installRoot)); assert(state.before.registry.every(row => row.data === null));
  assert(state.before.shortcuts.every(row => !row.exists)); assert.deepEqual(state.before.image_processes, []);
  assert(state.before.autostart.filter(row => row.name === state.config.productName).every(row => !row.present));
  const dataRoot = nativeDataRoot.root; assert(isAbsolute(dataRoot)); await assertWindowsPrivatePath(dataRoot);
  const database = join(dataRoot, 'nomifun-backend.db'), initialDatabase = await lstat(database);
  assert(initialDatabase.isFile() && !initialDatabase.isSymbolicLink());
  const verifyDatabase = async () => { const handle = await open(database, 'r'); try {
    const info = await handle.stat(); assert.equal(info.ino, initialDatabase.ino); assert.equal(info.dev, initialDatabase.dev);
    const header = Buffer.alloc(16); assert.equal((await handle.read(header, 0, 16, 0)).bytesRead, 16); assert.equal(header.toString(), 'SQLite format 3\0');
  } finally { await handle.close(); } };
  await verifyDatabase();
  const uninstallerCopy = join(fixture, `canonical-uninstaller-${randomUUID()}.exe`), sentinel = join(dataRoot, `.installer-preservation-${state.config.nonce}`);
  assert(await absent(uninstallerCopy)); assert(await absent(sentinel));
  const sentinelContent = `canonical installer data preservation ${randomUUID()}\n`, nativeEvidence = [];
  let installed, uninstalled = false;
  const preserved = async phase => { await verifyDatabase(); assert.equal(await fileDigest(sentinel), digest(sentinelContent)); nativeEvidence.push({ phase, native_data_root: dataRoot, database_identity_retained: true, sqlite_header_valid: true, sentinel_sha256: digest(sentinelContent) }); };
  const plan = canonicalInstallerCases({ config: artifact.config, workspace, installer: artifact.installer,
    installerSha256: artifact.installer_sha256, installRoot: state.installRoot, uninstallerCopy, sentinel, sentinelContent,
    getUninstallerSha256: () => { assert(installed, 'No fully verified install may be uninstalled'); return installed.uninstaller_sha256; },
    verifySentinel: async () => preserved('sentinel_created'),
    verifyInstall: async value => {
      const receipt = validateCanonicalInstallerReceipt(value, { phase: 'install', path: artifact.installer, sha256: artifact.installer_sha256, rawArguments: '/S' });
      nativeEvidence.push({ phase: 'installer_native_exit', ...receipt }); assert(windowsProcessGone(receipt.pid));
      const after = await inventory(state.config, state.rendered); validateRegistered(after, state.config, state.installRoot, state.exe, state.rendered.manufacturer);
      assert.deepEqual(after.daily, state.before.daily); assert.deepEqual(after.autostart, state.before.autostart); assert.deepEqual(after.image_processes, []);
      installed = await installedFiles(state); await preserved('after_install'); nativeEvidence.push({ phase: 'installed', files: installed, inventory: after });
    },
    verifyCopy: async value => {
      assert.equal(value.state, 'exited'); assert.equal(value.exit_code, 0); assert.equal(value.cleanup?.reaped, true);
      assert.equal(value.output.text.trim(), 'COPIED_VERIFIED_UNINSTALLER'); assert.equal(await fileDigest(uninstallerCopy), installed.uninstaller_sha256);
    },
    verifyUninstall: async value => {
      const receipt = validateCanonicalInstallerReceipt(value, { phase: 'uninstall', path: uninstallerCopy, sha256: installed.uninstaller_sha256, rawArguments: canonicalUninstallerRawArguments(state.installRoot, artifact.config) });
      nativeEvidence.push({ phase: 'uninstaller_native_exit', ...receipt }); assert(windowsProcessGone(receipt.pid));
      const after = await inventory(state.config, state.rendered); assert(await absent(state.installRoot));
      assert(after.registry.every(row => row.data === null)); assert(after.shortcuts.every(row => !row.exists)); assert.deepEqual(after.daily, state.before.daily);
      assert.deepEqual(after.autostart, state.before.autostart); assert.deepEqual(after.image_processes, []);
      await preserved('after_uninstall'); uninstalled = true; nativeEvidence.push({ phase: 'uninstalled_by_nsis', inventory: after });
    },
    verifyCopiedDeleted: async () => { assert(uninstalled); assert(await absent(uninstallerCopy)); },
    verifySentinelDeleted: async () => { assert(uninstalled); assert(await absent(sentinel)); await verifyDatabase(); },
  });
  return { ...plan, resources: { artifact_report: installerReport, artifact_report_sha256: reportSource.sha256,
    installer: artifact.installer, installer_sha256: artifact.installer_sha256, identifier: state.config.identifier,
    install_root: state.installRoot, exe: state.exe, uninstaller_copy: uninstallerCopy, data_sentinel: sentinel, native_data_root: dataRoot }, nativeEvidence,
    prompt: '仅执行本次已验证唯一Windows NSIS应用安装与同Session新Turn卸载，所有修改经现役canonical工具。',
    completionSummary: role => role === 'installer-install' ? '唯一UUID current-user NSIS经canonical普通用户exec安装，真实exit0/reaped与独立完整资源/注册/数据sentinel已核验；卸载尚未完成。' : '同Session新Turn经固定显式WinPS同步运行已验复制uninstaller；NSIS原生卸载、确切PID退出、Supervisor回收、唯一目录/注册消失及实际App数据保持已核验；复制品和sentinel经canonical文件工具清理。',
    // An unknown or partial installer is never automatically replayed as
    // recovery. Preserve the failed canonical report and exact native state.
    cleanupNeeded: async () => false, cleanupSteps: [],
    cleanupPrompt: '保留原失败与未知结果，不重放NSIS，也不直接删除安装目录。任何后续恢复必须先核实原canonical执行与唯一native目标。',
    cleanupIntent: { install_root: state.installRoot, uninstaller_copy: uninstallerCopy, sentinel, no_host_mutations: true, no_unknown_replay: true },
    allowsCanonicalFixturePath: path => samePath(path, sentinel),
    close: async () => { nativeEvidence.push({ phase: 'final_readonly_inventory', install_root_present: !await absent(state.installRoot), copied_uninstaller_present: !await absent(uninstallerCopy), sentinel_present: !await absent(sentinel) }); },
  };
}
export function validateInstallerFixtureClose(fixture, close, fixtureReport) {
  assert.equal(close.schema, 'nomifun.windows-global-os-fixture-close.v1'); assert.equal(close.status, 'exited'); assert.equal(close.path, 'existing_tauri_process_exit');
  assert.equal(close.actual_pid_gone, true); assert.equal(close.process_exit_proven, true); assert.equal(close.exit_code, 0);
  assert.equal(close.app_pid, fixture.app_pid); assert(samePath(close.fixture_report, fixtureReport));
  const native = validateWindowsExitResult(close.native_process_exit, windowsCloseProcessIdentity(fixture));
  assert.equal(native.exit_code, 0, 'A proven nonzero App exit does not authorize installer acceptance');
  assert.equal(native.observer_reaped, true); assert.equal(native.observer_exit_code, 0);
  return native;
}
export async function uninstallWindowsAcceptance({ config: configPath, renderedNsi, installReport, fixtureReport, fixtureCloseReport, reportRoot }) {
  return withReport(reportRoot, 'uninstall', async (report, flush) => {
    const state = await prepare(configPath, renderedNsi), installed = JSON.parse((await stableText(resolve(installReport))).text);
    assert.equal(installed.schema, 'nomifun.windows-installer-acceptance.v1'); assert.equal(installed.mode, 'install'); assert.equal(installed.status, 'installed'); assert.equal(installed.config_sha256, state.config_sha256); assert.equal(installed.rendered_nsi_sha256, state.rendered_nsi_sha256); assert(samePath(installed.install_root, state.installRoot)); assert(samePath(installed.exe, state.exe));
    assert.equal(state.login.sid, installed.login_identity.sid); validateRegistered(state.before, state.config, state.installRoot, state.exe, state.rendered.manufacturer); assert.deepEqual(state.before.image_processes, [], 'The exact unique executable name must have no remaining process');
    const fixture = JSON.parse((await stableText(resolve(fixtureReport))).text), close = JSON.parse((await stableText(resolve(fixtureCloseReport))).text);
    assert.equal(fixture.schema, 'nomifun.windows-global-os-fixture.v1'); assert.equal(fixture.status, 'ready'); assert(samePath(fixture.exe, state.exe)); assert.equal(fixture.exe_sha256, installed.files.executable_sha256); assert.equal(fixture.login_identity.sid, state.login.sid);
    assert.match(basename(fixture.parent), /^nomifun-global-os-owner-[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/); assert(samePath(dirname(fixture.parent), await realpath(tmpdir()))); assert(samePath(fixture.native_data_root, join(fixture.parent, 'native-data'))); assert(samePath(await realpath(fixture.parent), fixture.parent)); assert(samePath(await realpath(fixture.native_data_root), fixture.native_data_root)); await assertWindowsPrivatePath(fixture.parent); await assertWindowsPrivatePath(fixture.native_data_root);
    assert.equal(await readFile(join(fixture.parent, '.nomifun-global-os-fixture'), 'utf8'), 'nomifun-global-os-native-acceptance-fixture.v1\n');
    validateInstallerFixtureClose(fixture, close, fixtureReport); assert(windowsProcessGone(fixture.app_pid));
    const database = join(fixture.native_data_root, 'nomifun-backend.db'), databaseSha = await fileDigest(database); const header = await open(database, 'r'); try { const bytes = Buffer.alloc(16); assert.equal((await header.read(bytes, 0, 16, 0)).bytesRead, 16); assert.equal(bytes.toString(), 'SQLite format 3\0'); } finally { await header.close(); }
    const sentinel = join(fixture.native_data_root, `.installer-preservation-${state.config.nonce}`), sentinelBytes = `nomifun installer preserves actual application data ${randomUUID()}\n`; await writeFile(sentinel, sentinelBytes, { flag: 'wx' });
    assert.equal(await fileDigest(state.exe), installed.files.executable_sha256); const uninstaller = join(state.installRoot, 'uninstall.exe'); assert.equal(await fileDigest(uninstaller), installed.files.uninstaller_sha256);
    Object.assign(report, { status: 'prepared', config: state.config, install_report: resolve(installReport), fixture_report: resolve(fixtureReport), fixture_close_report: resolve(fixtureCloseReport), install_root: state.installRoot, exe: state.exe, login_identity: state.login, before: state.before, preserved_data: { native_data_root: fixture.native_data_root, database, database_sha256: databaseSha, sentinel, sentinel_sha256: digest(sentinelBytes), app_pid: fixture.app_pid, actual_pid_gone: true, exact_image_processes: [] } });
    report.signature_probe_before_dispatch = await observeInstallerSignatures([uninstaller]); report.status = 'native_uninstaller_dispatched_result_unknown'; await flush();
    report.native_execution = await runNativeInstaller(uninstaller); await flush(); verifyNativeExit(report.native_execution);
    // NSIS may finish self-deletion in its copied uninstaller after the initial
    // process returns. Observe boundedly; never remove leftovers ourselves.
    for (let attempt = 0; attempt < 100 && !await absent(state.installRoot); attempt++) await new Promise(done => setTimeout(done, 100));
    report.after = await inventory(state.config, state.rendered); assert(await absent(state.installRoot), 'Native uninstaller left the install directory; preserve the failure'); assert(report.after.registry.every(row => row.data === null)); assert(report.after.shortcuts.every(row => !row.exists)); assert.deepEqual(report.after.daily, state.before.daily); assert.deepEqual(report.after.autostart.filter(row => row.name === 'NomiFun'), state.before.autostart.filter(row => row.name === 'NomiFun')); assert(report.after.autostart.filter(row => row.name === state.config.productName).every(row => !row.present)); assert.deepEqual(report.after.image_processes, []);
    assert.equal(await fileDigest(database), databaseSha); assert.equal(await fileDigest(sentinel), digest(sentinelBytes)); assert(samePath(await realpath(fixture.native_data_root), fixture.native_data_root)); report.preserved_data.verified_after_native_uninstall = true; report.status = 'uninstalled_data_preserved';
  });
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2), flags = new Map();
  if (args.length === 0 || args.includes('--help')) console.log('node scripts/validation/global-os-windows-installer.mjs --prepare|--install --config ABS --rendered-nsi ABS --installer ABS --report-root ABS; --uninstall --config ABS --rendered-nsi ABS --install-report ABS --fixture-report ABS --fixture-close-report ABS --report-root ABS. Prepare is read-only artifact evidence for the separate canonical Agent installer scenario. Only a unique UUID currentUser acceptance variant is allowed. No /R, /D, forced exit, registration repair or manual cleanup.');
  else {
    const mode = args.shift(); assert(['--prepare', '--install', '--uninstall'].includes(mode)); assert.equal(args.length % 2, 0);
    for (let index = 0; index < args.length; index += 2) { assert(!flags.has(args[index])); flags.set(args[index], args[index + 1]); }
    const required = mode !== '--uninstall' ? ['--config', '--rendered-nsi', '--installer', '--report-root'] : ['--config', '--rendered-nsi', '--install-report', '--fixture-report', '--fixture-close-report', '--report-root']; assert.deepEqual([...flags.keys()].sort(), required.sort()); for (const value of flags.values()) assert(isAbsolute(value));
    const options = Object.fromEntries([...flags].map(([key, value]) => [key.slice(2).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase()), value]));
    if (mode === '--prepare') await prepareWindowsInstallerArtifact(options); else if (mode === '--install') await installWindowsAcceptance(options); else await uninstallWindowsAcceptance(options);
  }
}
