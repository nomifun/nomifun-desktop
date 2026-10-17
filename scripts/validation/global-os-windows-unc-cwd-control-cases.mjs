// Diagnostic only. This compares two child-process cwd spellings without
// changing PowerShell's own location or declaring the product UNC case passed.
import assert from 'node:assert/strict';
import{createHash}from'node:crypto';
import{readFile}from'node:fs/promises';
import{join}from'node:path';
import{powershell,windowsProcessObservation}from'./global-os-windows-cases.mjs';
import{createWindowsUncScenario}from'./global-os-windows-unc-cases.mjs';
import{windowsUncCwdProbeSource}from'./global-os-windows-unc-cwd-probe.mjs';

export function validateWindowsUncCwdControlLifecycle(value,{plainRoot,extendedRoot}){
  assert.equal(value.state,'exited');assert([0,1].includes(value.exit_code));assert.equal(value.cleanup?.reaped,true);assert.deepEqual(value.cleanup.errors??[],[]);
  assert.equal(value.output?.dropped_bytes,0);assert.equal(value.output?.decode_errors,0);assert.equal(typeof value.output.text,'string');const body=value.output.text.replace(/^\uFEFF/,'').trim();assert(Buffer.byteLength(body)<=3072,'UNC cwd control output exceeds its protocol budget');
  const protocol=JSON.parse(body);assert.equal(protocol.schema,'nomifun.windows-unc-cwd-spelling-control.v1');assert.equal(protocol.filesystem_mutations,0);assert.equal(protocol.results.length,2);
  const pids=new Set();
  for(const[index,row]of protocol.results.entries()){
    assert.equal(row.spelling,['plain','verbatim'][index]);assert.equal(row.requested_cwd,[plainRoot,extendedRoot][index]);assert(Number.isSafeInteger(row.pid)&&row.pid>1&&!pids.has(row.pid));pids.add(row.pid);
    assert.match(row.creation_filetime,/^[1-9][0-9]+$/);assert(Number.isSafeInteger(row.exit_code)&&row.exit_code>=0&&row.exit_code<=4294967295);assert.equal(row.wait_completed,true);assert.equal(row.handle_closed,true);
    assert(Number.isSafeInteger(row.stdout_bytes)&&row.stdout_bytes>=0&&row.stdout_bytes<=16384);assert(Number.isSafeInteger(row.stderr_bytes)&&row.stderr_bytes>=0&&row.stderr_bytes<=8192);
    assert(Number.isSafeInteger(row.command_line_utf16_units)&&row.command_line_utf16_units>0&&row.command_line_utf16_units<=32767);
    assert.equal(typeof row.stderr_text,'string');assert(row.stderr_text.length<=256);assert.match(row.stderr_sha256,/^[a-f0-9]{64}$/);assert.equal(typeof row.stderr_excerpt_truncated,'boolean');
    if(!row.stderr_excerpt_truncated){assert.equal(Buffer.byteLength(row.stderr_text),row.stderr_bytes);assert.equal(createHash('sha256').update(row.stderr_text).digest('hex'),row.stderr_sha256);}
    if(row.observation===null){assert.equal(typeof row.stdout_text,'string');assert(row.stdout_text.length<=256);assert.match(row.stdout_sha256,/^[a-f0-9]{64}$/);assert.equal(typeof row.stdout_excerpt_truncated,'boolean');if(!row.stdout_excerpt_truncated){assert.equal(Buffer.byteLength(row.stdout_text),row.stdout_bytes);assert.equal(createHash('sha256').update(row.stdout_text).digest('hex'),row.stdout_sha256);}}
    assert(row.protocol_error===null||typeof row.protocol_error==='string');
    assert(row.observation===null||(typeof row.observation==='object'&&!Array.isArray(row.observation)));
  }
  const succeeded=protocol.results.every(row=>row.exit_code===0&&row.protocol_error===null&&row.observation!==null);
  assert.equal(protocol.protocol_succeeded,succeeded);assert.equal(value.exit_code,succeeded?0:1,'The parent exit must preserve the actual child protocol status');
  return protocol;
}
export function validateWindowsUncCwdControlReceipt(value,expected){
  const protocol=validateWindowsUncCwdControlLifecycle(value,expected);assert.equal(protocol.protocol_succeeded,true);
  for(const row of protocol.results){
    const observation=row.observation;
    for(const key of ['native_cwd','environment_cwd','provider_path'])assert(typeof observation[key]==='string'&&observation[key].length>0&&observation[key].length<=4096&&!observation[key].includes('\0'));
    assert.equal(typeof observation.dotnet_marker.read,'boolean');
    if(observation.dotnet_marker.read)assert.equal(observation.dotnet_marker.content,expected.markerContent,'A relative read must never adopt a foreign marker');
    else{assert.equal(observation.dotnet_marker.content,null);assert.equal(typeof observation.dotnet_marker.error_code,'string');}
    assert.equal(typeof observation.ps_item.exists,'boolean');
    if(observation.ps_item.exists){assert.equal(typeof observation.ps_item.full_name,'string');assert(Number.isSafeInteger(observation.ps_item.length)&&observation.ps_item.length>=0);}
    else assert.equal(typeof observation.ps_item.error_code,'string');
  }
  return protocol;
}
export async function createWindowsUncCwdControlScenario(options){
  const base=await createWindowsUncScenario(options),resources=base.resources,priorCompletion=base.completionSummary,priorCleanupNeeded=base.cleanupNeeded;
  const cleanupGuard={phase:'diagnostic_child_cleanup_guard',probe_proposed:false,complete_child_protocol_and_exact_exit_verified:false,automatic_fixture_cleanup_allowed:true};base.nativeEvidence.push(cleanupGuard);
  const nonce=resources.share_name.slice('NomiFun_GlobalOS_'.length),plainRoot=`${resources.unc_root}\\`,extendedRoot=`${resources.extended_unc_root}\\`;
  const markerName=`identity-${nonce}.txt`,markerContent=`UNC_FIXTURE_${nonce}\n`,probePath=join(resources.physical_root,'unc-cwd-probe.ps1');
  const source=windowsUncCwdProbeSource({plainRoot,extendedRoot,markerName,markerContent,powershell});
  Object.assign(resources,{diagnostic_only:true,probe_path:probePath,probe_source_sha256:createHash('sha256').update(source).digest('hex'),marker_name:markerName});
  const write={id:'unc-cwd-control-write-probe',name:'write_file',args:{path:probePath,content:source},verify:async()=>assert.equal(await readFile(probePath,'utf8'),source)};
  const observeLifecycle=async(value,provenance)=>{
    const protocol=validateWindowsUncCwdControlLifecycle(value,{plainRoot,extendedRoot}),children=[];
    for(const row of protocol.results){
      const observed=windowsProcessObservation(row.pid);assert(!observed.native_error);assert(['absent','exited','running'].includes(observed.state));
      if(observed.state==='running')assert.notEqual(observed.creation_filetime,row.creation_filetime,'The exact diagnostic child is still alive');
      if(observed.state==='exited'&&observed.creation_filetime===row.creation_filetime)assert.equal(observed.exit_code,String(row.exit_code));
      children.push({pid:row.pid,original_creation_filetime:row.creation_filetime,exact_child_gone:true,native_process:observed});
    }
    base.nativeEvidence.push({phase:'canonical_unc_cwd_spelling_control',protocol,child_exit_observations:children,parent_exit_code:value.exit_code,parent_supervisor_reaped:true,product_unc_semantics_certified:false,...(provenance?{canonical_failure_provenance:provenance}:{})});
    cleanupGuard.complete_child_protocol_and_exact_exit_verified=true;cleanupGuard.automatic_fixture_cleanup_allowed=true;
  };
  const probe={id:'unc-cwd-two-spellings',name:'exec_command',args:()=>{cleanupGuard.probe_proposed=true;cleanupGuard.automatic_fixture_cleanup_allowed=false;return{command:powershell,args:['-NoLogo','-NoProfile','-NonInteractive','-File',probePath],cwd:resources.physical_root,timeout_ms:30000};},
    observeTerminalFailure:async({receipt,provenance})=>observeLifecycle(receipt,provenance),
    verify:async value=>{await observeLifecycle(value);validateWindowsUncCwdControlReceipt(value,{plainRoot,extendedRoot,markerContent});},
  };
  // Object.assign retains the original dynamic cleanupSteps getter and the
  // exact share admission/removal state captured by the original factory.
  Object.assign(base,{steps:[...base.steps.slice(0,3),write,probe,...base.steps.slice(-2)],
    cleanupNeeded:async()=>cleanupGuard.automatic_fixture_cleanup_allowed?priorCleanupNeeded():false,
    prompt:`仅诊断本次唯一共享 ${resources.share_name} 的两种UNC工作目录拼写。沿原canonical owner建立marker及严格SID share，写入并运行固定探针；外层ordinary ProcessSupervisor管理两个自然退出的WinPS子进程，逐项记录原生cwd、.NET cwd、PowerShell ProviderPath和唯一marker的两种相对读取。禁止Set-Location、profile、盘符映射或修改已有服务/认证。完整JSON及两个确切子进程退出证明取得后，只移除本次Name/Path/Description匹配共享和物理目录。不把差异当作产品UNC16项通过，不处理任何旧失败共享。`,
    completionSummary:role=>role==='main'?`本次独立UNC拼写控制已取得两子进程完整有界JSON、retained创建身份、自然exit0、确切PID退出及parent supervisor清理；原生/.NET/PowerShell路径差异如实保存，仅本次share和目录已移除。此诊断不代表产品UNC16项通过，也不核实或结算旧未知效果。`:priorCompletion(role),
    pendingAcceptance:[{id:'unc-cwd-product-contract',status:'diagnostic_only',reason:'The two-spelling child-process control records behavior; it cannot substitute for the full canonical product UNC regression.'}],
  });
  return base;
}
