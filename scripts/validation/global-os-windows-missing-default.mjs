// Opt-in native fixture: remove only the frozen Session's known empty default
// directory, then prove its absolute-path tools retain the same identity.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {lstat,readdir,realpath,rmdir} from 'node:fs/promises';
import {dirname,join} from 'node:path';
import {assertWindowsFixture,windowsObservation,psLiteral} from './global-os-windows-cases.mjs';
const stable=value=>JSON.stringify(value,(_,entry)=>entry&&typeof entry==='object'&&!Array.isArray(entry)?Object.fromEntries(Object.keys(entry).sort().map(key=>[key,entry[key]])):entry);
const digest=value=>createHash('sha256').update(stable(value)).digest('hex');
export function nativeMissingDefaultOptions({enabled=false,platform,scenario,administrator=false,protectedUserDirectory=false,retainServiceForAppExit=false,continueSession,installerReport,nativeDataRoot}){
  assert.equal(typeof enabled,'boolean','--missing-default is a boolean fixture option');if(!enabled)return;
  assert.equal(platform,'win32','--missing-default requires a native Windows App');assert.equal(scenario,'files-process','--missing-default only applies to ordinary files-process');
  assert(!administrator&&!protectedUserDirectory&&!retainServiceForAppExit&&!continueSession&&installerReport===undefined,'The missing-default fixture cannot combine independent scenarios or continuation');
  assert(typeof nativeDataRoot==='string'&&nativeDataRoot.length>0,'--missing-default requires the same private --native-data-root');
}
export function missingDefaultFrozenIdentity({session,projection,events,workspace,nativeDataset,announcement,caller}){
  const fact=session.session;assert(fact&&typeof fact.agent_session_id==='string');assert.equal(session.head.session_id,fact.agent_session_id);assert.equal(session.head.status,'ready');assert(session.head.active_turn_id==null);
  assert.equal(projection.conversation_id,fact.agent_session_id);assert.equal(projection.extra.workspace,workspace);assert.deepEqual(projection.agent_snapshot.canonical_binding,fact.agent_binding);
  assert(Array.isArray(fact.agent_binding.typed_resource_bindings)&&fact.agent_binding.typed_resource_bindings.length>0);assert(fact.agent_binding.resolved_snapshot_ref);assert(projection.agent_snapshot.enabled_capability_actions);
  assert(!events.some(event=>event.kind==='turn/started'||event.kind==='runtime/execution-claimed'),'Default directory removal must precede the first Turn');
  assert(events.length>0&&events.every(event=>event.agent_session_id===fact.agent_session_id));assert.equal(events.at(-1).seq,session.head.last_seq);
  assert.equal(caller.platform,'windows');assert.equal(caller.process_id,announcement.pid);assert.equal(nativeDataset.portPath,join(nativeDataset.root,'port.json'));
  return{session_id:fact.agent_session_id,owner_ref:structuredClone(fact.owner_ref),binding:structuredClone(fact.agent_binding),snapshot:structuredClone(projection.agent_snapshot),workspace,native_data_root:nativeDataset.root,native_data_parent:nativeDataset.parent,app_pid:announcement.pid,initial_events:structuredClone(events),prefix_last_seq:session.head.last_seq,
    binding_sha256:digest(fact.agent_binding),resource_bindings_sha256:digest(fact.agent_binding.typed_resource_bindings),snapshot_sha256:digest(projection.agent_snapshot),canonical_prefix_sha256:digest(events)};
}
export function verifyMissingDefaultFrozenIdentity({frozen,session,projection,events,nativeDataset,announcement}){
  assert.equal(session.session.agent_session_id,frozen.session_id);assert.deepEqual(session.session.owner_ref,frozen.owner_ref);assert.deepEqual(session.session.agent_binding,frozen.binding,'Default absence must not replace the frozen binding or resources');
  assert.equal(projection.conversation_id,frozen.session_id);assert.equal(projection.extra.workspace,frozen.workspace);assert.deepEqual(projection.agent_snapshot,frozen.snapshot,'Default absence must not replace the frozen resolved snapshot or grants');
  assert.equal(nativeDataset.root,frozen.native_data_root);assert.equal(nativeDataset.parent,frozen.native_data_parent);assert.equal(announcement.pid,frozen.app_pid);
  assert.equal(digest(events.filter(event=>event.seq<=frozen.prefix_last_seq)),frozen.canonical_prefix_sha256,'Removing the default directory must preserve all original facts');
  return{session_id_unchanged:true,owner_unchanged:true,binding_unchanged:true,resource_ids_unchanged:true,resolved_snapshot_unchanged:true,grants_unchanged:true,native_data_root_unchanged:true,app_pid_unchanged:true,new_dataset_created:false,original_canonical_prefix_unchanged:true};
}
export async function beginWindowsMissingDefaultCase({fixture,workspace,loginIdentity,session,projection,events,nativeDataset,announcement,caller,evidence}){
  assert.equal(process.platform,'win32');assert.equal(loginIdentity.administrator,false);await assertWindowsFixture(fixture,workspace);
  const frozen=missingDefaultFrozenIdentity({session,projection,events,workspace,nativeDataset,announcement,caller});
  Object.assign(evidence,{schema:'nomifun.windows-missing-default.v1',status:'preparing',workspace,binding_sha256:frozen.binding_sha256,resource_bindings_sha256:frozen.resource_bindings_sha256,snapshot_sha256:frozen.snapshot_sha256,canonical_prefix_sha256:frozen.canonical_prefix_sha256,session_id:frozen.session_id,native_data_root:frozen.native_data_root,original_prefix_last_seq:frozen.prefix_last_seq,host_removal_kind:'rmdir_known_empty_directory_only',host_recursive_delete:false,host_workspace_recreated:false});
  for(let parent=workspace;;parent=dirname(parent)){const info=await lstat(parent);assert(info.isDirectory()&&!info.isSymbolicLink(),'The fixture and all its parents must be actual directories');if(dirname(parent)===parent)break;}
  assert.equal((await realpath(workspace)).toLowerCase(),workspace.toLowerCase());assert.deepEqual(await readdir(workspace),[],'Only the known empty default directory may be removed');
  const owner=await windowsObservation(`[IO.Directory]::GetAccessControl(${psLiteral(workspace)}).GetOwner([Security.Principal.SecurityIdentifier]).Value|ConvertTo-Json -Compress`);assert.equal(owner,loginIdentity.sid);
  const before=await lstat(workspace);evidence.original_directory={dev:before.dev,ino:before.ino,owner_sid:owner};
  try{await rmdir(workspace);}catch(error){Object.assign(evidence,{status:'preparation_failed',os_code:error.code,removed:false,requires_same_dataset_normal_exit_before_empty_remove_and_restart:error.code==='EBUSY',app_closed_by_driver:false,forced_delete:false});throw error;}
  await assert.rejects(lstat(workspace),error=>error.code==='ENOENT');Object.assign(evidence,{status:'default_removed_before_first_turn',removed:true,default_directory_absent:true,removed_at:new Date().toISOString()});return frozen;
}
export async function verifyWindowsMissingDefaultCase({frozen,session,projection,events,nativeDataset,announcement,evidence,scenarioCompleted=true}){
  const identity=verifyMissingDefaultFrozenIdentity({frozen,session,projection,events,nativeDataset,announcement});await assert.rejects(lstat(frozen.workspace),error=>error.code==='ENOENT','Absolute-path operations must not silently recreate the default directory');
  Object.assign(evidence,{...identity,status:'identity_and_default_absence_verified',scenario_completed:scenarioCompleted,default_directory_absent:true,host_workspace_recreated:false,verified_at:new Date().toISOString()});return identity;
}
