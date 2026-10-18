// Real SCM acceptance through the original default Agent process/file tools.
// The driver only observes its UUID service; all service mutations are tools.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { lstat, readFile } from 'node:fs/promises';
import { join,win32 } from 'node:path';
import { assertWindowsFixture, powershell, psArguments, psLiteral, windowsObservation, windowsProcessObservationScript } from './global-os-windows-cases.mjs';

const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const servicePattern = /^NomiFunAcceptance_[0-9a-f]{32}$/;
const exited = value => {
  assert.equal(value.state, 'exited');
  assert.equal(value.exit_code, 0);
  assert.equal(value.cleanup?.reaped, true);
};
const json = value => JSON.parse(value.output.text.trim());
const missing = path => lstat(path).then(() => false, error => {
  if (error.code === 'ENOENT') return true;
  throw error;
});

export function windowsServiceFixtureSource({ serviceName, nonce, startMarker, stopMarker }) {
  assert.match(serviceName, servicePattern);
  assert.match(nonce, /^[0-9a-f-]{36}$/);
  const literal = value => JSON.stringify(String(value));
  return `using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Security.Principal;
using System.ServiceProcess;
using System.Text;
using System.Web.Script.Serialization;
public sealed class NomiFunFixtureService : ServiceBase {
  public NomiFunFixtureService() { ServiceName = ${literal(serviceName)}; CanStop = true; AutoLog = false; }
  private void Record(string path, string phase) {
    using (WindowsIdentity identity = WindowsIdentity.GetCurrent()) {
      var data = new Dictionary<string, object> {
        {"fixture", ${literal(nonce)}}, {"service_name", ServiceName}, {"phase", phase},
        {"sid", identity.User.Value}, {"pid", Process.GetCurrentProcess().Id},
        {"session_id", Process.GetCurrentProcess().SessionId},
        {"profile", Environment.GetEnvironmentVariable("USERPROFILE")}
      };
      byte[] bytes = new UTF8Encoding(false).GetBytes(new JavaScriptSerializer().Serialize(data));
      using (var output = new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.Read)) {
        output.Write(bytes, 0, bytes.Length); output.Flush(true);
      }
    }
  }
  protected override void OnStart(string[] args) { Record(${literal(startMarker)}, "started"); }
  protected override void OnStop() { Record(${literal(stopMarker)}, "stopped"); }
  public static void Main() { ServiceBase.Run(new NomiFunFixtureService()); }
}
`;
}

