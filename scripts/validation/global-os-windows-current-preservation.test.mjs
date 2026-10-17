import{test,expect}from'bun:test';
import{resolve,win32}from'node:path';
import{validateWindowsCurrentPreservationIdentity}from'./global-os-windows-current-preservation.mjs';
function fixture(){
  const parent='C:\\fixture\\nomifun-global-os-owner-11111111-1111-4111-8111-111111111111',exe='C:\\fixture\\current-app.exe',creation_filetime='134366996950137334',sha='a'.repeat(64),tokenSha256='b'.repeat(64);
  const originalReady={schema:'nomifun.windows-global-os-fixture.v1',status:'ready',parent,native_data_root:win32.join(parent,'native-data'),token_file:win32.join(parent,'owner-jwt'),app_pid:30552};
  const ready={...originalReady,app_pid:22600,prior_app_pid:30552,owner_jwt_reused:true,exe,exe_sha256:sha,base_url:'http://127.0.0.1:10445',announcement:{host:'127.0.0.1',port:10445,pid:22600},actual_app_identity:{native_process:{creation_filetime}},upgrade:{native_data_root_unchanged:true,owner_jwt_unchanged:true,new_dataset_created:false,new_exe_sha256:sha}};
  const nativeBefore={pid:22600,state:'running',creation_filetime,image_path:exe},sourceReferences=Object.fromEntries(['current_ready','original_ready','desktop_guard','unc_guard'].map(name=>[name,{path:resolve(`${name}.json`),sha256:'c'.repeat(64)}]));
  return{ready,originalReady,caller:{platform:'windows',process_id:22600},tokenSha256,expectedTokenSha256:tokenSha256,ownerId:'owner',expectedOwnerId:'owner',nativeBefore,nativeAfter:structuredClone(nativeBefore),executableSha256:sha,announcement:structuredClone(ready.announcement),sourceReferences,observedAt:'2026-10-17T09:00:00.000Z'};
}
test('fresh preservation observation proves the currently running immutable App and same private owner without creating an attestation',()=>{
  const input=fixture(),original=structuredClone(input),proof=validateWindowsCurrentPreservationIdentity(input);
  expect(proof.app_pid).toBe(22600);expect(proof.new_owner_attestation_created).toBe(false);expect(proof.new_dataset_created).toBe(false);expect(proof.agent_calls_dispatched).toBe(0);expect(proof.reconciliation_requests).toBe(0);expect(input).toEqual(original);
});
test('a current snapshot cannot borrow a stale process, substituted image, caller, owner, token, data root or unverified source',()=>{
  for(const mutate of [x=>{x.caller.process_id=30552;},x=>{x.caller.platform='macos';},x=>{x.ready.status='failed';},x=>{x.ready.prior_app_pid=99;},x=>{x.ready.upgrade.new_dataset_created=true;},x=>{x.ready.owner_jwt_reused=false;},x=>{x.ready.native_data_root+='-new';},x=>{x.tokenSha256='d'.repeat(64);},x=>{x.ownerId='foreign';},x=>{x.executableSha256='d'.repeat(64);},x=>{x.nativeBefore.state='exited';},x=>{x.nativeAfter.creation_filetime+='0';},x=>{x.nativeAfter.image_path='C:\\foreign.exe';},x=>{x.announcement.pid=30552;},x=>{x.sourceReferences.desktop_guard.path='relative.json';},x=>{x.sourceReferences.unc_guard.sha256='invalid';},x=>{x.observedAt='2020-01-01T00:00:00Z';}]){const input=fixture();mutate(input);expect(()=>validateWindowsCurrentPreservationIdentity(input)).toThrow();}
});
