// Windows-native cases for the existing public default-Agent/canonical driver.
// Product mutations are proposed as ordinary Agent tools. Host subprocesses
// query exact fixtures; the independent control additionally protects only its
// newly allocated empty fixture before any Session or product content exists.
import assert from 'node:assert/strict';
import { execFile, execFileSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { lstat, open, readFile, readdir, realpath, writeFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { promisify } from 'node:util';
import { allocateLoopbackPort } from './global-os-system-cases.mjs';

const execute = promisify(execFile);
const delay = ms => new Promise(done => setTimeout(done, ms));
export const powershell = join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe');
export const psLiteral = value => `'${String(value).replaceAll("'", "''")}'`;
export const psArguments = script => ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from([
  "$ErrorActionPreference='Stop'", "$ProgressPreference='SilentlyContinue'", '[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)', script,
].join('\n'), 'utf16le').toString('base64')];
const normalized = text => text.replaceAll('\r\n', '\n');
const json = value => JSON.parse(value.output.text.trim());
const exited = value => { assert.equal(value.state, 'exited'); assert.equal(value.exit_code, 0); assert.equal(value.cleanup?.reaped, true); };
const absent = async path => lstat(path).then(() => false, error => { if (error.code === 'ENOENT') return true; throw error; });
export function windowsProcessObservationScript(pid,{includeImagePath=false}={}) {
  assert(Number.isSafeInteger(pid) && pid > 1);
  const script=`Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class NomiAcceptanceProcessQuery {
  [StructLayout(LayoutKind.Sequential)] public struct FT { public uint Low; public uint High; }
  [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
  [DllImport("kernel32.dll", SetLastError=true)] static extern uint WaitForSingleObject(IntPtr handle, uint ms);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetProcessTimes(IntPtr handle, out FT creation, out FT exit, out FT kernel, out FT user);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetExitCodeProcess(IntPtr handle, out uint code);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool QueryFullProcessImageName(IntPtr handle, uint flags, System.Text.StringBuilder text, ref uint size);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
  public static string[] Query(uint pid, bool includeImage) {
    // QUERY_LIMITED_INFORMATION | SYNCHRONIZE. Never request terminate rights.
    IntPtr handle=OpenProcess(0x00101000, false, pid);
    if(handle==IntPtr.Zero) { int error=Marshal.GetLastWin32Error(); if(error==87) return new[]{"absent", "", ""}; throw new Win32Exception(error); }
    try {
      FT creation, exit, kernel, user;
      if(!GetProcessTimes(handle,out creation,out exit,out kernel,out user)) throw new Win32Exception(Marshal.GetLastWin32Error());
      string identity=(((ulong)creation.High<<32)|creation.Low).ToString();
      string image=""; if(includeImage) { var text=new System.Text.StringBuilder(32768); uint size=(uint)text.Capacity; if(!QueryFullProcessImageName(handle,0,text,ref size)) throw new Win32Exception(Marshal.GetLastWin32Error()); image=text.ToString(); }
      uint wait=WaitForSingleObject(handle,0);
      if(wait==258) return new[]{"running",identity,"",image};
      if(wait!=0) throw new Win32Exception(Marshal.GetLastWin32Error(),"Exact process wait failed");
      uint code; if(!GetExitCodeProcess(handle,out code)) throw new Win32Exception(Marshal.GetLastWin32Error());
      return new[]{"exited",identity,code.ToString(),image};
    } finally { CloseHandle(handle); }
  }
}
'@; $v=[NomiAcceptanceProcessQuery]::Query(${pid},$${includeImagePath?'true':'false'}); [pscustomobject]@{pid=${pid};state=$v[0];creation_filetime=$v[1];exit_code=$v[2];image_path=$(if($v.Length-gt3){$v[3]}else{$null});access='QUERY_LIMITED_INFORMATION|SYNCHRONIZE'}|ConvertTo-Json -Compress`;
  return script;
}
export function windowsProcessObservation(pid,options={}) {
  const script=windowsProcessObservationScript(pid,options);
  const raw=execFileSync(powershell,psArguments(script),{encoding:'utf8',windowsHide:true,timeout:10000,maxBuffer:256*1024});
  const value=JSON.parse(raw.replace(/^\uFEFF/,'').trim());
  assert.equal(value.pid,pid);assert(['running','exited','absent'].includes(value.state));
  if(value.state!=='absent')assert.match(value.creation_filetime,/^[1-9][0-9]+$/);
  return value;
}
export const windowsProcessGone = pid => windowsProcessObservation(pid).state !== 'running';
export async function windowsObservation(script) {
  const value = await execute(powershell, psArguments(script), { encoding: 'utf8', windowsHide: true, timeout: 10000, maxBuffer: 256 * 1024 });
  return JSON.parse(value.stdout.replace(/^\uFEFF/, '').trim());
}
export async function windowsLoginIdentity() {
  const identity = await windowsObservation(`$i=[Security.Principal.WindowsIdentity]::GetCurrent(); $p=New-Object Security.Principal.WindowsPrincipal($i); [pscustomobject]@{sid=$i.User.Value; name=$i.Name; administrator=$p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator); profile=$env:USERPROFILE; local_data=[Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData); session_id=(Get-Process -Id $PID).SessionId}|ConvertTo-Json -Compress`);
  assert.match(identity.sid, /^S-1-5-/);
  assert.equal(identity.administrator, false, 'Run acceptance from the ordinary login token, not an elevated terminal');
  assert(identity.session_id > 0, 'The native product requires the real interactive login session');
  assert.equal(resolve(identity.profile).toLowerCase(), resolve(homedir()).toLowerCase());
  return identity;
}
export async function assertWindowsPrivatePath(path) {
  const actual=await windowsObservation(`$p=${psLiteral(path)};$i=[Security.Principal.WindowsIdentity]::GetCurrent();$a=if([IO.Directory]::Exists($p)){[IO.Directory]::GetAccessControl($p)}else{[IO.File]::GetAccessControl($p)};[pscustomobject]@{sid=$i.User.Value;owner=$a.GetOwner([Security.Principal.SecurityIdentifier]).Value;rules=@($a.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier])|ForEach-Object{[pscustomobject]@{sid=$_.IdentityReference.Value;kind=[string]$_.AccessControlType}})}|ConvertTo-Json -Compress`);
  assert.equal(actual.owner,actual.sid,'Private fixture must be owned by the login user');
  const allowed=new Set([actual.sid,'S-1-5-18','S-1-5-32-544']);
  assert(actual.rules.some(rule=>rule.kind==='Allow'&&rule.sid===actual.sid),'Private fixture must allow its actual owner');
  assert(actual.rules.every(rule=>rule.kind!=='Allow'||allowed.has(rule.sid)),'Private fixture ACL exposes it to another principal');
  return {owner_sid:actual.owner,acl_private:true};
}
export async function readWindowsOwnedFixtureBytes(path,maximum) {
  const before=await lstat(path);assert(before.isFile()&&!before.isSymbolicLink()&&before.nlink===1&&before.size<=maximum,'Expected a bounded regular singly linked private fixture file');
  await assertWindowsPrivatePath(path);
  const handle=await open(path,'r');
  const changed=()=>Object.assign(new Error('Private fixture changed during read'),{code:'NATIVE_FIXTURE_METADATA_CHANGED'});
  try{const opened=await handle.stat();if(opened.ino!==before.ino||opened.dev!==before.dev)throw changed();const bytes=await handle.readFile(),after=await handle.stat(),current=await lstat(path);
    if(!(bytes.length<=maximum&&bytes.length===before.size&&after.size===before.size&&after.mtimeMs===before.mtimeMs&&current.ino===before.ino&&current.dev===before.dev&&current.nlink===1))throw changed();return bytes;
  }finally{await handle.close();}
}
export async function assertWindowsFixture(fixture, workspace) {
  assert.equal(process.platform, 'win32');
  assert.match(basename(fixture), /^nomifun-global-os-product-[0-9a-f-]{36}$/);
  assert.equal((await realpath(fixture)).toLowerCase(), fixture.toLowerCase());
  assert.equal(dirname(fixture).toLowerCase(), (await realpath(tmpdir())).toLowerCase(), 'Only a new host-allocated TEMP fixture is accepted');
  assert.equal(workspace, join(fixture, 'workspace'));
  const info = await lstat(fixture); assert(info.isDirectory() && !info.isSymbolicLink());
}
// Host fixture infrastructure only: called immediately after exclusive mkdir,
// before any workspace, Session, content or product target exists in this root.
export async function initializeWindowsPrivateEmptyFixture(fixture,loginIdentity) {
  await assertWindowsFixture(fixture,join(fixture,'workspace'));
  assert.match(loginIdentity.sid,/^S-1-5-21-(?:[0-9]+-){3}[0-9]+$/);assert.equal(loginIdentity.administrator,false);
  assert.deepEqual(await readdir(fixture),[],'Only a newly allocated empty fixture may receive the fixture ACL');
  const before=await lstat(fixture),sid=psLiteral(loginIdentity.sid);
  await windowsObservation(`$p=${psLiteral(fixture)};$i=[Security.Principal.WindowsIdentity]::GetCurrent();if($i.User.Value -cne ${sid}){throw 'Fixture owner identity changed'};if([IO.Directory]::GetFileSystemEntries($p).Length-ne0){throw 'Fixture is no longer empty'};$a=New-Object Security.AccessControl.DirectorySecurity;$a.SetOwner($i.User);$a.SetAccessRuleProtection($true,$false);foreach($s in @(${sid},'S-1-5-18','S-1-5-32-544')){$r=New-Object Security.AccessControl.FileSystemAccessRule((New-Object Security.Principal.SecurityIdentifier($s)),'FullControl','ContainerInherit,ObjectInherit','None','Allow');$a.AddAccessRule($r)};[IO.Directory]::SetAccessControl($p,$a);[pscustomobject]@{applied=$true}|ConvertTo-Json -Compress`);
  const after=await lstat(fixture);assert.equal(after.dev,before.dev);assert.equal(after.ino,before.ino);assert.deepEqual(await readdir(fixture),[]);
  return assertWindowsPrivatePath(fixture);
}
const execStep = (id, script, verify, extra = {}) => ({ id, name: 'start_process', args: { command: powershell, args: psArguments(script), timeout_ms: 20000 },
  verify: async value => { exited(value); await verify?.(value); }, ...extra });

export function createWindowsFilesScenario({ fixture, workspace, outside, initial, changed }) {
  const file = join(outside, 'sample.txt'), renamed = join(outside, 'renamed.txt');
  const canonical=path=>path.startsWith('\\\\?\\')?path:path.startsWith('\\\\')?`\\\\?\\UNC\\${path.slice(2)}`:`\\\\?\\${path}`;
  let processId, pid, cursor = 0, output = '';
  const nativeEvidence = [];
  const cancel = { id: 'cancel', name: 'cancel_process', args: () => ({ process_id: processId }), verify: value => {
    assert.equal(value.state, 'cancelled'); assert.equal(value.cleanup.reaped, true); cursor = value.output.next_cursor;
  } };
  const end = { id: 'poll-exit', name: 'poll_process', allowCancelledObservation: true, args: () => ({ process_id: processId, cursor, wait_ms: 0 }), verify: value => {
    assert.equal(value.state, 'cancelled'); assert.equal(value.cleanup.reaped, true); assert(windowsProcessGone(pid));
    nativeEvidence.push({ phase: 'exact_pid_exit', pid, gone: true });
  } };
  const steps = [
    { id: 'discover-file-instructions', name: 'read_file', args: { path: file, format: 'instruction_scope' }, verify: value => {
      assert.equal(value.observation_kind, 'instruction_scope'); assert.equal(value.complete, true);
    } },
    { id: 'create', name: 'write_file', args: { path: file, content: initial }, verify: async value => {assert.equal(await readFile(file, 'utf8'), initial);assert.equal(value.native_path,canonical(file));assert.equal(value.workspace_path,null);nativeEvidence.push({phase:'external_file_write_receipt',native_path:value.native_path,workspace_path:value.workspace_path});} },
    { id: 'read-before', name: 'read_file', args: { path: `${workspace}\\..\\${relative(fixture, file)}` }, verify: value => assert.equal(value.content, initial) },
    { id: 'patch', name: 'apply_patch', args: { files: [{ path: file, expected_source: { kind: 'existing', sha256: createHash('sha256').update(initial).digest('hex') }, hunks: [{ old_start: 1, old_lines: 1, new_start: 1, new_lines: 1, lines: [{ kind: 'remove', text: initial.trimEnd() }, { kind: 'add', text: changed.trimEnd() }] }] }] }, verify: async () => assert.equal(await readFile(file, 'utf8'), changed) },
    { id: 'read-after', name: 'read_file', args: { path: file }, verify: value => assert.equal(value.content, changed) },
    { id: 'search', name: 'search_files', args: { path: outside, query: changed.trimEnd() }, verify: value => assert(JSON.stringify(value).includes('sample.txt')) },
    { id: 'rename', name: 'rename_path', args: { path: file, destination: renamed }, verify: async value => {
      assert(await absent(file)); assert.equal(await readFile(renamed, 'utf8'), changed);
      assert.equal(value.path,canonical(file));assert.equal(value.destination,canonical(renamed));assert.equal(value.source_workspace_path,null);assert.equal(value.destination_workspace_path,null);nativeEvidence.push({phase:'external_file_rename_receipt',source:value.path,destination:value.destination,source_workspace_path:null,destination_workspace_path:null});
    } },
    { id: 'start', name: 'start_process', awaitExit: false, args: { command: powershell, args: psArguments(`[Console]::WriteLine((Get-Location).ProviderPath); [Console]::Write([IO.File]::ReadAllText('renamed.txt')); [Console]::WriteLine('WAITING'); Start-Sleep -Seconds 60`), cwd: outside, wait_ms: 0, timeout_ms: 60000 }, verify: value => {
      assert.equal(value.state, 'running'); processId = value.process_id; pid = value.pid; cursor = value.output.next_cursor; output = value.output.text;
      assert(processId && Number.isSafeInteger(pid)); assert(!windowsProcessGone(pid));
    } },
    { id: 'poll-running', name: 'poll_process', repeatable: true, maxAttempts: 12, args: () => ({ process_id: processId, cursor, wait_ms: 1000 }), verify: value => {
      assert.equal(value.state, 'running'); cursor = value.output.next_cursor; output += value.output.text; assert(!windowsProcessGone(pid));
      if (!output.includes('WAITING')) return false;
      assert.equal(normalized(output), `${outside}\n${changed}WAITING\n`);
      nativeEvidence.push({ phase: 'ordinary_process', pid, cwd: outside, output_verified: true });
    } }, cancel, end,
    { id: 'delete', name: 'delete_path', args: { path: outside }, verify: async value => {assert(await absent(outside));assert.equal(value.native_path,canonical(outside));assert.equal(value.workspace_path,null);nativeEvidence.push({phase:'external_file_delete_receipt',native_path:value.native_path,workspace_path:null});} },
  ];
  return { steps, resources: { root: outside }, nativeEvidence,
    prompt: `只操作本次随机工作区外目录 ${outside}：canonical文件工具写入/read/patch/search/rename；再从该目录启动系统PowerShell等待进程，独立poll核对真实cwd和内容，cancel/reaped及确切PID退出后删除本次目录。不得使用既有文件或服务。`,
    completionSummary: role => role==='main'?`本次Windows普通文件和进程验收经默认Agent现役工具完成：工作区外写入、相对..读取、patch、搜索、无覆盖rename均由真实磁盘复核；实际cwd与中文内容经独立running poll核对，取消和末次poll证明reaped，确切PID ${pid} 已退出，唯一测试目录已删除。本机scripted provider不代表联网模型推理。`:`原普通文件验收失败保留。本次恢复Turn只删除唯一 ${outside}；原Turn取消和进程清理已分别结算，不据恢复删除声称原业务步骤成功。`,
    cleanupNeeded: async () => !await absent(outside),
    cleanupPrompt: `原验收失败保持失败；先前Turn已取消且原生清理、pending/unknown效果均已结算。只用delete_path清理本次随机目录 ${outside}。`,
    cleanupSteps: [{ id: 'files-cleanup', name: 'delete_path', args: { path: outside }, verify: async () => assert(await absent(outside)) }],
    cleanupIntent: { temporary_process: () => ({ process_id: processId, pid }), path: outside, only_after_canonical_settlement: true },
    pendingAcceptance: [{ id: 'volumes-unc-acl', status: 'not_run', reason: 'This case proves only its actual TEMP volume and ordinary file/process path; external/UNC volumes and administrator ACL fixtures are separate cases.' }],
  };
}

export function createWindowsAdministratorScenario({ fixture, root, instructions, instructionContent, marker, markerValue, instructionEvidence, verifyInstructions, loginIdentity }) {
  let processId, pid, creationFiletime, cursor = 0, output = '', exitVerified = false;
  const nativeEvidence = [], evidence = { expected_elevated: true, expected_cwd: root, expected_login_home: homedir(), identity_and_exit_observations: nativeEvidence };
  const admin = script => ({ command: powershell, args: psArguments(script), execution_identity: 'administrator', timeout_ms: 20000 });
  const base = `if (-not (New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {throw 'Expected elevated native token'}; `;
  const acl = `$acl=New-Object Security.AccessControl.DirectorySecurity; $ba=New-Object Security.Principal.SecurityIdentifier('S-1-5-32-544'); $sy=New-Object Security.Principal.SecurityIdentifier('S-1-5-18'); $acl.SetOwner($ba); $acl.SetAccessRuleProtection($true,$false); foreach($sid in @($ba,$sy)){$rule=New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow'); $acl.AddAccessRule($rule)}; `;
  const cleanupScript = `${base}$root=${psLiteral(root)}; if(Test-Path -LiteralPath $root){$a=[IO.Directory]::GetAccessControl($root); if($a.GetOwner([Security.Principal.SecurityIdentifier]).Value -ne 'S-1-5-32-544'){throw 'Fixture ACL owner changed'}; foreach($p in @(${psLiteral(marker)},${psLiteral(instructions)})){if(Test-Path -LiteralPath $p){Remove-Item -LiteralPath $p -Force}}; if(@(Get-ChildItem -Force -LiteralPath $root).Count){throw 'Unexpected fixture contents; preserve'}; Remove-Item -LiteralPath $root -Force}`;
  const steps = [
    { id: 'admin-fixture', name: 'start_process', args: admin(`${base}$root=${psLiteral(root)}; if(Test-Path -LiteralPath $root){throw 'Fixture collision'}; ${acl}[IO.Directory]::CreateDirectory($root,$acl)|Out-Null; [IO.File]::WriteAllText(${psLiteral(instructions)},${psLiteral(instructionContent)},(New-Object Text.UTF8Encoding($false))); [pscustomobject]@{elevated=$true; sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value; sddl=([IO.Directory]::GetAccessControl($root)).GetSecurityDescriptorSddlForm('All'); instructions_sha256=([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([IO.File]::ReadAllBytes(${psLiteral(instructions)})))).Replace('-','').ToLowerInvariant()}|ConvertTo-Json -Compress`), verify: async value => {
      exited(value); const result = json(value); assert.equal(result.elevated, true); assert.match(result.sddl, /^O:BA/); assert(result.sddl.includes('D:P'));
      assert.equal(result.instructions_sha256, createHash('sha256').update(instructionContent).digest('hex'));
      const denied = join(root, `ordinary-must-fail-${randomUUID()}`);
      await assert.rejects(writeFile(denied, 'must not be written', { flag: 'wx' }), error => ['EACCES', 'EPERM'].includes(error.code));
      await assert.rejects(readFile(instructions), error => ['EACCES', 'EPERM'].includes(error.code));
      nativeEvidence.push({ phase: 'ordinary_acl_denied', elevated: false, write_denied: true, read_denied: true, sddl: result.sddl });
      instructionEvidence.expected_uid = undefined; instructionEvidence.expected_mode = undefined; instructionEvidence.expected_acl = 'protected Administrators/SYSTEM full control';
    } },
    { id: 'admin-write-read', name: 'start_process', args: admin(`${base}$p=${psLiteral(marker)}; $f=[IO.File]::Open($p,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None); try{$b=[Text.Encoding]::UTF8.GetBytes(${psLiteral(markerValue)});$f.Write($b,0,$b.Length)}finally{$f.Dispose()}; [pscustomobject]@{elevated=$true; content=[IO.File]::ReadAllText($p); sddl=([IO.File]::GetAccessControl($p)).GetSecurityDescriptorSddlForm('All')}|ConvertTo-Json -Compress`), verify: value => {
      exited(value); const result=json(value); assert.equal(result.elevated,true); assert.equal(result.content,markerValue); assert(result.sddl.includes('BA')); nativeEvidence.push({phase:'administrator_marker',...result});
    } },
    { id: 'admin-process-start', name: 'start_process', awaitExit: false, repeatable: true, maxAttempts: 3, allowInstructionRefresh: true,
      args: { ...admin(`${base}[pscustomobject]@{elevated=$true; sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value; cwd=(Get-Location).ProviderPath; profile=$env:USERPROFILE; path=$env:PATH; session_id=(Get-Process -Id $PID).SessionId; pid=$PID}|ConvertTo-Json -Compress; [Console]::WriteLine('NOMIFUN_ADMIN_WAITING'); Start-Sleep -Seconds 60`), cwd: root, env: { PATH: root }, wait_ms: 0, timeout_ms: 60000 }, verify: async (value, context) => {
        assert.equal(value.state,'running'); processId=value.process_id; pid=value.pid; cursor=value.output.next_cursor; output=value.output.text;
        Object.assign(evidence,{process_id:processId,pid,execution_identity:'administrator'});
        instructionEvidence.observations.push({phase:'successful_start_proposal_context',...await verifyInstructions(context.callId)});
      } },
    { id: 'admin-process-poll-running', name: 'poll_process', repeatable: true, maxAttempts: 12, args: () => ({process_id:processId,cursor,wait_ms:1000}), verify: async value => {
      assert.equal(value.state,'running'); cursor=value.output.next_cursor; output+=value.output.text;
      if(!output.includes('NOMIFUN_ADMIN_WAITING'))return false;
      const lines=normalized(output).trim().split('\n'); assert.equal(lines.length,2); assert.equal(lines[1],'NOMIFUN_ADMIN_WAITING'); const identity=JSON.parse(lines[0]);
      assert.equal(identity.elevated,true); assert.equal(identity.cwd.toLowerCase(),root.toLowerCase()); assert.equal(identity.profile.toLowerCase(),homedir().toLowerCase());
      assert(typeof loginIdentity?.local_data==='string'&&loginIdentity.local_data.length>0,'The user CLI fallback must derive from the actual native LocalAppData known folder');
      const pathEntries=identity.path.split(';'),cli=join(loginIdentity.local_data,'nomifun-user-tools','npm');assert.deepEqual(pathEntries,[root,cli],'Explicit PATH discovery must come first, followed only by the exact durable user CLI fallback; never inherit host PATH entries');
      assert.equal(identity.pid,pid); assert(identity.session_id>0);nativeEvidence.push({phase:'administrator_explicit_path',entries:pathEntries,explicit_first:root,durable_cli_fallback:cli,host_path_entries:[]});
      const observed=await windowsObservation(`$p=Get-CimInstance Win32_Process -Filter ${psLiteral(`ProcessId=${pid}`)}; if($null -eq $p){throw 'Expected exact live process'}; $o=Invoke-CimMethod -InputObject $p -MethodName GetOwnerSid; [pscustomobject]@{pid=$p.ProcessId; sid=$o.Sid; return_value=$o.ReturnValue; session_id=$p.SessionId}|ConvertTo-Json -Compress`);
      assert.equal(observed.pid,pid); assert.equal(observed.return_value,0); assert.equal(observed.sid,identity.sid);const processObservation=windowsProcessObservation(pid);assert.equal(processObservation.state,'running');creationFiletime=processObservation.creation_filetime;nativeEvidence.push({phase:'administrator_process_identity',...identity,independent:observed,native_process:processObservation});
    } },
    { id: 'admin-process-cancel', name: 'cancel_process', args: () => ({process_id:processId}), verify: value => {assert.equal(value.state,'cancelled');assert.equal(value.cleanup.reaped,true);cursor=value.output.next_cursor;} },
    { id: 'admin-process-poll-exit', name: 'poll_process', allowCancelledObservation: true, args: () => ({process_id:processId,cursor,wait_ms:0}), verify: value => {assert.equal(value.state,'cancelled');assert.equal(value.cleanup.reaped,true);const observed=windowsProcessObservation(pid);assert.notEqual(observed.state,'running');if(observed.state==='exited')assert.equal(observed.creation_filetime,creationFiletime);exitVerified=true;nativeEvidence.push({phase:'exact_administrator_pid_exit',pid,original_creation_filetime:creationFiletime,gone:true,native_process:observed});} },
    { id: 'admin-source-read', name: 'read_file', args: { path: marker, execution_identity: 'administrator' }, verify: value => {
      assert.equal(value.content, markerValue); assert.equal(value.sha256, createHash('sha256').update(markerValue).digest('hex'));
    } },
    { id: 'admin-clean', name: 'start_process', args: admin(cleanupScript), verify: async value => {assert(exitVerified);exited(value);assert(await absent(root));} },
  ];
  return { steps, resources:{root,marker,instructions}, nativeEvidence, administratorEvidence:evidence,
    prompt:`只对宿主新建的唯一夹具 ${root} 运行管理员验收：原生UAC路径建立仅Administrators/SYSTEM可写目录和AGENTS.md，普通用户真实写读拒绝，再管理员写标记、同一read_file只读获取。管理员cwd指令必须真实预检，延期后fresh call，再启动管理员等待进程核对elevated token、cwd和真实登录USERPROFILE，poll/cancel/reaped、确切PID退出后清理唯一夹具。凭据只在系统安全桌面输入，不普通用户降级。`,
    completionSummary:role=>role==='main'?`本次Windows管理员夹具 ${root} 的ACL拒绝普通写读；管理员写标记并以正式read_file读取。管理员AGENTS预检和InstructionsUpdated先于fresh start提案，等待进程PID ${pid} 的elevated token/cwd/登录USERPROFILE及独立GetOwnerSid一致。cancel及terminal poll为cancelled/reaped，确切PID退出后删除本次夹具。证据仅覆盖本次真实调用；scripted provider不代表联网模型推理。`:`原管理员验收失败保留。本次恢复Turn只清理本次唯一 ${root} 的已知marker和AGENTS并确认目录不存在；原Turn和进程清理另有canonical结算，不据恢复删除声称原管理员业务步骤成功。`,
    cleanupNeeded:async()=>!await absent(root), cleanupPrompt:`原管理员验收失败保持失败。原Turn已取消且临时进程真实清理、pending/unknown均结算后，只清理唯一 ${root} 中本次AGENTS和marker，ACL owner或额外内容变化则保留，不触碰任何其他对象。`,
    cleanupSteps:[{id:'admin-recovery-clean',name:'start_process',args:admin(cleanupScript),verify:async value=>{exited(value);assert(await absent(root));}}],
    cleanupIntent:{temporary_process:()=>({process_id:processId,pid}),path:root,only_after_canonical_settlement:true},
    pendingAcceptance:[{id:'uac-denial-disconnect-over-the-shoulder',status:'not_run',reason:'This successful-operation case does not prove UAC deny/cancel, transport loss, or authentication as a different account.'}],
  };
}

export async function createWindowsSystemScenario({ fixture, workspace, loginIdentity, preserveServiceForHostExit = false }) {
  await assertWindowsFixture(fixture, workspace);
  assert.equal(loginIdentity.administrator, false);
  const nonce=randomUUID(), root=join(fixture,`system-cases-${nonce}`), config=join(root,'config.json');
  const registryPath=`Software\\NomiFun\\Acceptance\\${nonce}`, taskName=`NomiFun-Global-OS-${nonce}`, taskPath=`\\${taskName}`;
  const serviceMarker=join(root,'task-marker.json'), taskScript=join(root,'task.ps1'), listenerScript=join(root,'listener.ps1');
  const userToolName=`nomifun-global-os-tool-${nonce}.ps1`, userToolSource=join(root,'source.ps1'), userToolInstalled=join(root,'bin',userToolName);
  const userToolArgs=['','中文 空格',`single' and "double" quote`,'% ! & | < > ^ $ $(throw "NOT_LITERAL")','back\\slash','trailing\\'];
  const toolContent=`$ProgressPreference='SilentlyContinue'\n[Console]::OutputEncoding=New-Object Text.UTF8Encoding($false)\nConvertTo-Json -InputObject @($args) -Compress\n`;
  const initial=`${JSON.stringify({fixture:nonce,state:'before'})}\n`, changed=initial.replace('"before"','"after"');
  const loopbackPort=await allocateLoopbackPort(), networkPath=`/nomifun-${nonce}`, networkBody=`NOMIFUN_LOOPBACK_${nonce}\n`;
  const nativeEvidence=[], resources={root,config,target:taskPath,taskName,registryPath,serviceMarker,userToolName,userToolInstalled,loopbackPort,networkPath,retainedService:preserveServiceForHostExit};
  let taskPid, networkPid, networkProcessId, cursor=0, output='', registryAbsentBefore=false, taskAbsentBefore=false;
  const taskConnect=`$scheduler=New-Object -ComObject 'Schedule.Service';$scheduler.Connect();$folder=$scheduler.GetFolder('\\');`;
  const taskQuery=`${taskConnect}$t=$null;try{$t=$folder.GetTask(${psLiteral(taskName)})}catch{if($_.Exception.HResult -notin @(-2147024894,-2147024893)){throw}};`;
  const observeTask=async absentExpected=>{
    const observation=await windowsObservation(`${taskQuery}$v=if($null -eq $t){[pscustomobject]@{exists=$false;path=${psLiteral(taskPath)}}}else{[pscustomobject]@{exists=$true;path=$t.Path;state=$t.State;instances=@($t.GetInstances(0)).Count}};$v|ConvertTo-Json -Compress`);
    nativeEvidence.push({phase:absentExpected?'task_absent':'task_registered',...observation});assert.equal(observation.path,taskPath);assert.equal(observation.exists,!absentExpected);
    if(!absentExpected){assert.equal(observation.state,4);assert(observation.instances>0);assert(!windowsProcessGone(taskPid));} return observation;
  };
  const observeRegistry=async expected=>{
    const result=await windowsObservation(`$k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey(${psLiteral(registryPath)});try{[pscustomobject]@{exists=($null-ne$k);value=$(if($k){$k.GetValue('FixtureEnabled')})}|ConvertTo-Json -Compress}finally{if($k){$k.Dispose()}}`);
    nativeEvidence.push({phase:expected?'registry_present':'registry_absent',path:`HKCU\\${registryPath}`,...result});assert.equal(result.exists,expected);if(expected)assert.equal(result.value,1);
  };
  const observeListener=async()=>{
    const result=await windowsObservation(`$v=@(Get-NetTCPConnection -State Listen -LocalAddress '127.0.0.1' -LocalPort ${loopbackPort} -ErrorAction SilentlyContinue); ConvertTo-Json -InputObject @($v|ForEach-Object{[pscustomobject]@{pid=$_.OwningProcess;address=$_.LocalAddress;port=$_.LocalPort}}) -Compress`);
    if(!result.length)return false; assert.equal(result.length,1);assert.equal(result[0].pid,networkPid);assert.equal(result[0].address,'127.0.0.1');nativeEvidence.push({phase:'loopback_listener',...result[0]});return true;
  };
  const stopTaskScript=`${taskQuery}if($t){$t.Stop(0);for($i=0;$i-lt50;$i++){if(@($t.GetInstances(0)).Count-eq0){break};Start-Sleep -Milliseconds 100};if(@($t.GetInstances(0)).Count){throw 'Exact fixture task still running'};$folder.DeleteTask(${psLiteral(taskName)},0)}; [Console]::WriteLine('NOMIFUN_TASK_REMOVED')`;
  const stopTask=execStep('system-service-stop-delete',stopTaskScript,async value=>{assert.equal(value.output.text.trim(),'NOMIFUN_TASK_REMOVED');await observeTask(true);if(taskPid)assert(windowsProcessGone(taskPid));});
  const registryCleanup=execStep('system-registry-recovery',`$k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey(${psLiteral(registryPath)});if($k){$k.Dispose();${registryAbsentBefore?'':'throw \'Initial registry absence was not proven\';'}[Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree(${psLiteral(registryPath)},$false)}`,()=>observeRegistry(false),{args:()=>({command:powershell,args:psArguments(`$k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey(${psLiteral(registryPath)});if($k){$k.Dispose();${registryAbsentBefore?'':'throw \'Initial registry absence was not proven\';'}[Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree(${psLiteral(registryPath)},$false)}`),timeout_ms:20000})});
  const deleteFiles={id:'system-files-delete',name:'delete_path',args:{path:root},verify:async()=>assert(await absent(root))};
  const fileWrite=(id,path,content)=>({id,name:'write_file',args:{path,content},verify:async()=>assert.equal(await readFile(path,'utf8'),content)});
  const preparation=[
    {id:'system-instruction-scope',name:'read_file',args:{path:fixture,format:'instruction_scope',recursive:true},verify:v=>{assert.equal(v.observation_kind,'instruction_scope');assert.equal(v.complete,true);}},
    execStep('system-preflight',`[pscustomobject]@{sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value;profile=$env:USERPROFILE}|ConvertTo-Json -Compress`,async value=>{assert.equal(json(value).sid,loginIdentity.sid);assert(await absent(root));await observeTask(true);taskAbsentBefore=true;await observeRegistry(false);registryAbsentBefore=true;}),
    fileWrite('system-config-write',config,initial),
    {id:'system-config-patch',name:'apply_patch',args:{files:[{path:config,expected_source:{kind:'existing',sha256:createHash('sha256').update(initial).digest('hex')},hunks:[{old_start:1,old_lines:1,new_start:1,new_lines:1,lines:[{kind:'remove',text:initial.trimEnd()},{kind:'add',text:changed.trimEnd()}]}]}]},verify:async()=>assert.equal(await readFile(config,'utf8'),changed)},
    {id:'system-config-read',name:'read_file',args:{path:config},verify:value=>assert.equal(value.content,changed)},
    fileWrite('system-user-tool-source',userToolSource,toolContent),
    execStep('system-user-tool-install',`$p=${psLiteral(userToolInstalled)};if(Test-Path -LiteralPath $p){throw 'Install target collision'};[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($p))|Out-Null;[IO.File]::Copy(${psLiteral(userToolSource)},$p,$false)`,async()=>{assert.equal(await readFile(userToolInstalled,'utf8'),toolContent);nativeEvidence.push({phase:'user_tool_installed',path:userToolInstalled,sha256:createHash('sha256').update(toolContent).digest('hex')});}),
    {id:'system-user-tool-bare-literal',name:'start_process',args:{command:userToolName,args:userToolArgs,env:{PATH:`${dirname(userToolInstalled)};${dirname(powershell)};${join(process.env.SystemRoot,'System32')}`},timeout_ms:20000},verify:value=>{exited(value);assert.deepEqual(json(value),userToolArgs);}},
    {id:'system-user-tool-uninstall',name:'delete_path',args:{path:userToolInstalled},verify:async()=>assert(await absent(userToolInstalled))},
  ];
  const scheduledScript=`$ErrorActionPreference='Stop'\n$ProgressPreference='SilentlyContinue'\n[Console]::OutputEncoding=New-Object Text.UTF8Encoding($false)\n$v=[pscustomobject]@{fixture=${psLiteral(nonce)};sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value;pid=$PID;profile=$env:USERPROFILE;session_id=(Get-Process -Id $PID).SessionId}\n[IO.File]::WriteAllText(${psLiteral(serviceMarker)},($v|ConvertTo-Json -Compress),(New-Object Text.UTF8Encoding($false)))\nStart-Sleep -Seconds 900\n`;
  const service=[
    execStep('system-registry-write',`$k=[Microsoft.Win32.Registry]::CurrentUser.CreateSubKey(${psLiteral(registryPath)});try{$k.SetValue('FixtureEnabled',1,[Microsoft.Win32.RegistryValueKind]::DWord)}finally{$k.Dispose()}`,()=>observeRegistry(true)),
    execStep('system-registry-read',`$k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey(${psLiteral(registryPath)});try{[Console]::WriteLine($k.GetValue('FixtureEnabled'))}finally{$k.Dispose()}`,value=>assert.equal(value.output.text.trim(),'1')),
    execStep('system-registry-delete',`[Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree(${psLiteral(registryPath)},$false)`,()=>observeRegistry(false)),
    fileWrite('system-task-script',taskScript,scheduledScript),
    execStep('system-task-register',`${taskConnect}$d=$scheduler.NewTask(0);$d.RegistrationInfo.Description=${psLiteral(`NomiFun isolated acceptance ${nonce}`)};$d.Principal.UserId=${psLiteral(loginIdentity.sid)};$d.Principal.LogonType=3;$d.Principal.RunLevel=0;$d.Settings.ExecutionTimeLimit='PT15M';$d.Settings.DisallowStartIfOnBatteries=$false;$d.Settings.StopIfGoingOnBatteries=$false;$a=$d.Actions.Create(0);$a.Path=${psLiteral(powershell)};$a.Arguments=${psLiteral(`-NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -File "${taskScript}"`)};$a.WorkingDirectory=${psLiteral(root)};$folder.RegisterTaskDefinition(${psLiteral(taskName)},$d,2,${psLiteral(loginIdentity.sid)},$null,3,$null)|Out-Null`,()=>assert(taskAbsentBefore)),
    execStep('system-task-start',`${taskConnect}$t=$folder.GetTask(${psLiteral(taskName)});$t.Run($null)|Out-Null`,async()=>{
      let marker;for(let i=0;i<50;i++){if(!await absent(serviceMarker)){marker=JSON.parse(await readFile(serviceMarker,'utf8'));break;}await delay(100);}assert(marker,'Registered task produced no bounded fixture marker');assert.equal(marker.fixture,nonce);assert.equal(marker.sid,loginIdentity.sid);assert.equal(marker.profile.toLowerCase(),homedir().toLowerCase());assert(marker.session_id>0);taskPid=marker.pid;assert(Number.isSafeInteger(taskPid));resources.task_pid=taskPid;await observeTask(false);nativeEvidence.push({phase:'task_actual_identity',...marker});
    }),
    {id:'system-task-marker-read',name:'read_file',args:{path:serviceMarker},verify:value=>{const marker=JSON.parse(value.content);assert.equal(marker.pid,taskPid);assert.equal(marker.fixture,nonce);}},
  ];
  const listenerContent=`$ErrorActionPreference='Stop'\n[Console]::OutputEncoding=New-Object Text.UTF8Encoding($false)\n$l=New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback,${loopbackPort});$l.Start();try{[Console]::WriteLine('NOMIFUN_LISTENING');$c=$l.AcceptTcpClient();try{$s=$c.GetStream();$r=New-Object IO.StreamReader($s);$line=$r.ReadLine();[Console]::WriteLine($line);while($r.ReadLine()-ne''){};$body=[Text.Encoding]::UTF8.GetBytes(${psLiteral(networkBody)});$h=[Text.Encoding]::ASCII.GetBytes("HTTP/1.1 200 OK\`r\`nContent-Type: text/plain\`r\`nContent-Length: $($body.Length)\`r\`nConnection: close\`r\`n\`r\`n");$s.Write($h,0,$h.Length);$s.Write($body,0,$body.Length);$s.Flush()}finally{$c.Dispose()};Start-Sleep -Seconds 60}finally{$l.Stop()}\n`;
  const network=[
    fileWrite('system-network-script',listenerScript,listenerContent),
    {id:'system-network-start',name:'start_process',awaitExit:false,args:{command:powershell,args:['-NoLogo','-NoProfile','-NonInteractive','-File',listenerScript],cwd:root,wait_ms:0,timeout_ms:60000},verify:value=>{assert.equal(value.state,'running');networkProcessId=value.process_id;networkPid=value.pid;cursor=value.output.next_cursor;output=value.output.text;assert(!windowsProcessGone(networkPid));}},
    {id:'system-network-poll-running',name:'poll_process',repeatable:true,maxAttempts:12,args:()=>({process_id:networkProcessId,cursor,wait_ms:200}),verify:async value=>{assert.equal(value.state,'running');cursor=value.output.next_cursor;output+=value.output.text;if(!output.includes('NOMIFUN_LISTENING'))return false;return observeListener();}},
    {id:'system-network-request',name:'start_process',args:{command:join(process.env.SystemRoot,'System32','curl.exe'),args:['--noproxy','*','--fail','--silent','--show-error','--max-time','10',`http://127.0.0.1:${loopbackPort}${networkPath}`],timeout_ms:15000},verify:value=>{exited(value);assert.equal(value.output.text,networkBody);}},
    {id:'system-network-stop',name:'cancel_process',args:()=>({process_id:networkProcessId}),verify:value=>{assert.equal(value.state,'cancelled');assert.equal(value.cleanup.reaped,true);cursor=value.output.next_cursor;output+=value.output.text;}},
    {id:'system-network-confirm-exit',name:'poll_process',allowCancelledObservation:true,args:()=>({process_id:networkProcessId,cursor,wait_ms:0}),verify:async value=>{assert.equal(value.state,'cancelled');assert.equal(value.cleanup.reaped,true);assert(windowsProcessGone(networkPid));assert(output.includes(`GET ${networkPath} HTTP/1.1`));assert.equal(await observeListener(),false);}},
  ];
  const finalization=[fileWrite('system-config-restore',config,initial),{id:'system-config-restored-read',name:'read_file',args:{path:config},verify:value=>assert.equal(value.content,initial)},...preserveServiceForHostExit?[]:[stopTask,deleteFiles]];
  const phases=[{role:'system-install-config',steps:preparation,prompt:`只在本次 ${root} 写入/patch/read配置，安装并以本次PATH裸命令运行唯一PowerShell用户工具，literal argv必须完全一致，再卸载。只读确认唯一注册表键和计划任务不存在，不创建后续任务或监听。`},{role:'system-service',steps:service,prompt:`只在本次UUID注册表HKCU\\${registryPath} 写入/读取/删除恢复，并以当前登录SID注册/运行独占InteractiveToken计划任务 ${taskPath}，读回真实PID/SID/USERPROFILE/session marker及Task Scheduler状态。明确保留该任务用于后续阶段，不操作其他任务、注册表或系统服务。`},{role:'system-network',steps:network,prompt:`在本次目录启动唯一127.0.0.1:${loopbackPort} TcpListener受管进程，独立poll确认同PID监听，curl.exe访问唯一${networkPath}核对响应，cancel/terminal/reaped及PID/监听消失。保留本次计划任务。`},{role:'system-finalize',steps:finalization,prompt:`只恢复并读回本次配置。${preserveServiceForHostExit?'明确保留本次唯一计划任务与文件用于App退出存续测试。':'停止并注销本次唯一计划任务，确认原生任务不存在和已观察PID退出后删除本次文件。'}不重跑先前阶段。`}];
  return {resources,nativeEvidence,steps:phases.flatMap(p=>p.steps),phases,
    prompt:`仅执行本次Windows唯一夹具 ${root} 的四个独立阶段。`,
    completionSummary:role=>`本次Windows隔离系统验收阶段 ${role} 的实际工具回执已核验；唯一计划任务 ${taskPath}，HKCU UUID键 ${registryPath}，loopback 127.0.0.1:${loopbackPort}。只报告当前阶段已经完成的步骤，不把注册、exit0或其他阶段当作本阶段证明。${role==='system-finalize'&&!preserveServiceForHostExit?'本次任务和确切PID已停止/注销且测试目录已删除。':''}`,
    cleanupNeeded:async()=>!await absent(root),cleanupPrompt:`原验收失败保持失败；先前Turn已取消且pending/unknown与临时进程清理已证明。仅删除本次UUID注册表HKCU\\${registryPath}，停止/注销唯一任务 ${taskPath}，确认其确切PID退出后删除 ${root}。不改其他对象。`,
    retainedCleanupPrompt:`本次唯一InteractiveToken计划任务 ${taskPath} 是先前阶段明确保留的测试产物。宿主已证明原App确切PID退出后该任务和PID仍存续，原owner JWT同Session在相同数据根的新App复用。只清理本次UUID注册表、停止注销唯一任务，确认原生不存在和准确PID退出，再删除 ${root}。不重跑主验收。`,cleanupSteps:[registryCleanup,stopTask,deleteFiles],cleanupIntent:{temporary_process:()=>({process_id:networkProcessId,pid:networkPid}),persistent_task:taskPath,registry_key:`HKCU\\${registryPath}`,only_after_canonical_settlement:true},
    pendingAcceptance:[{id:'system-install-scm-security-config',status:'not_run',reason:'This case installs a local user tool, mutates only a UUID HKCU key and uses Task Scheduler. It does not prove system-wide installer/SCM/firewall/proxy/certificate operations.'},{id:'task-survives-app-exit',status:preserveServiceForHostExit?'pending':'not_run',reason:'Only the explicit retained task phase plus actual App exit/restart can prove persistence.'}],
    async observeAfterAppExit({appPid}){assert(preserveServiceForHostExit);assert(windowsProcessGone(appPid));await observeTask(false);return{status:'passed',app_pid:appPid,task_pid:taskPid,task_path:taskPath,cleanup_required:true};},
  };
}

export { allocateLoopbackPort };
