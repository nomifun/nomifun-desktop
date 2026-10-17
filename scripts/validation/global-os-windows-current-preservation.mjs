// Fresh, read-only observations for the currently running acceptance App.
// Prior guards stay immutable; these files neither attest a new user outcome
// nor dispatch, settle, replay or hydrate either original Session.
import assert from'node:assert/strict';
import{createHash,randomUUID}from'node:crypto';
import{createReadStream}from'node:fs';
import{lstat,readFile,realpath,writeFile}from'node:fs/promises';
import{dirname,isAbsolute,join,resolve,win32}from'node:path';
import{tmpdir}from'node:os';
import{fileURLToPath}from'node:url';
import{assertWindowsPrivatePath,readWindowsOwnedFixtureBytes,windowsProcessObservation}from'./global-os-windows-cases.mjs';
import{assertWindowsDesktopPreservationSource,assertWindowsUpgradePreservation}from'./global-os-windows-fixture.mjs';
import{loadWindowsAdditionalSessionReport,readWindowsAdditionalSessionFacts,assertWindowsAdditionalSessionPreservation}from'./global-os-windows-unsettled-preservation.mjs';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const samePath=(left,right)=>typeof left==='string'&&typeof right==='string'&&left.toLowerCase()===right.toLowerCase();
async function stableReport(path){
  assert(isAbsolute(path));const before=await lstat(path);assert(before.isFile()&&!before.isSymbolicLink()&&before.size<=8*1024*1024);
  const bytes=await readFile(path),after=await lstat(path);for(const key of['ino','dev','size','mtimeMs'])assert.equal(after[key],before[key]);assert.equal(bytes.length,before.size);
  return{path:resolve(path),sha256:hash(bytes),bytes,value:JSON.parse(bytes)};
}
async function stableImageDigest(path){
  const before=await lstat(path);assert(before.isFile()&&!before.isSymbolicLink()&&before.size<=512*1024*1024);const digest=createHash('sha256');for await(const bytes of createReadStream(path))digest.update(bytes);
  const after=await lstat(path);for(const key of['ino','dev','size','mtimeMs'])assert.equal(after[key],before[key]);return digest.digest('hex');
}
export function validateWindowsCurrentPreservationIdentity({ready,originalReady,caller,tokenSha256,expectedTokenSha256,ownerId,expectedOwnerId,nativeBefore,nativeAfter,executableSha256,announcement,sourceReferences,observedAt}){
  for(const fixture of[ready,originalReady]){assert.equal(fixture.schema,'nomifun.windows-global-os-fixture.v1');assert.equal(fixture.status,'ready');assert.equal(fixture.native_data_root,win32.join(fixture.parent,'native-data'));assert.equal(fixture.token_file,win32.join(fixture.parent,'owner-jwt'));}
  for(const key of['parent','native_data_root','token_file'])assert.equal(ready[key],originalReady[key]);
  assert.equal(ready.owner_jwt_reused,true);assert.equal(ready.upgrade.native_data_root_unchanged,true);assert.equal(ready.upgrade.owner_jwt_unchanged,true);assert.equal(ready.upgrade.new_dataset_created,false);
  assert.equal(ready.prior_app_pid,originalReady.app_pid);assert.equal(ready.exe_sha256,ready.upgrade.new_exe_sha256);
  assert.equal(caller.platform,'windows');assert.equal(caller.process_id,ready.app_pid);assert.deepEqual(announcement,ready.announcement);assert.equal(announcement.pid,ready.app_pid);
  assert.equal(String(announcement.port),new URL(ready.base_url).port);assert.match(tokenSha256,/^[a-f0-9]{64}$/);assert.equal(tokenSha256,expectedTokenSha256);assert.equal(ownerId,expectedOwnerId);
  assert.equal(executableSha256,ready.exe_sha256);assert.match(executableSha256,/^[a-f0-9]{64}$/);
  for(const native of[nativeBefore,nativeAfter]){assert.equal(native.pid,ready.app_pid);assert.equal(native.state,'running');assert.match(native.creation_filetime,/^[1-9][0-9]+$/);assert.equal(native.creation_filetime,ready.actual_app_identity.native_process.creation_filetime);assert(samePath(native.image_path,ready.exe));}
  const created=Number(BigInt(nativeBefore.creation_filetime)/10000n-11644473600000n);assert(Number.isFinite(Date.parse(observedAt))&&created<=Date.parse(observedAt));
  for(const source of Object.values(sourceReferences)){assert(isAbsolute(source.path));assert.match(source.sha256,/^[a-f0-9]{64}$/);}
  return{schema:'nomifun.windows-current-upgrade-observation.v1',observed_at:observedAt,app_pid:ready.app_pid,current_ready:sourceReferences.current_ready,original_desktop_guard:sourceReferences.desktop_guard,original_unc_guard:sourceReferences.unc_guard,
    original_ready:sourceReferences.original_ready,native_data_root:ready.native_data_root,token_file:ready.token_file,owner_jwt_sha256:tokenSha256,authenticated_owner_id:ownerId,authenticated_native_caller:{platform:caller.platform,process_id:caller.process_id},native_process_before:nativeBefore,native_process_after:nativeAfter,executable_sha256:executableSha256,
    original_report_mutations:0,agent_calls_dispatched:0,reconciliation_requests:0,new_owner_attestation_created:false,new_dataset_created:false};
}
export async function freezeWindowsCurrentUpgradePreservations({fixtureReport,originalFixtureReport,desktopGuardReport,additionalGuardReport,reportRoot}){
  assert.equal(process.platform,'win32');
  const[current,original,desktop]=await Promise.all([stableReport(fixtureReport),stableReport(originalFixtureReport),stableReport(desktopGuardReport)]),ready=current.value;
  assertWindowsDesktopPreservationSource({source:original.value,preserved:desktop.value,preservedBytes:desktop.bytes,preservedSessionReport:desktop.path});
  const additional=await loadWindowsAdditionalSessionReport(resolve(additionalGuardReport),original.value),beforeUnc=additional.value;
  assert.equal(ready.upgrade.preserved_session_report,desktop.path);assert.equal(ready.upgrade.preserved_session_report_sha256,desktop.sha256);
  assert.equal(ready.upgrade.additional_session_report,additional.path);assert.equal(ready.upgrade.additional_session_report_sha256,additional.sha256);
  assert(samePath(dirname(ready.parent),await realpath(tmpdir())));for(const path of[ready.parent,ready.native_data_root]){assert(samePath(await realpath(path),path));await assertWindowsPrivatePath(path);}
  assert.equal((await readWindowsOwnedFixtureBytes(join(ready.parent,'.nomifun-global-os-fixture'),4096)).toString('utf8'),'nomifun-global-os-native-acceptance-fixture.v1\n');
  const tokenBytes=await readWindowsOwnedFixtureBytes(ready.token_file,4096),token=tokenBytes.toString('utf8').trim(),claims=JSON.parse(Buffer.from(token.split('.')[1],'base64url'));
  const announcement=JSON.parse((await readWindowsOwnedFixtureBytes(join(ready.native_data_root,'port.json'),4096)).toString('utf8'));
  const get=async path=>{const response=await fetch(new URL(path,ready.base_url),{headers:{Authorization:`Bearer ${token}`},signal:AbortSignal.timeout(15000)});assert(response.ok);const body=await response.json();assert.notEqual(body.success,false);return body.data??body;};
  const nativeBefore=windowsProcessObservation(ready.app_pid,{includeImagePath:true}),executableSha256=await stableImageDigest(ready.exe),caller=await get('/api/system/permissions');
  const desktopPath=`/api/agent-sessions/${desktop.value.session_id}`,session=await get(desktopPath),execution=await get(`${desktopPath}/execution`),events=[];let after=0;
  for(let page=0;page<20;page++){const result=await get(`${desktopPath}/events?after_seq=${after}&limit=500`);events.push(...result.events);if(!result.events.length||result.next_cursor.seq<=after)break;after=result.next_cursor.seq;assert(page<19);}
  assert.equal(events.length,desktop.value.canonical_prefix.event_count);assert.equal(session.head.last_seq,desktop.value.canonical_prefix.last_seq);assert(!session.head.active_turn_id);
  assert.deepEqual(session.session.owner_ref,{principal_kind:'user',principal_id:claims.user_id});const desktopProof=assertWindowsUpgradePreservation({before:desktop.value,execution,events});
  const observation=await readWindowsAdditionalSessionFacts(get),uncProof=assertWindowsAdditionalSessionPreservation({before:beforeUnc,fixture:ready,tokenSha256:hash(tokenBytes),observation});
  assert.equal(uncProof.transition,'pending_preserved');assert.equal(uncProof.pending_effects,1);assert.equal(uncProof.unknown_effects,0);
  const nativeAfter=windowsProcessObservation(ready.app_pid,{includeImagePath:true}),observedAt=new Date().toISOString(),ref=source=>({path:source.path,sha256:source.sha256});
  const identity=validateWindowsCurrentPreservationIdentity({ready,originalReady:original.value,caller,tokenSha256:hash(tokenBytes),expectedTokenSha256:beforeUnc.fixture.owner_jwt_sha256,ownerId:claims.user_id,expectedOwnerId:beforeUnc.session.owner_ref.principal_id,nativeBefore,nativeAfter,executableSha256,announcement,observedAt,
    sourceReferences:{current_ready:ref(current),original_ready:ref(original),desktop_guard:ref(desktop),unc_guard:ref(additional)}});
  for(const source of[current,original,desktop,additional])assert.equal((await stableReport(source.path)).sha256,source.sha256);
  assert.equal(hash(await readWindowsOwnedFixtureBytes(ready.token_file,4096)),hash(tokenBytes));
  const nextDesktop={...desktop.value,app_pid:ready.app_pid,observed_at:observedAt,current_app_observation:identity,current_session:{session:session.session,head:session.head},current_preservation:desktopProof};
  const nextUnc={...beforeUnc,app_pid:ready.app_pid,observed_at:observedAt,current_app_observation:identity,current_preservation:uncProof};
  const suffix=randomUUID(),desktopPathOut=join(resolve(reportRoot),`windows-current-desktop-preservation-${suffix}.json`),uncPathOut=join(resolve(reportRoot),`windows-current-unc-preservation-${suffix}.json`);
  await writeFile(desktopPathOut,JSON.stringify(nextDesktop,null,2)+'\n',{flag:'wx'});await writeFile(uncPathOut,JSON.stringify(nextUnc,null,2)+'\n',{flag:'wx'});
  const report={schema:'nomifun.windows-current-preservation-freeze.v1',status:'passed',...identity,desktop:{path:desktopPathOut,sha256:hash(await readFile(desktopPathOut)),event_count:events.length,proof:desktopProof},unc:{path:uncPathOut,sha256:hash(await readFile(uncPathOut)),event_count:observation.events.length,proof:uncProof}};
  const path=join(resolve(reportRoot),`windows-current-preservation-freeze-${suffix}.json`);await writeFile(path,JSON.stringify(report,null,2)+'\n',{flag:'wx'});console.log(JSON.stringify({status:'passed',report:path,app_pid:ready.app_pid,desktop_report:desktopPathOut,unc_report:uncPathOut,desktop_events:events.length,unc_events:observation.events.length,pending_effects:1,mutating_requests:0}));return report;
}
if(process.argv[1]===fileURLToPath(import.meta.url)){const flag=name=>{const index=process.argv.indexOf(name);assert(index>=0&&process.argv[index+1]);return process.argv[index+1];};await freezeWindowsCurrentUpgradePreservations({fixtureReport:flag('--fixture-report'),originalFixtureReport:flag('--original-fixture-report'),desktopGuardReport:flag('--desktop-guard-report'),additionalGuardReport:flag('--additional-guard-report'),reportRoot:flag('--report-root')});}