export const windowsServiceCompileScript = (source, executable) => `if(Test-Path -LiteralPath ${psLiteral(executable)}){throw 'Fixture executable collision'}; Add-Type -TypeDefinition ([IO.File]::ReadAllText(${psLiteral(source)})) -Language CSharp -ReferencedAssemblies 'System.dll','System.Core.dll','System.ServiceProcess.dll','System.Web.Extensions.dll' -OutputAssembly ${psLiteral(executable)} -OutputType WindowsApplication; [pscustomobject]@{path=${psLiteral(executable)};sha256=([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([IO.File]::ReadAllBytes(${psLiteral(executable)})))).Replace('-','').ToLowerInvariant()}|ConvertTo-Json -Compress`;
export function windowsServiceProtectedCopyScript({root,compiledExecutable,executable,expectedSha256}){
  assert.match(expectedSha256,/^[0-9a-f]{64}$/);assert(win32.isAbsolute(root));assert.equal(win32.dirname(executable).toLowerCase(),root.toLowerCase());assert.notEqual(win32.dirname(compiledExecutable).toLowerCase(),root.toLowerCase(),'Ordinary compilation must never observe the protected service directory');
  return `$root=${psLiteral(root)};$source=${psLiteral(compiledExecutable)};$exe=${psLiteral(executable)};$expected=${psLiteral(expectedSha256)};if(Test-Path -LiteralPath $root){throw 'Protected service directory collision'};$hash=([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([IO.File]::ReadAllBytes($source)))).Replace('-','').ToLowerInvariant();if($hash-cne$expected){throw 'Compiled service source hash changed'};$ba=New-Object Security.Principal.SecurityIdentifier('S-1-5-32-544');$sy=New-Object Security.Principal.SecurityIdentifier('S-1-5-18');$acl=New-Object Security.AccessControl.DirectorySecurity;$acl.SetOwner($ba);$acl.SetAccessRuleProtection($true,$false);foreach($sid in @($ba,$sy)){$acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow')))};[IO.Directory]::CreateDirectory($root,$acl)|Out-Null;if((Get-Item -Force -LiteralPath $root).Attributes-band[IO.FileAttributes]::ReparsePoint){throw 'Protected root became a link'};[IO.File]::Copy($source,$exe,$false);$fileAcl=New-Object Security.AccessControl.FileSecurity;$fileAcl.SetOwner($ba);$fileAcl.SetAccessRuleProtection($true,$false);foreach($sid in @($ba,$sy)){$fileAcl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','Allow')))};[IO.File]::SetAccessControl($exe,$fileAcl);$directory=[IO.Directory]::GetAccessControl($root);$file=[IO.File]::GetAccessControl($exe);$rules={param($a)@($a.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier])|ForEach-Object{[pscustomobject]@{sid=$_.IdentityReference.Value;kind=$_.AccessControlType.ToString();full_control=(($_.FileSystemRights-band[Security.AccessControl.FileSystemRights]::FullControl)-eq[Security.AccessControl.FileSystemRights]::FullControl)}})};[pscustomobject]@{source_sha256=$hash;sha256=([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([IO.File]::ReadAllBytes($exe)))).Replace('-','').ToLowerInvariant();owner_sid=$directory.GetOwner([Security.Principal.SecurityIdentifier]).Value;protected=$directory.AreAccessRulesProtected;rules=@(&$rules $directory);file_owner_sid=$file.GetOwner([Security.Principal.SecurityIdentifier]).Value;file_protected=$file.AreAccessRulesProtected;file_rules=@(&$rules $file);sddl=$directory.GetSecurityDescriptorSddlForm('All')}|ConvertTo-Json -Depth 6 -Compress`;
}

