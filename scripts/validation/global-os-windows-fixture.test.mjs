import{test,expect}from'bun:test';
import{createHash}from'node:crypto';
import{resolve}from'node:path';
import{assertWindowsDesktopPreservationSource,assertWindowsUpgradePreservation,windowsRendererReadinessTransient}from'./global-os-windows-fixture.mjs';
function preservedUpgrade(){
  const effect={event_id:'uncertain',seq:3,kind:'effect/uncertain',payload:{value:{outcome:'unknown'}}};
  const preserved={schema:'nomifun.windows-desktop-unknown-observation.v1',app_pid:100,session_id:'original',execution:{operation_id:'operation',state:'cancelled',unknown_effects:1,pending_effects:0},effect_events:[effect]};
  const preservedBytes=Buffer.from(JSON.stringify(preserved)),preservedSessionReport=resolve('original-desktop-guard.json');
  const canonical_preservation=assertWindowsUpgradePreservation({before:preserved,execution:preserved.execution,events:preserved.effect_events});
  const source={status:'ready',app_pid:200,prior_app_pid:100,owner_jwt_reused:true,exe_sha256:'a'.repeat(64),upgrade:{new_exe_sha256:'a'.repeat(64),preserved_session_report:preservedSessionReport,preserved_session_report_sha256:createHash('sha256').update(preservedBytes).digest('hex'),native_data_root_unchanged:true,owner_jwt_unchanged:true,new_dataset_created:false,canonical_preservation}};
  return{source,preserved,preservedBytes,preservedSessionReport};
}
test('a second native App upgrade can retain the exact original Desktop guard through its verified previous receipt',()=>{
  expect(assertWindowsDesktopPreservationSource(preservedUpgrade())).toEqual({kind:'verified_prior_upgrade',app_pid:200,original_guard_app_pid:100,source_exe_sha256:'a'.repeat(64)});
  const direct=preservedUpgrade();direct.source.app_pid=100;delete direct.source.upgrade;
  expect(assertWindowsDesktopPreservationSource(direct).kind).toBe('direct_original_app');
});
test('an inherited Desktop guard rejects an unproved App hop, changed bytes, dataset, executable or canonical facts',()=>{
  for(const change of [x=>{x.source.status='failed';},x=>{delete x.source.upgrade;},x=>{x.source.prior_app_pid=99;},x=>{x.source.owner_jwt_reused=false;},x=>{x.source.exe_sha256='b'.repeat(64);},x=>{x.source.upgrade.preserved_session_report=resolve('different.json');},x=>{x.source.upgrade.preserved_session_report_sha256='b'.repeat(64);},x=>{x.source.upgrade.new_dataset_created=true;},x=>{x.source.upgrade.native_data_root_unchanged=false;},x=>{x.source.upgrade.owner_jwt_unchanged=false;},x=>{delete x.source.upgrade.canonical_preservation;},x=>{x.source.upgrade.canonical_preservation.unknown_effects=0;},x=>{x.source.upgrade.canonical_preservation.driver_original_calls_dispatched=1;}]){
    const input=preservedUpgrade();change(input);expect(()=>assertWindowsDesktopPreservationSource(input)).toThrow();
  }
});
test('renderer readiness waits only for exact Node or observed Bun transport transients',()=>{
  for(const code of ['ECONNREFUSED','ECONNRESET','UND_ERR_SOCKET'])expect(windowsRendererReadinessTransient({name:'TypeError',cause:{code}})).toBe(true);
  expect(windowsRendererReadinessTransient({name:'TypeError',code:'ConnectionRefused'})).toBe(true);
  for(const name of ['TimeoutError','AbortError'])expect(windowsRendererReadinessTransient({name})).toBe(true);
  for(const error of [new TypeError('renderer broke'),new SyntaxError('invalid CDP JSON'),new Error('The original owner JWT was rejected after native restart'),{name:'TypeError',code:'EPERM'},{name:'TypeError',cause:{code:'EACCES'}},{name:'Error',code:'ConnectionRefused'},undefined])expect(windowsRendererReadinessTransient(error)).toBe(false);
});
