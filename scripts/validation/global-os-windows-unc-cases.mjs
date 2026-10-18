// The host only inventories existing SMB facilities and observes this unique
// fixture. Share/files/process changes use canonical default-Agent tools.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { lstat, readFile } from 'node:fs/promises';
import { hostname } from 'node:os';
import { join } from 'node:path';
import { assertWindowsFixture, powershell, psArguments, psLiteral, windowsObservation } from './global-os-windows-cases.mjs';
import { probeWindowsUncDirectoryIdentity } from './global-os-windows-unc-readonly.mjs';

const sha256 = value => createHash('sha256').update(value).digest('hex');
const json = value => JSON.parse(value.output.text.replace(/^\uFEFF/, '').trim());
const exited = value => { assert.equal(value.state, 'exited'); assert.equal(value.exit_code, 0); assert.equal(value.cleanup?.reaped, true); };
const absent = async path => lstat(path).then(() => false, error => { if (error.code === 'ENOENT') return true; throw error; });
const plainNative = value => value.startsWith('\\\\?\\UNC\\') ? `\\\\${value.slice(8)}` : value.startsWith('\\\\?\\') ? value.slice(4) : value;
const sameNative = (left, right) => plainNative(left).replace(/\\+$/, '').toLowerCase() === plainNative(right).replace(/\\+$/, '').toLowerCase();
const facilityScript = "$s=Get-Service -Name LanmanServer -ErrorAction SilentlyContinue;$client=Get-Service -Name LanmanWorkstation -ErrorAction SilentlyContinue;$c=@('Get-SmbShare','New-SmbShare','Remove-SmbShare','Get-SmbShareAccess')|ForEach-Object{[pscustomobject]@{name=$_;available=($null -ne (Get-Command $_ -ErrorAction SilentlyContinue))}};[pscustomobject]@{service_exists=($null -ne $s);service_status=if($s){[string]$s.Status}else{$null};client_service_status=if($client){[string]$client.Status}else{$null};computer_name=$env:COMPUTERNAME;commands=@($c)}|ConvertTo-Json -Depth 5 -Compress";

export function ownedUncFixturePath(path, { shareName, hosts, verified }) {
  if (!verified || typeof path !== 'string' || path.includes('\0') || path.includes('/')) return false;
  path = plainNative(path);
  if (!path.startsWith('\\\\')) return false;
  const parts = path.slice(2).split('\\');
  if (parts.at(-1) === '') parts.pop();
  if (parts.length < 2 || parts.some(part => !part || part === '.' || part === '..')) return false;
  return hosts.some(host => host.toLowerCase() === parts[0].toLowerCase()) && parts[1].toLowerCase() === shareName.toLowerCase();
}

