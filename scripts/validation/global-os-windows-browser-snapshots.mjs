// Read-only acceptance evidence. Snapshot contents are never opened here.
import assert from 'node:assert/strict';
import { lstat, open, readFile, readdir, realpath } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { basename,dirname,isAbsolute,join, resolve,win32 } from 'node:path';
import { assertWindowsPrivatePath, readWindowsOwnedFixtureBytes, psLiteral, windowsObservation, windowsProcessObservation } from './global-os-windows-cases.mjs';

const prefix = 'nomifun-browser-upload-';
const snapshotName = name => /^nomifun-browser-upload-[A-Za-z0-9]{6,}$/.test(name);
const stamp = () => new Date().toISOString();
const identity = value => ({ device: String(value.dev), file_id: String(value.ino), birthtime_ns: String(value.birthtimeNs) });
const sameIdentity = (left, right) => left.device === right.device && left.file_id === right.file_id && left.birthtime_ns === right.birthtime_ns;
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const samePath=(left,right)=>left.toLowerCase()===right.toLowerCase();
async function stableReport(path){
  assert(isAbsolute(path));
  const before=await lstat(path);assert(before.isFile()&&!before.isSymbolicLink()&&before.size<=16*1024*1024);
  const bytes=await readFile(path),after=await lstat(path);for(const key of ['ino','dev','size','mtimeMs'])assert.equal(after[key],before[key]);assert.equal(bytes.length,before.size);
  return{value:JSON.parse(bytes),sha256:hash(bytes)};
}
async function stableImageDigest(path){
  const before=await lstat(path);assert(before.isFile()&&!before.isSymbolicLink()&&before.size<=512*1024*1024);
  const handle=await open(path,'r'),digest=createHash('sha256');
  try{const opened=await handle.stat();for(const key of ['ino','dev','size','mtimeMs'])assert.equal(opened[key],before[key]);for await(const bytes of handle.createReadStream({autoClose:false}))digest.update(bytes);
    for(const current of [await handle.stat(),await lstat(path)])for(const key of ['ino','dev','size','mtimeMs'])assert.equal(current[key],before[key]);return digest.digest('hex');
  }finally{await handle.close();}
}
export function validateWindowsBrowserHostIdentity({browserReport,browserReportSha256,hostIdentity}){
  assert.equal(hostIdentity.schema,'nomifun.windows-browser-host-identity.v1');assert.match(browserReportSha256,/^[0-9a-f]{64}$/);assert.equal(hostIdentity.browser_report_sha256,browserReportSha256);
  assert.equal(browserReport.schema,'nomifun.global-os-native-regression.v2');assert.equal(browserReport.platform,'win32');assert.equal(browserReport.status,'passed');assert.equal(browserReport.scenario,'browser');assert.equal(hostIdentity.session_id,browserReport.session_id);assert.equal(hostIdentity.browser_run_id,browserReport.run_id);assert.equal(hostIdentity.host,browserReport.host);
  assert.equal(hostIdentity.browser_started_at,browserReport.started_at);assert.equal(hostIdentity.browser_finished_at,browserReport.finished_at);
  const ready=hostIdentity.native_ready;assert.match(hostIdentity.native_ready_report_sha256,/^[0-9a-f]{64}$/);assert.equal(ready.schema,'nomifun.windows-global-os-fixture.v1');assert.equal(ready.status,'ready');assert.equal(ready.base_url,browserReport.host);
  const origin=new URL(ready.base_url);assert.equal(origin.protocol,'http:');assert.equal(origin.hostname,'127.0.0.1');
  assert.equal(ready.native_data_root,join(ready.parent,'native-data'));assert.equal(ready.token_file,join(ready.parent,'owner-jwt'));
  const pid=ready.app_pid;assert(Number.isSafeInteger(pid)&&pid>1);assert.equal(hostIdentity.app_pid,pid);assert.equal(ready.announcement.pid,pid);assert.equal(hostIdentity.announcement.pid,pid);
  assert.deepEqual(hostIdentity.announcement,ready.announcement);assert.equal(String(hostIdentity.announcement.port),new URL(browserReport.host).port);assert.equal(hostIdentity.authenticated_native_caller.platform,'windows');assert.equal(hostIdentity.authenticated_native_caller.process_id,pid);
  assert.equal(ready.authenticated_native_caller.platform,'windows');assert.equal(ready.authenticated_native_caller.process_id,pid);
  const original=ready.actual_app_identity.native_process;assert.equal(original.pid,pid);assert.equal(original.state,'running');assert.match(original.creation_filetime,/^[1-9][0-9]+$/);assert(samePath(original.image_path,ready.exe));
  assert.equal(ready.actual_app_identity.sid,ready.login_identity.sid);assert.equal(ready.actual_app_identity.integrity_rid,8192);assert.equal(hostIdentity.owner.principal_kind,'user');assert.equal(hostIdentity.owner.principal_id,hostIdentity.authenticated_owner_id);
  const began=Date.parse(browserReport.started_at),finished=Date.parse(browserReport.finished_at),readiness=Date.parse(ready.ready_verified_at??ready.readonly_readiness_verification?.observed_at),observed=Date.parse(hostIdentity.observed_at);
  assert(Number.isFinite(began)&&finished>=began&&readiness<=began&&observed>=finished,'The explicit supplemental observation cannot masquerade as a pre-run observation');
  const creationMs=Number(BigInt(original.creation_filetime)/10000n-11644473600000n);assert(creationMs<=readiness&&creationMs<=began);
  for(const process of [hostIdentity.native_process_before,hostIdentity.native_process_after]){assert.equal(process.pid,pid);assert.equal(process.state,'running');assert.equal(process.creation_filetime,original.creation_filetime);assert(samePath(process.image_path,ready.exe));}
  assert.equal(hostIdentity.executable_sha256,ready.exe_sha256);assert.match(ready.exe_sha256,/^[0-9a-f]{64}$/);assert.equal(hostIdentity.original_report_mutations,0);assert.equal(hostIdentity.agent_calls_dispatched,0);assert.equal(hostIdentity.snapshot_content_reads,0);
  return{pid,creation_filetime:original.creation_filetime,image_path:ready.exe,source_browser_report_sha256:browserReportSha256,source_native_ready_report_sha256:hostIdentity.native_ready_report_sha256};
}
// This supplements an older driver report explicitly. It never edits that
// report or represents this later read-only observation as its initial facts.
export async function acquireWindowsBrowserHostIdentity({browserReportPath,fixtureReportPath}){
  assert.equal(process.platform,'win32');const source=await stableReport(browserReportPath),fixture=await stableReport(fixtureReportPath),ready=fixture.value;
  assert.equal(ready.schema,'nomifun.windows-global-os-fixture.v1');assert.equal(ready.status,'ready');assert.equal(ready.base_url,source.value.host);const origin=new URL(ready.base_url);assert.equal(origin.protocol,'http:');assert.equal(origin.hostname,'127.0.0.1');
  assert.match(basename(ready.parent),/^nomifun-global-os-owner-[0-9a-f-]{36}$/);assert(samePath(dirname(ready.parent),await realpath(tmpdir())));assert.equal(ready.native_data_root,join(ready.parent,'native-data'));assert.equal(ready.token_file,join(ready.parent,'owner-jwt'));
  for(const path of [ready.parent,ready.native_data_root]){assert(samePath(await realpath(path),path));await assertWindowsPrivatePath(path);}
  assert.equal((await readWindowsOwnedFixtureBytes(join(ready.parent,'.nomifun-global-os-fixture'),4096)).toString('utf8'),'nomifun-global-os-native-acceptance-fixture.v1\n');
  const announcement=JSON.parse((await readWindowsOwnedFixtureBytes(join(ready.native_data_root,'port.json'),4096)).toString('utf8'));assert.equal(announcement.pid,ready.app_pid);assert.equal(String(announcement.port),origin.port);assert.deepEqual(announcement,ready.announcement);
  const token=(await readWindowsOwnedFixtureBytes(ready.token_file,4096)).toString('utf8').trim(),claims=JSON.parse(Buffer.from(token.split('.')[1],'base64url'));
  const get=async path=>{const response=await fetch(new URL(path,ready.base_url),{headers:{Authorization:`Bearer ${token}`},signal:AbortSignal.timeout(15000)});assert(response.ok);const body=await response.json();assert.notEqual(body.success,false);return body.data??body;};
  const before=windowsProcessObservation(ready.app_pid,{includeImagePath:true});
  assert.equal(before.state,'running');assert.equal(before.pid,ready.app_pid);assert.equal(before.creation_filetime,ready.actual_app_identity.native_process.creation_filetime);assert(samePath(before.image_path,ready.exe));
  const sha256=await stableImageDigest(ready.exe);assert.equal(sha256,ready.exe_sha256);
  const caller=await get('/api/system/permissions'),session=await get(`/api/agent-sessions/${source.value.session_id}`),after=windowsProcessObservation(ready.app_pid,{includeImagePath:true});
  const nativeReady=Object.fromEntries(['schema','status','base_url','parent','native_data_root','token_file','app_pid','announcement','login_identity','actual_app_identity','ready_verified_at','readonly_readiness_verification','authenticated_native_caller','exe','exe_sha256'].map(key=>[key,ready[key]]));
  const result={schema:'nomifun.windows-browser-host-identity.v1',observed_at:stamp(),browser_report_path:resolve(browserReportPath),browser_report_sha256:source.sha256,session_id:source.value.session_id,browser_run_id:source.value.run_id,host:source.value.host,browser_started_at:source.value.started_at,browser_finished_at:source.value.finished_at,
    native_ready_report_path:resolve(fixtureReportPath),native_ready_report_sha256:fixture.sha256,native_ready:nativeReady,app_pid:ready.app_pid,announcement,authenticated_native_caller:caller,authenticated_owner_id:claims.user_id,owner:session.session.owner_ref,native_process_before:before,native_process_after:after,executable_sha256:sha256,
    original_report_mutations:0,agent_calls_dispatched:0,snapshot_content_reads:0};
  validateWindowsBrowserHostIdentity({browserReport:source.value,browserReportSha256:source.sha256,hostIdentity:result});assert.equal((await stableReport(browserReportPath)).sha256,source.sha256);assert.equal((await stableReport(fixtureReportPath)).sha256,fixture.sha256);return result;
}
async function metadata(path) {
  const value = await lstat(path, { bigint: true });
  return { path, identity: identity(value), directory: value.isDirectory(), file: value.isFile(), link: value.isSymbolicLink(), size: Number(value.size), mtime_ms: Number(value.mtimeNs / 1000000n) };
}
async function nativeAttributes(paths) {
  const values = await windowsObservation(`@(${paths.map(path => `[pscustomobject]@{path=${psLiteral(path)};attributes=[int][IO.File]::GetAttributes(${psLiteral(path)})}`).join(';')})|ConvertTo-Json -Compress`);
  const rows = Array.isArray(values) ? values : [values];
  assert.equal(rows.length, paths.length);
  rows.forEach((row, index) => { assert.equal(row.path, paths[index]); assert(Number.isInteger(row.attributes)); assert.equal(row.attributes & 1024, 0, 'Never inspect a reparse snapshot path'); });
  return rows;
}
async function inventory(root) {
  const names = (await readdir(root)).filter(name => name.startsWith(prefix)).sort();
  const entries = await Promise.all(names.map(name => metadata(join(root, name))));
  return { at: stamp(), entries };
}