export async function createWindowsServiceScenario({ fixture, workspace, loginIdentity }) {
  await assertWindowsFixture(fixture, workspace);
  assert.match(loginIdentity.sid, /^S-1-5-/);
  const nonce = randomUUID(), serviceName = `NomiFunAcceptance_${nonce.replaceAll('-', '')}`;
  const root = join(fixture, `scm-${nonce}`),buildRoot=join(fixture,`scm-build-${nonce}`),source=join(buildRoot,'service.cs'),compiledExecutable=join(buildRoot,'service.exe'),executable=join(root,'service.exe');
  const startMarker = join(root, 'started.json'), stopMarker = join(root, 'stopped.json');
  const sc = join(process.env.SystemRoot, 'System32', 'sc.exe');
  const nativeEvidence = [], resources = { root,buildRoot,source,compiledExecutable,serviceName, executable, startMarker, stopMarker };
  let pid,processCreation,executableSha256,absentBefore = false,created = false,removed = false;
  const queryScript = `$items=@(Get-CimInstance Win32_Service -Filter ${psLiteral(`Name='${serviceName}'`)}); ConvertTo-Json -InputObject @($items|ForEach-Object{[pscustomobject]@{name=$_.Name;state=$_.State;start_mode=$_.StartMode;start_name=$_.StartName;process_id=$_.ProcessId;path=$_.PathName;error_control=$_.ErrorControl}}) -Compress`;
  const observe = async expected => {
    const items = await windowsObservation(queryScript);
    assert(Array.isArray(items));
    if (!expected) { assert.equal(items.length, 0); return null; }
    assert.equal(items.length, 1);
    const item = items[0];
    assert.equal(item.name, serviceName);
    assert.equal(item.path, `"${executable}"`, 'Only this fixture executable may back the service');
    assert.equal(item.start_name, 'LocalSystem');
    return item;
  };
  const adminStep = (id, script, verify) => ({ id, name: 'start_process', args: { command: powershell, args: psArguments(script), execution_identity: 'administrator', timeout_ms: 20000 }, verify: async value => { exited(value); await verify?.(value); } });
  const scStep = (id, args, verify) => ({ id, name: 'start_process', args: { command: sc, args, execution_identity: 'administrator', timeout_ms: 20000 }, verify: async value => { exited(value); await verify?.(value); } });
  const waitState = async (state, allowAbsent = false) => {
    for (let attempt = 0; attempt < 100; attempt++) {
      const items = await windowsObservation(queryScript);
      if (!items.length && allowAbsent) return null;
      assert.equal(items.length, 1);
      const value = await observe(true);
      if (value.state === state) return value;
      await pause(100);
    }
    throw new Error(`Exact fixture SCM state did not become ${state}`);
  };
  const cleanupScript = `$name=${psLiteral(serviceName)};$root=${psLiteral(root)};$exe=${psLiteral(executable)};$s=@(Get-CimInstance Win32_Service -Filter ${psLiteral(`Name='${serviceName}'`)});if($s.Count-gt1){throw 'Duplicate service identity'};if($s.Count){if($s[0].PathName -cne ('"'+$exe+'"') -or $s[0].StartName -ne 'LocalSystem'){throw 'Service identity changed; preserve'};if($s[0].State -ne 'Stopped'){Stop-Service -Name $name -ErrorAction Stop;$controller=New-Object ServiceProcess.ServiceController($name);try{$controller.WaitForStatus([ServiceProcess.ServiceControllerStatus]::Stopped,[TimeSpan]::FromSeconds(15))}finally{$controller.Dispose()}};& ${psLiteral(sc)} delete $name|Out-Null;if($LASTEXITCODE-ne0){throw 'SCM delete failed'}};for($i=0;$i-lt100;$i++){if(-not(Get-CimInstance Win32_Service -Filter ${psLiteral(`Name='${serviceName}'`)})){break};Start-Sleep -Milliseconds 100};if(Get-CimInstance Win32_Service -Filter ${psLiteral(`Name='${serviceName}'`)}){throw 'Service still registered'};if(Test-Path -LiteralPath $root){$r=Get-Item -Force -LiteralPath $root;if($r.Attributes-band[IO.FileAttributes]::ReparsePoint){throw 'Fixture root became a reparse point'};if(([IO.Directory]::GetAccessControl($root)).GetOwner([Security.Principal.SecurityIdentifier]).Value -notin @('S-1-5-32-544',${psLiteral(loginIdentity.sid)})){throw 'Fixture owner changed; preserve'};foreach($f in @(Get-ChildItem -Force -LiteralPath $root)){if($f.PSIsContainer -or ($f.Attributes-band[IO.FileAttributes]::ReparsePoint) -or $f.Name -notin @('service.exe','started.json','stopped.json')){throw 'Unexpected fixture content; preserve'}};foreach($p in @(${[executable, startMarker, stopMarker].map(psLiteral).join(',')})){if(Test-Path -LiteralPath $p){Remove-Item -LiteralPath $p -Force}};Remove-Item -LiteralPath $root -Force};[Console]::WriteLine('NOMIFUN_SCM_CLEANED')`;
  const buildCleanupScript=`if(Test-Path -LiteralPath ${psLiteral(buildRoot)}){$b=Get-Item -Force -LiteralPath ${psLiteral(buildRoot)};if($b.Attributes-band[IO.FileAttributes]::ReparsePoint){throw 'Build root became a link'};if(([IO.Directory]::GetAccessControl($b.FullName)).GetOwner([Security.Principal.SecurityIdentifier]).Value-cne${psLiteral(loginIdentity.sid)}){throw 'Build owner changed'};foreach($f in @(Get-ChildItem -Force -LiteralPath $b.FullName)){if($f.PSIsContainer-or($f.Attributes-band[IO.FileAttributes]::ReparsePoint)-or$f.Name-notin@('service.cs','service.exe')){throw 'Unexpected build files; preserve'}};foreach($p in @(${psLiteral(source)},${psLiteral(compiledExecutable)})){if(Test-Path -LiteralPath $p){Remove-Item -LiteralPath $p -Force}};Remove-Item -LiteralPath $b.FullName -Force}`;
  const cleanup = {id:'scm-cleanup',name:'start_process',args:()=>({command:powershell,args:psArguments(cleanupScript.replace(";[Console]::WriteLine('NOMIFUN_SCM_CLEANED')",`;${buildCleanupScript};${pid?`$p=(&{${windowsProcessObservationScript(pid)}}|ConvertFrom-Json);if($p.state-eq'running'${processCreation?`-and$p.creation_filetime-ceq${psLiteral(processCreation)}`:''}){throw 'Exact service process still running'};`:''}[Console]::WriteLine('NOMIFUN_SCM_CLEANED')`)),execution_identity:'administrator',timeout_ms:30000}),verify:async value=>{
    exited(value);
    assert.equal(value.output.text.trim(), 'NOMIFUN_SCM_CLEANED');
    await observe(false);
    assert(await missing(root));assert(await missing(buildRoot)); removed = true;
    nativeEvidence.push({ phase: 'scm_deleted_files_removed', service_name: serviceName, pid, exact_pid_gone: pid ? true : undefined });
  }};
  const content = windowsServiceFixtureSource({ serviceName, nonce, startMarker, stopMarker });
  const steps = [
    { id: 'scm-scope', name: 'read_file', args: { path: fixture, format: 'instruction_scope', recursive: false }, verify: value => { assert.equal(value.complete, true); assert.equal(value.observation_kind, 'instruction_scope'); } },
    { id: 'scm-preflight', name: 'start_process', args: { command: powershell, args: psArguments(queryScript), timeout_ms: 20000 }, verify: async value => { exited(value); assert.deepEqual(json(value), []); assert(await missing(root));assert(await missing(buildRoot)); absentBefore = true; } },
    { id: 'scm-source', name: 'write_file', args: { path: source, content }, verify: async () => assert.equal(await readFile(source, 'utf8'), content) },
    { id: 'scm-compile', name: 'start_process', args: { command: powershell, args: psArguments(windowsServiceCompileScript(source, compiledExecutable)), timeout_ms: 30000 }, verify: value => { exited(value); const result = json(value); assert.equal(result.path, compiledExecutable); assert.match(result.sha256, /^[0-9a-f]{64}$/); executableSha256 = result.sha256; resources.executable_sha256 = executableSha256; } },
    {id:'scm-protect-binary',name:'start_process',args:()=>({command:powershell,args:psArguments(windowsServiceProtectedCopyScript({root,compiledExecutable,executable,expectedSha256:executableSha256})),execution_identity:'administrator',timeout_ms:20000}),verify:async value=>{
      exited(value);const result=json(value);assert.equal(result.source_sha256,executableSha256);assert.equal(result.sha256,executableSha256);assert.equal(result.owner_sid,'S-1-5-32-544');assert.equal(result.file_owner_sid,'S-1-5-32-544');assert.equal(result.protected,true);assert.equal(result.file_protected,true);
      for(const rules of [result.rules,result.file_rules]){assert.equal(rules.length,2);assert.deepEqual(rules.map(rule=>rule.sid).sort(),['S-1-5-18','S-1-5-32-544'].sort());assert(rules.every(rule=>rule.kind==='Allow'&&rule.full_control===true));}
      await assert.rejects(readFile(executable),error=>['EACCES','EPERM'].includes(error.code));nativeEvidence.push({phase:'scm_binary_protected_from_creation',sha256:executableSha256,ordinary_compile_root:buildRoot,protected_service_root:root,ordinary_content_read_denied:true,...result});
    }},
    scStep('scm-create', ['create', serviceName, 'binPath=', `"${executable}"`, 'type=', 'own', 'start=', 'demand', 'obj=', 'LocalSystem', 'DisplayName=', serviceName], async () => { assert(absentBefore); created = true; const item = await observe(true); assert.equal(item.state, 'Stopped'); assert.equal(item.start_mode, 'Manual'); nativeEvidence.push({ phase: 'scm_created', ...item }); }),
    scStep('scm-configure', ['config', serviceName, 'error=', 'normal', 'start=', 'demand'], async () => { const item = await observe(true); assert.equal(item.error_control, 'Normal'); assert.equal(item.start_mode, 'Manual'); nativeEvidence.push({ phase: 'scm_configured', ...item }); }),
    scStep('scm-start', ['start', serviceName], async () => { const item = await waitState('Running'); pid = item.process_id; assert(Number.isSafeInteger(pid) && pid > 1); resources.pid = pid; nativeEvidence.push({ phase: 'scm_running', ...item,ordinary_host_process_access_not_assumed:true }); }),
    {id:'scm-process-identity',name:'start_process',args:()=>({command:powershell,args:psArguments(windowsProcessObservationScript(pid,{includeImagePath:true})),execution_identity:'administrator',timeout_ms:20000}),verify:value=>{exited(value);const result=json(value);assert.equal(result.pid,pid);assert.equal(result.state,'running');assert.equal(result.image_path.toLowerCase(),executable.toLowerCase());assert.match(result.creation_filetime,/^[1-9][0-9]+$/);processCreation=result.creation_filetime;resources.process_creation_filetime=processCreation;nativeEvidence.push({phase:'scm_admin_readonly_exact_process',...result});}},
    { id: 'scm-system-marker', name: 'read_file', args: { path: startMarker, execution_identity: 'administrator' }, verify: value => { const marker = JSON.parse(value.content); assert.equal(marker.fixture, nonce); assert.equal(marker.service_name, serviceName); assert.equal(marker.phase, 'started'); assert.equal(marker.sid, 'S-1-5-18'); assert.equal(marker.session_id, 0); assert.equal(marker.pid, pid); nativeEvidence.push({ phase: 'scm_actual_system_identity', ...marker }); } },
    scStep('scm-query', ['queryex', serviceName], async () => { const item = await observe(true); assert.equal(item.state, 'Running'); assert.equal(item.process_id, pid); nativeEvidence.push({ phase: 'scm_independent_lifecycle', ...item, create_start_commands_already_reaped: true }); }),
    scStep('scm-stop', ['stop', serviceName], async () => { const item = await waitState('Stopped'); assert.equal(item.process_id, 0);nativeEvidence.push({phase:'scm_stopped',...item,pid}); }),
    {id:'scm-process-exited',name:'start_process',args:()=>({command:powershell,args:psArguments(`for($i=0;$i-lt50;$i++){$v=(&{${windowsProcessObservationScript(pid)}}|ConvertFrom-Json);if($v.state-ne'running'-or$v.creation_filetime-cne${psLiteral(processCreation)}){break};Start-Sleep -Milliseconds 100};$v|ConvertTo-Json -Compress`),execution_identity:'administrator',timeout_ms:20000}),verify:value=>{exited(value);const result=json(value);assert.equal(result.pid,pid);assert(result.state==='absent'||result.state==='exited'&&result.creation_filetime===processCreation||result.creation_filetime!==processCreation&&/^[1-9][0-9]+$/.test(result.creation_filetime),'The exact creation identity must be proven gone');nativeEvidence.push({phase:'scm_stopped_exact_pid_gone',original_creation_filetime:processCreation,...result});}},
    { id: 'scm-stop-marker', name: 'read_file', args: { path: stopMarker, execution_identity: 'administrator' }, verify: value => { const marker = JSON.parse(value.content); assert.equal(marker.fixture, nonce); assert.equal(marker.phase, 'stopped'); assert.equal(marker.sid, 'S-1-5-18'); assert.equal(marker.pid, pid); nativeEvidence.push({ phase: 'scm_real_stop_callback', ...marker }); } },
    scStep('scm-delete', ['delete', serviceName], async () => { for (let i = 0; i < 100; i++) { if (!(await windowsObservation(queryScript)).length) break; await pause(100); } await observe(false); }),
    cleanup,
    {id:'scm-final-build-absence',name:'read_file',args:{path:compiledExecutable,missing_ok:true},verify:value=>{assert.equal(value.kind,'workspace_file_absent');assert([compiledExecutable,`\\\\?\\${compiledExecutable}`].includes(value.path));nativeEvidence.push({phase:'canonical_final_build_source_absent',path:value.path,execution_identity:'current_user'});}},
    {id:'scm-final-system-absence',name:'read_file',args:{path:startMarker,missing_ok:true,execution_identity:'administrator'},verify:value=>{assert.equal(value.kind,'workspace_file_absent');assert([startMarker,`\\\\?\\${startMarker}`].includes(value.path));nativeEvidence.push({phase:'canonical_final_system_source_absent',path:value.path,execution_identity:'administrator'});}},
  ];
  return {
    title: 'Windows real SCM LocalSystem service acceptance', steps, resources, nativeEvidence,
    prompt: `只操作本次唯一SCM服务 ${serviceName}、普通编译目录 ${buildRoot} 与管理员服务目录 ${root}。经canonical文件工具写测试ServiceBase源码、普通进程仅在build目录编译；管理员从创建起直接赋予独立service目录BA/SY保护DACL并复制已验hash的exe、核对owner/文件ACL/hash，普通用户从不预读该protected目录规则scope。以现役administrator start_process的sc.exe create/config/start/query/stop/delete真实服务，正式管理员read_file读取ServiceBase真实OnStart/OnStop标记，核对LocalSystem SID/Session0/确切PID；独立SCM查询证明启动命令已退出时服务仍运行，停止并确认PID退出，注销并删除两个唯一目录中的已知普通文件。不得操作既有服务或其他路径。`,
    completionSummary: role => role === 'main'
      ? `本次唯一SCM服务 ${serviceName} 的真实create/config/start/query/stop/delete已完成；管理员只读标记与SCM的PID ${pid} 一致，真实LocalSystem SID S-1-5-18/Session0及OnStop回调已核验；确切PID已退出、注册与文件清理=${removed}。不据此声称第三方installer、网络或安全配置全面通过。`
      : `原SCM业务验收失败保持失败。恢复Turn只按确切服务名 ${serviceName}、本次exe路径及LocalSystem身份核对并清理唯一注册与文件，清理证明=${removed}；不声明原create/config/start/query/stop/delete业务全部成功。`,
    cleanupNeeded: async () => !(removed&&await missing(root)&&await missing(buildRoot))&&(!await missing(root)||!await missing(buildRoot)||created),
    cleanupSteps: [cleanup],
    cleanupPrompt: `原SCM验收失败保持失败，原Turn已取消且受管进程和canonical效果结算已证明。只停止、注销本次唯一 ${serviceName}（必须匹配本次exe路径/LocalSystem），确认确切PID退出后删除 ${root} 中已列出的三个普通文件和 ${buildRoot} 的两个编译文件；两个目录均严格核对原owner/无reparse/仅已知regular文件，任何身份或内容变化立即保留现场。`,
    cleanupIntent: { service_name: serviceName, executable, protected_root: root,build_root:buildRoot,only_after_canonical_settlement: true },
    pendingAcceptance: [{ id: 'third-party-installer-and-system-settings', status: 'not_run', reason: 'This exact isolated SCM LocalSystem fixture does not prove arbitrary third-party installers or global security/network setting changes.' }],
  };
}