export async function createWindowsUncScenario({ fixture, workspace, loginIdentity }) {
  await assertWindowsFixture(fixture, workspace); assert.equal(loginIdentity.administrator, false);
  const nonce = randomUUID(), root = join(fixture, `unc-cases-${nonce}`), physical = join(root, 'share-root');
  const shareName = `NomiFun_GlobalOS_${nonce}`, description = `NomiFun unique native acceptance ${nonce}`;
  const unc = `\\\\localhost\\${shareName}`, extended = `\\\\?\\UNC\\localhost\\${shareName}`;
  const seedName = `identity-${nonce}.txt`, seed = `UNC_FIXTURE_${nonce}\n`;
  const initial = `Windows UNC 初始 ${nonce}\n`, changed = `Windows UNC 已修改 ${nonce}\n`;
  const file = `${unc}\\sample.txt`, renamed = `${unc}\\renamed.txt`, scriptPath = `${unc}\\literal-tool.ps1`;
  const literal = ['', '中文 空格', `single' and "double" quote`, '% ! & | < > ^ $ $(throw "NOT_LITERAL")', 'trailing\\'];
  const script = "$ProgressPreference='SilentlyContinue'\n[Console]::OutputEncoding=New-Object Text.UTF8Encoding($false)\n[pscustomobject]@{cwd=(Get-Location).ProviderPath;native_cwd=[Environment]::CurrentDirectory;argv=@($args)}|ConvertTo-Json -Depth 4 -Compress\n";
  const nativeEvidence = [], hosts = ['localhost', hostname()];
  let inventory, shareAttempted = false, shareVerified = false, shareRemoved = false;
  try { inventory = await windowsObservation(facilityScript); }
  catch (error) { inventory = { query_state: 'failed', os_code: error.code, message: error.message }; }
  nativeEvidence.push({ phase: 'ordinary_readonly_smb_facilities', inventory });
  const resources = { root, physical_root: physical, share_name: shareName, unc_root: unc, extended_unc_root: extended, share_acl_sid: loginIdentity.sid, existing_facilities: inventory };
  const execStep = (id, source, verify, administrator = false) => ({ id, name: 'start_process', args: {
    command: powershell, args: psArguments(source), timeout_ms: 30000, ...(administrator ? { execution_identity: 'administrator' } : {}),
  }, verify: async value => { exited(value); await verify?.(value); } });
  const shareQuery = `$name=${psLiteral(shareName)};$share=$null;try{$share=Get-SmbShare -Name $name -ErrorAction Stop}catch{if($_.CategoryInfo.Category -ne 'ObjectNotFound'){throw}};`;
  const requireOwned = `if($null -ne $share){if($share.Name -cne ${psLiteral(shareName)} -or $share.Path.TrimEnd('\\') -ine ${psLiteral(physical)}.TrimEnd('\\') -or $share.Description -cne ${psLiteral(description)}){throw 'Unique share identity changed; preserve it'}};`;
  const observeShare = `$access=@(Get-SmbShareAccess -Name $name -ErrorAction Stop|ForEach-Object{$account=New-Object Security.Principal.NTAccount($_.AccountName);[pscustomobject]@{sid=$account.Translate([Security.Principal.SecurityIdentifier]).Value;kind=[string]$_.AccessControlType;right=[string]$_.AccessRight}});[pscustomobject]@{name=$share.Name;path=$share.Path;description=$share.Description;state=[string]$share.ShareState;access=$access}|ConvertTo-Json -Depth 5 -Compress`;
  const createShare = `${shareQuery}if($null -ne $share){throw 'Unique share collision'};New-SmbShare -Name $name -Path ${psLiteral(physical)} -Description ${psLiteral(description)} -FullAccess ${psLiteral(loginIdentity.name)} -ErrorAction Stop|Out-Null;$share=Get-SmbShare -Name $name -ErrorAction Stop;${requireOwned}${observeShare}`;
  const removeShare = `${shareQuery}${requireOwned}if($null -ne $share){Remove-SmbShare -Name $name -Force -Confirm:$false -ErrorAction Stop};${shareQuery}if($null -ne $share){throw 'Unique share still exists'};[pscustomobject]@{name=$name;exists=$false;expected_path=${psLiteral(physical)}}|ConvertTo-Json -Compress`;
  const remove = id => execStep(id, removeShare, value => {
    const result = json(value); assert.equal(result.name, shareName); assert.equal(result.exists, false); assert(sameNative(result.expected_path, physical));
    shareRemoved = true; shareVerified = false; nativeEvidence.push({ phase: id, kind: 'canonical_native_share_absent', ...result });
  }, true);
  const deleteRoot = id => ({ id, name: 'delete_path', args: { path: root }, verify: async () => { assert(shareRemoved || !shareAttempted, 'Remove the owned SMB share before deleting its physical root'); assert(await absent(root)); } });
  const steps = [
    execStep('unc-existing-facilities', facilityScript, value => {
      const actual = json(value); nativeEvidence.push({ phase: 'canonical_readonly_smb_facilities', inventory: actual });
      assert.equal(actual.service_exists, true, 'LanmanServer is absent; preserve native platform limitation');
      assert.equal(actual.service_status, 'Running', 'Existing LanmanServer is not running; this case never starts/configures the service');
      assert.equal(actual.client_service_status, 'Running', 'Existing LanmanWorkstation is not running; preserve the facility state');
      assert(actual.commands.length === 4 && actual.commands.every(command => command.available));
      assert(hosts.some(host => host.toLowerCase() === actual.computer_name.toLowerCase()));
    }),
    execStep('unc-physical-fixture', `if(Test-Path -LiteralPath ${psLiteral(root)}){throw 'Fixture collision'};New-Item -ItemType Directory -Path ${psLiteral(physical)}|Out-Null;[IO.File]::WriteAllText(${psLiteral(join(physical, seedName))},${psLiteral(seed)},(New-Object Text.UTF8Encoding($false)));[Console]::WriteLine('UNC_PHYSICAL_FIXTURE_READY')`, async () => assert.equal(await readFile(join(physical, seedName), 'utf8'), seed)),
    { ...execStep('unc-create-share', createShare, async value => {
      const actual = json(value); assert.equal(actual.name, shareName); assert(sameNative(actual.path, physical)); assert.equal(actual.description, description);
      assert.equal(actual.state, 'Online'); assert.equal(actual.access.length, 1); assert.deepEqual(actual.access[0], { sid: loginIdentity.sid, kind: 'Allow', right: 'Full' });
      nativeEvidence.push({ phase: 'unique_native_smb_share', ...actual });
      // An ordinary actual UNC read independently proves this registered share
      // reaches the owned physical marker, without adding credentials/grants.
      assert.equal(await readFile(`${unc}\\${seedName}`, 'utf8'), seed);
      shareVerified = true; nativeEvidence.push({ phase: 'ordinary_unc_identity_read', login_sid: loginIdentity.sid, matched_physical_marker_sha256: sha256(seed) });
      nativeEvidence.push({ phase: 'unc_directory_identity', ...await probeWindowsUncDirectoryIdentity({ uncRoot: unc, extendedUncRoot: extended, physicalRoot: physical }) });
    }, true), args: () => { shareAttempted = true; return { command: powershell, args: psArguments(createShare), execution_identity: 'administrator', timeout_ms: 30000 }; } },
    { id: 'unc-write', name: 'write_file', args: { path: file, content: initial }, verify: async () => assert.equal(await readFile(join(physical, 'sample.txt'), 'utf8'), initial) },
    { id: 'unc-read', name: 'read_file', args: { path: file }, verify: value => { assert.equal(value.content, initial); assert.equal(value.sha256, sha256(initial)); } },
    { id: 'unc-patch', name: 'apply_patch', args: { files: [{ path: file, expected_source: { kind: 'existing', sha256: sha256(initial) }, hunks: [{ old_start: 1, old_lines: 1, new_start: 1, new_lines: 1, lines: [{ kind: 'remove', text: initial.trimEnd() }, { kind: 'add', text: changed.trimEnd() }] }] }] }, verify: async () => assert.equal(await readFile(join(physical, 'sample.txt'), 'utf8'), changed) },
    { id: 'unc-search', name: 'search_files', args: { path: `${unc}\\`, query: changed.trimEnd() }, verify: value => assert(JSON.stringify(value).includes('sample.txt')) },
    { id: 'unc-rename', name: 'rename_path', args: { path: file, destination: renamed }, verify: async () => { assert(await absent(join(physical, 'sample.txt'))); assert.equal(await readFile(join(physical, 'renamed.txt'), 'utf8'), changed); } },
    { id: 'unc-extended-alias', name: 'read_file', args: { path: `${extended}\\renamed.txt` }, verify: value => { assert.equal(value.content, changed); assert.equal(value.sha256, sha256(changed)); nativeEvidence.push({ phase: 'plain_extended_unc_same_file', sha256: value.sha256, physical: join(physical, 'renamed.txt'), plain: renamed, extended: `${extended}\\renamed.txt` }); } },
    { id: 'unc-write-script', name: 'write_file', args: { path: scriptPath, content: script }, verify: async () => assert.equal(await readFile(join(physical, 'literal-tool.ps1'), 'utf8'), script) },
    { id: 'unc-native-exe-cwd', name: 'start_process', args: { command: powershell, args: psArguments("[pscustomobject]@{cwd=(Get-Location).ProviderPath;native_cwd=[Environment]::CurrentDirectory;content=[IO.File]::ReadAllText('renamed.txt')}|ConvertTo-Json -Compress"), cwd: `${unc}\\`, timeout_ms: 20000 }, verify: value => {
      exited(value); const actual = json(value); assert(sameNative(actual.cwd, unc)); assert(sameNative(actual.native_cwd, unc)); assert.equal(actual.content, changed); nativeEvidence.push({ phase: 'actual_native_exe_unc_cwd', ...actual });
    } },
    { id: 'unc-native-ps1-literal-cwd', name: 'start_process', args: { command: scriptPath, args: literal, cwd: `${unc}\\`, timeout_ms: 20000 }, verify: value => {
      exited(value); const actual = json(value); assert(sameNative(actual.cwd, unc)); assert(sameNative(actual.native_cwd, unc)); assert.deepEqual(actual.argv, literal); nativeEvidence.push({ phase: 'actual_ps1_unc_cwd_literal_argv', ...actual });
    } },
    { id: 'unc-delete-script', name: 'delete_path', args: { path: scriptPath }, verify: async () => assert(await absent(join(physical, 'literal-tool.ps1'))) },
    { id: 'unc-delete-file', name: 'delete_path', args: { path: renamed }, verify: async () => assert(await absent(join(physical, 'renamed.txt'))) },
    remove('unc-remove-share'), deleteRoot('unc-delete-physical-fixture'),
  ];
  return { steps, resources, nativeEvidence,
    allowsCanonicalFixturePath: path => ownedUncFixturePath(path, { shareName, hosts, verified: shareVerified }),
    prompt: `只操作本次唯一 ${root} 和本机唯一SMB共享 ${shareName}。先只读核对现有LanmanServer已Running与SMB设施；不启动/配置任何已有服务。普通Agent准备自己的目录，管理员canonical exec New-SmbShare只将真实登录 ${loginIdentity.name}/${loginIdentity.sid} 加为Share FullAccess，不Everyone、不改Firewall/SSP/UAC、不传凭据。普通canonical文件工具经 ${unc} write/read/patch/search/rename/delete，并从扩展UNC别名读同一目标；native exe及共享PS1入口核对真实UNC cwd与literal argv。最后管理员只查询/移除Name+Path+Description严格匹配本次共享，确认不存在后删自己的物理目录。原生认证/设施拒绝则保留真实失败，不重放unknown、不降级其他身份读写。`,
    completionSummary: role => role === 'main' ? `Windows现有SMB设施已只读确认Running。本次唯一share仅真实登录SID获FullAccess；普通身份从真实UNC读到同物理marker，canonical write/read/patch/search/rename/delete与扩展UNC别名摘要一致，native PowerShell exe和PS1的真实UNC cwd/literal argv均核验。唯一share经canonical管理员Remove-SmbShare并确认absent=${shareRemoved}，物理夹具已删；未改服务、防火墙、认证或UAC。本地scripted provider不代表联网模型推理。` : `原UNC验收失败保留。本次新Turn仅按精确Name/Path/Description移除自己唯一share，确认absent=${shareRemoved}后清理 ${root}；不重跑原业务或变更已有SMB配置。`,
    cleanupNeeded: async () => (shareAttempted && !shareRemoved) || !await absent(root),
    get cleanupSteps() { return [...(shareAttempted && !shareRemoved ? [remove('unc-recovery-remove-share')] : []), deleteRoot('unc-recovery-delete-physical')]; },
    cleanupPrompt: `原UNC失败保持失败；先前Turn canonical pending/unknown与原生清理已结算后，只用管理员canonical exec查询本次唯一share ${shareName}，Name/Path/Description不匹配则保留，匹配才Remove-SmbShare并证明不存在。然后普通delete_path清理 ${root}。不创建共享、不改SMB服务/防火墙/SSP/UAC，不提供凭据。`,
    cleanupIntent: { temporary_process: () => undefined, path: root, share_name: shareName, only_after_canonical_settlement: true },
    pendingAcceptance: [{ id: 'remote-unc-and-cmd-cwd', status: 'not_run', reason: 'The loopback SMB product case does not prove another computer/network volume or cmd.exe UNC cwd. cmd.exe has its native UNC limitation; no mapped-drive or fake-cwd substitution is used.' }],
  };
}