export function selectNewWindowsBrowserSnapshot({ temporaryRoot, baseline, current }) {
  assert(win32.isAbsolute(temporaryRoot));
  const known = new Set(baseline.entries.map(entry => entry.path));
  for (const previous of baseline.entries) {
    const observed = current.entries.find(entry => entry.path === previous.path);
    assert(observed && sameIdentity(observed.identity, previous.identity), 'A preexisting snapshot changed during the attribution window');
  }
  const added = current.entries.filter(entry => !known.has(entry.path));
  assert.equal(added.length, 1, 'Exactly one new snapshot must appear during this serialized canonical upload');
  const candidate = added[0];
  assert.equal(win32.dirname(candidate.path), temporaryRoot);
  assert(snapshotName(win32.basename(candidate.path)));
  assert(candidate.directory && !candidate.file && !candidate.link, 'The new snapshot must be a real directory');
  return candidate;
}

export function validateWindowsBrowserSnapshotReceipt(receipt) {
  assert.equal(receipt?.schema, 'nomifun.windows-browser-upload-snapshot.v1');
  assert.match(receipt.fixture_nonce, /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
  assert(win32.isAbsolute(receipt.temporary_root));
  for (const value of [receipt.temporary_identity, receipt.root.identity]) { assert.match(value.device, /^[0-9]+$/); assert.match(value.file_id, /^[1-9][0-9]*$/); assert.match(value.birthtime_ns, /^[1-9][0-9]*$/); }
  assert.equal(win32.dirname(receipt.root.path), receipt.temporary_root);
  assert(snapshotName(win32.basename(receipt.root.path)));
  assert(receipt.root.directory && !receipt.root.link && !receipt.root.file);
  assert(receipt.root.identity.file_id && receipt.root.identity.file_id !== '0');
  assert.equal(receipt.files.length, 3);
  const allPaths = [receipt.root.path];
  receipt.files.forEach((file, index) => {
    assert(typeof file.expected.name === 'string' && file.expected.name.length > 0 && !/[\\/:\0]/.test(file.expected.name) && !['.', '..'].includes(file.expected.name));
    assert.equal(file.stage.path, win32.join(receipt.root.path, String(index)));
    assert(file.stage.directory && !file.stage.link && !file.stage.file);
    assert.equal(win32.dirname(file.path), file.stage.path);
    assert.equal(win32.basename(file.path), file.expected.name);
    assert(file.file && !file.directory && !file.link);
    assert.equal(file.size, file.expected.size);
    assert.equal(file.mtime_ms, file.expected.lastModified);
    allPaths.push(file.stage.path, file.path);
  });
  assert.equal(receipt.native_attributes.length, allPaths.length);
  receipt.native_attributes.forEach((entry, index) => { assert.equal(entry.path, allPaths[index]); assert(Number.isInteger(entry.attributes)); assert.equal(entry.attributes & 1024, 0); assert.equal((entry.attributes & 16) !== 0, index === 0 || index % 2 === 1); });
  assert.equal(receipt.file_content_reads, 0); assert.equal(receipt.filesystem_mutations, 0);
  return receipt;
}

export function createWindowsBrowserSnapshotObserver({ fixtureNonce, expected }) {
  assert.equal(expected.length, 3);
  let temporaryRoot, temporaryIdentity, baseline;
  return {
    async beforeUpload() {
      assert.equal(process.platform, 'win32'); assert.equal(baseline, undefined, 'Do not replay a snapshot observation window');
      temporaryRoot = await realpath(tmpdir());
      temporaryIdentity = await metadata(temporaryRoot); assert(temporaryIdentity.directory && !temporaryIdentity.link);
      baseline = await inventory(temporaryRoot);
      return { schema: 'nomifun.windows-browser-upload-baseline.v1', temporary_root: temporaryRoot, temporary_identity: temporaryIdentity.identity, fixture_nonce: fixtureNonce, ...baseline, file_content_reads: 0, filesystem_mutations: 0 };
    },
    async afterUpload() {
      assert(baseline, 'Capture the before-upload baseline first');
      assert(sameIdentity((await metadata(temporaryRoot)).identity, temporaryIdentity.identity));
      const current = await inventory(temporaryRoot), root = selectNewWindowsBrowserSnapshot({ temporaryRoot, baseline, current });
      await nativeAttributes([root.path]);
      assert.deepEqual((await readdir(root.path)).sort(), ['0', '1', '2']);
      const files = [];
      for (const [index, source] of expected.entries()) {
        const stage = await metadata(join(root.path, String(index))); assert(stage.directory && !stage.link);
        await nativeAttributes([stage.path]);
        assert.deepEqual(await readdir(stage.path), [source.name]);
        const file = await metadata(join(stage.path, source.name));
        files.push({ ...file, stage, expected: { name: source.name, size: source.size, lastModified: source.lastModified } });
      }
      const native_attributes = await nativeAttributes([root.path, ...files.flatMap(file => [file.stage.path, file.path])]);
      assert(sameIdentity((await metadata(root.path)).identity, root.identity), 'The observed root must not have been replaced');
      for (const file of files) {
        for (const previous of [file.stage, file]) { const current = await metadata(previous.path); assert(sameIdentity(current.identity, previous.identity)); assert.equal(current.link, false); assert.equal(current.size, previous.size); assert.equal(current.mtime_ms, previous.mtime_ms); }
      }
      return validateWindowsBrowserSnapshotReceipt({ schema: 'nomifun.windows-browser-upload-snapshot.v1', fixture_nonce: fixtureNonce, temporary_root: temporaryRoot, temporary_identity: temporaryIdentity.identity, root, files, native_attributes,
        baseline_at: baseline.at, observed_at: current.at, attribution: 'One newly created root during the single serialized canonical upload, with the three controlled file names, sizes and mtimes; no PID handle attribution is inferred.', file_content_reads: 0, filesystem_mutations: 0 });
    },
  };
}

export async function observeWindowsBrowserSnapshotState(receipt, { stage } = {}) {
  validateWindowsBrowserSnapshotReceipt(receipt);
  assert.equal(process.platform, 'win32'); assert(typeof stage === 'string' && stage.length > 0);
  assert.equal(await realpath(tmpdir()), receipt.temporary_root, 'Observe only this login process actual temporary directory');
  assert(sameIdentity((await metadata(receipt.temporary_root)).identity, receipt.temporary_identity));
  let current;
  try { current = await metadata(receipt.root.path); }
  catch (error) { if (error.code !== 'ENOENT') throw error; return { schema: 'nomifun.windows-browser-upload-snapshot-state.v1', at: stamp(), stage, root: receipt.root.path, original_identity: receipt.root.identity, absent: true, file_content_reads: 0, filesystem_mutations: 0 }; }
  assert(sameIdentity(current.identity, receipt.root.identity), 'The exact retained snapshot root changed identity; preserve it');
  assert(current.directory && !current.link); await nativeAttributes([receipt.root.path]);
  return { schema: 'nomifun.windows-browser-upload-snapshot-state.v1', at: stamp(), stage, root: receipt.root.path, original_identity: receipt.root.identity, current, absent: false, file_content_reads: 0, filesystem_mutations: 0 };
}

export async function observeWindowsBrowserSnapshotAfterAppClose({ browserReport, closeReport,hostIdentity,browserReportSha256 }) {
  assert.equal(browserReport.scenario, 'browser'); assert.equal(browserReport.status, 'passed');
  const proofs = browserReport.native_case_evidence.filter(value => value.kind === 'native_upload_snapshot_metadata');
  assert.equal(proofs.length, 1);
  const receipt = validateWindowsBrowserSnapshotReceipt(proofs[0].receipt);
  assert.equal(win32.basename(browserReport.native_case_resources.root), `browser-cases-${receipt.fixture_nonce}`);
  const supplemented=hostIdentity?validateWindowsBrowserHostIdentity({browserReport,browserReportSha256,hostIdentity}):undefined;
  const appPid = supplemented?.pid??browserReport.initial_native_announcement.pid;
  assert(Number.isSafeInteger(appPid) && appPid > 1); assert.equal(closeReport.app_pid, appPid);
  assert.equal(closeReport.normal_close_requested, true, 'The caller must supply its normal native App close receipt');
  const nativeClose=closeReport.native_close_receipt;assert.equal(nativeClose?.schema,'nomifun.windows-global-os-fixture-close.v1');assert.equal(nativeClose.app_pid,appPid);assert.equal(nativeClose.status,'exited');assert.equal(nativeClose.path,'existing_tauri_process_exit');assert.equal(nativeClose.actual_pid_gone,true);
  assert.match(closeReport.native_close_receipt_sha256,/^[a-f0-9]{64}$/);const closeSource=await stableReport(closeReport.native_close_receipt_path);assert.equal(closeSource.sha256,closeReport.native_close_receipt_sha256);assert.deepEqual(closeSource.value,nativeClose);
  if(supplemented){assert.equal(nativeClose.fixture_report,hostIdentity.native_ready_report_path);assert(Date.parse(nativeClose.requested_at)>=Date.parse(hostIdentity.observed_at));
    assert.equal((await stableReport(hostIdentity.browser_report_path)).sha256,browserReportSha256);assert.equal((await stableReport(hostIdentity.native_ready_report_path)).sha256,hostIdentity.native_ready_report_sha256);
  }
  const appProcess = windowsProcessObservation(appPid); assert(appProcess.state!=='running'||(supplemented&&appProcess.creation_filetime!==supplemented.creation_filetime), 'The exact App must actually have exited');
  const snapshot = await observeWindowsBrowserSnapshotState(receipt, { stage: 'after_normal_app_close' });
  assert.equal(snapshot.absent, true, 'The exact retained snapshot must actually be absent after native teardown');
  const before = closeReport.snapshot_before_close;
  if (before) { assert.equal(before.root, receipt.root.path); assert(sameIdentity(before.original_identity, receipt.root.identity)); }
  return { schema: 'nomifun.windows-browser-upload-native-teardown.v1', at: stamp(), session_id: browserReport.session_id, fixture_nonce: receipt.fixture_nonce, app_pid: appPid, app_process: appProcess, snapshot,
    ...(supplemented?{supplemental_host_identity:supplemented,host_identity_observed_at:hostIdentity.observed_at}:{}),release_stage: before?.absent === true ? 'already_absent_before_app_close' : before?.absent === false ? 'present_before_normal_close_and_absent_after' : 'absent_after_close_without_a_preclose_sample',
    file_content_reads: 0, filesystem_mutations: 0 };
}
