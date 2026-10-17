// Independent SMB control protocol, dispatched only through canonical tools.
// It neither reads nor settles the original pending physical operation.
import assert from 'node:assert/strict';
import {randomUUID,createHash} from 'node:crypto';
import {lstat,readFile,realpath} from 'node:fs/promises';
import {isAbsolute,join,win32} from 'node:path';
import {assertWindowsFixture,assertWindowsPrivatePath,powershell,psArguments,psLiteral} from './global-os-windows-cases.mjs';
import {windowsSmbDispositionControlTarget,windowsSmbDispositionProbeScript} from './global-os-windows-smb-disposition-probe.mjs';

const preservedSha='be97ccdfcde70b3ea454cd2fb29a682a0698dc089205f626946649a1a027ea11';
const preservedSession='01a14893-93f7-7a90-b573-bc0b31116207';
const preservedNonce='34d53b78-f4d3-4128-b8a6-cb7e6b509895';
const digest=bytes=>createHash('sha256').update(bytes).digest('hex');
const same=(left,right)=>left.replace(/^\\\\\?\\/,'').toLowerCase()===right.replace(/^\\\\\?\\/,'').toLowerCase();
const absent=path=>lstat(path).then(()=>false,error=>{if(error.code==='ENOENT')return true;throw error;});
export function preservedUncControlSource(report,sha256,loginSid){
  assert.equal(sha256,preservedSha,'Use only the exact preserved failed UNC report');
  assert.equal(report.schema,'nomifun.global-os-native-regression.v2');assert.equal(report.platform,'win32');
  assert.equal(report.scenario,'unc');assert.equal(report.status,'failed');assert.equal(report.administrator,false);
  assert.equal(report.session_id,preservedSession);assert.equal(report.failed_phase.accepted.agent_session_id,preservedSession);
  assert.equal(report.failed_phase.accepted.replayed,false);assert.equal(report.login_identity.sid,loginSid);
  const resources=report.native_case_resources;
  assert.equal(resources.share_name,`NomiFun_GlobalOS_${preservedNonce}`);
  assert.equal(resources.root,win32.join(report.fixture,`unc-cases-${preservedNonce}`));
  assert.equal(resources.physical_root,win32.join(resources.root,'share-root'));
  assert.equal(resources.unc_root,`\\\\localhost\\${resources.share_name}`);
  assert.equal(report.workspace,win32.join(report.fixture,'workspace'));
  assert.equal(report.cleanup_intent.only_after_canonical_settlement,true);
  assert(report.failure.message.includes('EXECUTION_CLEANUP_UNPROVEN'));
  for(const step of ['unc-create-share','unc-write','unc-read'])assert(report.checks.some(row=>row.step===step&&row.status==='passed'));
  return{report_sha256:sha256,session_id:report.session_id,physical_root:resources.physical_root,share_name:resources.share_name};
}
export function validateSmbDispositionControlReceipt(value,target){
  assert.equal(value.state,'exited','Do not replay an unknown control process');assert.equal(value.exit_code,0);
  assert.equal(value.cleanup?.reaped,true);assert.deepEqual(value.cleanup?.errors??[],[]);
  const receipt=JSON.parse(value.output.text.trim());assert.equal(receipt.schema,'nomifun.windows-smb-disposition-control.v1');
  assert.equal(receipt.nonce,target.nonce);assert.equal(receipt.automatic_fallbacks,0);assert.equal(receipt.manual_file_cleanup,0);
  assert.equal(receipt.original_effect_settlement,false);assert.equal(receipt.cases.length,16);
  const names=new Set();
  for(const row of receipt.cases){
    assert(['local','unc'].includes(row.transport));assert.equal(typeof row.metadata_holder,'boolean');
    assert(row.information_class===4?row.flags===null:row.information_class===21&&[0x13,3,1].includes(row.flags));
    const expected=`control-${target.nonce}-${row.transport}-${row.metadata_holder?'pinned':'unheld'}-${(row.flags??0).toString(16).padStart(2,'0')}.bin`;
    assert.equal(row.name,expected);assert(!names.has(expected));names.add(expected);
    assert.equal(row.open_access,0x110080);assert.equal(row.open_sharing,3);assert.equal(typeof row.marked,'boolean');
    assert.equal(row.all_own_handles_closed,true,'A close failure is not confirmed cleanup');assert(!row.exception,'Retain unexpected control failure');
    for(const step of row.steps){assert.equal(typeof step.success,'boolean');assert(Number.isInteger(step.raw_os_error)&&step.raw_os_error>=0);if(step.success)assert.equal(step.raw_os_error,0);}
    assert(row.steps.some(step=>step.operation==='CREATE_NEW'&&step.success));
    for(const state of [row.name_after_all_own_handles_closed,row.local_name_after_all_own_handles_closed,row.unc_name_after_all_own_handles_closed]){
      assert.equal(typeof state.absent,'boolean');assert(Number.isInteger(state.raw_os_error));assert.equal(state.absent,[2,3].includes(state.raw_os_error));
    }
  }
  return receipt;
}
export function windowsSmbControlPlan({probeInput,workspace,loginIdentity,verifyMarker,verifyShare,verifyProbe,verifyRemoved,verifyRoot,verifyAbsence}){
  const target=windowsSmbDispositionControlTarget(probeInput),root=win32.dirname(target.physicalRoot),marker=win32.join(target.physicalRoot,target.markerName);
  assert.equal(win32.dirname(root),win32.dirname(workspace));assert.equal(win32.basename(workspace),'workspace');
  assert.match(loginIdentity.sid,/^S-1-5-21-(?:[0-9]+-){3}[0-9]+$/);assert.equal(loginIdentity.administrator,false);
  const description=`NomiFun independent disposition controls ${target.nonce}`;
  const query=`$name=${psLiteral(target.shareName)};$share=$null;try{$share=Get-SmbShare -Name $name -ErrorAction Stop}catch{if($_.CategoryInfo.Category -ne 'ObjectNotFound'){throw}};`;
  const requireOwned=`if($null -ne $share){if($share.Name -cne ${psLiteral(target.shareName)} -or $share.Path.TrimEnd('\\') -ine ${psLiteral(target.physicalRoot)}.TrimEnd('\\') -or $share.Description -cne ${psLiteral(description)}){throw 'Independent control share changed identity; preserve it'}};`;
  const inspect=`$access=@(Get-SmbShareAccess -Name $name -ErrorAction Stop|ForEach-Object{$account=New-Object Security.Principal.NTAccount($_.AccountName);[pscustomobject]@{sid=$account.Translate([Security.Principal.SecurityIdentifier]).Value;kind=[string]$_.AccessControlType;right=[string]$_.AccessRight}});[pscustomobject]@{name=$share.Name;path=$share.Path;description=$share.Description;state=[string]$share.ShareState;access=$access}|ConvertTo-Json -Depth 5 -Compress`;
  const create=`${query}if($null -ne $share){throw 'Independent control share collision'};New-SmbShare -Name $name -Path ${psLiteral(target.physicalRoot)} -Description ${psLiteral(description)} -FullAccess ${psLiteral(loginIdentity.name)} -ErrorAction Stop|Out-Null;${query}${requireOwned}${inspect}`;
  const remove=`${query}${requireOwned}if($null -eq $share){throw 'Independent control share unexpectedly absent'};Remove-SmbShare -Name $name -Force -Confirm:$false -ErrorAction Stop;${query}if($null -ne $share){throw 'Independent control share remains'};[pscustomobject]@{name=$name;exists=$false;expected_path=${psLiteral(target.physicalRoot)}}|ConvertTo-Json -Compress`;
  const probe=windowsSmbDispositionProbeScript(probeInput);
  const exec=(id,script,verify,administrator=false)=>({id,name:'exec_command',args:{command:powershell,args:psArguments(script),cwd:workspace,timeout_ms:60000,...(administrator?{execution_identity:'administrator'}:{})},verify:async value=>{assert.equal(value.state,'exited');assert.equal(value.exit_code,0);assert.equal(value.cleanup?.reaped,true);assert.deepEqual(value.cleanup?.errors??[],[]);await verify?.(value);}});
  return{target,root,marker,description,probe_script_sha256:digest(probe),steps:[
    {id:'smb-control-marker',name:'write_file',args:{path:marker,content:target.markerText},verify:verifyMarker},
    exec('smb-control-create-share',create,verifyShare,true),
    exec('smb-control-disposition-probe',probe,verifyProbe),
    exec('smb-control-remove-share',remove,verifyRemoved,true),
    {id:'smb-control-delete-local-root',name:'delete_path',args:{path:root},verify:verifyRoot},
    {id:'smb-control-marker-absence',name:'read_file',args:{path:marker,missing_ok:true},verify:verifyAbsence},
  ]};
}
export async function createWindowsSmbControlScenario({fixture,workspace,loginIdentity,preservedUncReport}){
  await assertWindowsFixture(fixture,workspace);await assertWindowsPrivatePath(fixture);assert.equal(loginIdentity.administrator,false);
  assert(isAbsolute(preservedUncReport));const before=await lstat(preservedUncReport);assert(before.isFile()&&!before.isSymbolicLink()&&before.size<=2*1024*1024);
  const bytes=await readFile(preservedUncReport),after=await lstat(preservedUncReport);assert.equal(after.ino,before.ino);assert.equal(after.dev,before.dev);assert.equal(after.mtimeMs,before.mtimeMs);assert.equal(bytes.length,before.size);
  const source=preservedUncControlSource(JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(bytes)),digest(bytes),loginIdentity.sid);
  const nonce=randomUUID(),root=join(fixture,`smb-disposition-cases-${nonce}`),physicalRoot=join(root,'share-root'),shareName=`NomiFun_GlobalOS_Disposition_${nonce}`;
  const probeInput={nonce,physicalRoot,shareName,preservedPhysicalRoot:source.physical_root,preservedShareName:source.share_name};
  const nativeEvidence=[];let shareVerified=false,shareRemoved=false,probeReceipt;
  assert(await absent(root));
  const plan=windowsSmbControlPlan({probeInput,workspace,loginIdentity,
    verifyMarker:async()=>{assert.equal(await readFile(plan.marker,'utf8'),plan.target.markerText);assert(same(await realpath(physicalRoot),physicalRoot));},
    verifyShare:async value=>{const result=JSON.parse(value.output.text.trim());assert.equal(result.name,shareName);assert(same(result.path,physicalRoot));assert.equal(result.description,plan.description);assert.equal(result.state,'Online');assert.deepEqual(result.access,[{sid:loginIdentity.sid,kind:'Allow',right:'Full'}]);assert.equal(await readFile(`${plan.target.uncRoot}\\${plan.target.markerName}`,'utf8'),plan.target.markerText);shareVerified=true;nativeEvidence.push({phase:'independent_control_share_registered',...result});},
    verifyProbe:value=>{assert(shareVerified);probeReceipt=validateSmbDispositionControlReceipt(value,plan.target);nativeEvidence.push({phase:'independent_disposition_control_protocol',...probeReceipt});},
    verifyRemoved:value=>{assert(probeReceipt,'No teardown after an unverified/unknown control process');const result=JSON.parse(value.output.text.trim());assert.equal(result.name,shareName);assert.equal(result.exists,false);assert(same(result.expected_path,physicalRoot));shareRemoved=true;nativeEvidence.push({phase:'independent_control_share_absent',...result});},
    verifyRoot:async()=>{assert(shareRemoved);assert(await absent(root));},
    verifyAbsence:value=>{assert(shareRemoved&&probeReceipt);assert.equal(value.kind,'workspace_file_absent');assert(same(value.path,plan.marker));nativeEvidence.push({phase:'canonical_control_marker_source_absent',path:value.path,execution_identity:'current_user'});},
  });
  return{steps:plan.steps,resources:{root,physical_root:physicalRoot,share_name:shareName,unc_root:plan.target.uncRoot,marker:plan.marker,
    preserved_unc_report:preservedUncReport,preserved_unc_report_sha256:source.report_sha256,preserved_session_id:source.session_id,
    preserved_physical_root:source.physical_root,preserved_share_name:source.share_name,probe_script_sha256:plan.probe_script_sha256},nativeEvidence,
    prompt:`仅运行新namespace ${root}/${shareName} 的独立SMB disposition协议控制。普通canonical write_file创建本次marker，管理员canonical仅建立登录SID的本次share，普通exec精确运行16个新controlfile实验并保留rawOSerror/ID/handle关闭后的名称状态。marked不等于unlink，不自动fallback。然后只移除本次share并普通delete_path删除本次localroot，最后read_file missing_ok取得marker typedabsence。原${source.share_name}及其sample/backups/pending完全不动；不得结算、重放或从本实验追认旧效果。`,
    completionSummary:()=>`独立UUID SMB控制协议16组结果已保存，含rawOSerror、FileID、metadata holder与所有自有handle关闭后的名称状态；仅本控制share/localroot已canonical移除且正式marker Source absence已读。此证据不代表产品UNC patch通过，也不核实或结算原pending效果。`,
    cleanupNeeded:async()=>false,cleanupSteps:[],cleanupPrompt:'控制过程失败或未知时保留新独立namespace，不自动重放实验或清理；原UNC pending目标始终不操作。',
    cleanupIntent:{path:root,share_name:shareName,no_unknown_replay:true,no_host_mutations:true,preserved_session_id:source.session_id},
    allowsCanonicalFixturePath:path=>same(path,root)||same(win32.dirname(path),root)||same(win32.dirname(path),physicalRoot),
  };
}
