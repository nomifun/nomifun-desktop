// Protocol/metadata guards only; these do not prove native Browser teardown.
import { test, expect } from 'bun:test';
import { win32 } from 'node:path';
import { selectNewWindowsBrowserSnapshot, validateWindowsBrowserSnapshotReceipt, validateWindowsBrowserHostIdentity,observeWindowsBrowserSnapshotAfterAppClose } from './global-os-windows-browser-snapshots.mjs';

const temporaryRoot = 'C:\\Users\\muli\\AppData\\Local\\Temp';
const metadata = (path, directory, fileId = '2') => ({ path, identity: { device: '1', file_id: fileId, birthtime_ns: '123000000' }, directory, file: !directory, link: false, size: directory ? 0 : 80, mtime_ms: 1704164645000 });
const fixture = () => {
  const root = metadata(win32.join(temporaryRoot, 'nomifun-browser-upload-Ab12Cd'), true);
  const names = ['absolute 中文.txt', 'relative 空格.txt', 'linked-文件.txt'];
  const files = names.map((name, index) => { const stage = metadata(win32.join(root.path, String(index)), true, `${index + 3}`); return { ...metadata(win32.join(stage.path, name), false, `${index + 6}`), stage, expected: { name, size: 80, lastModified: 1704164645000 } }; });
  return { schema: 'nomifun.windows-browser-upload-snapshot.v1', fixture_nonce: '11111111-1111-4111-8111-111111111111', temporary_root: temporaryRoot, temporary_identity: root.identity, root, files,
    native_attributes: [root, ...files.flatMap(file => [file.stage, file])].map(value => ({ path: value.path, attributes: value.directory ? 16 : 32 })), file_content_reads: 0, filesystem_mutations: 0 };
};

test('snapshot attribution requires exactly one new real root and never adopts a baseline root or sibling', () => {
  const root = fixture().root, old = metadata(win32.join(temporaryRoot, 'nomifun-browser-upload-Old123'), true, '9');
  const input = { temporaryRoot, baseline: { entries: [old] }, current: { entries: [old, root] } };
  expect(selectNewWindowsBrowserSnapshot(input)).toEqual(root);
  for (const current of [{ entries: [old] }, { entries: [old, root, { ...root, path: win32.join(temporaryRoot, 'nomifun-browser-upload-Other1') }] }, { entries: [{ ...root, path: 'C:\\Other\\nomifun-browser-upload-Ab12Cd' }] }, { entries: [{ ...root, link: true }] }, { entries: [{ ...root, path: win32.join(temporaryRoot, 'nomifun-browser-upload-..') }] }]) {
    expect(() => selectNewWindowsBrowserSnapshot({ ...input, current })).toThrow();
  }
  expect(() => selectNewWindowsBrowserSnapshot({ ...input, current: { entries: [{ ...old, identity: root.identity }, root] } })).toThrow();
});

test('retained snapshot proof preserves the controlled leaf names, stage indexes, bytes and mtimes without reading content', () => {
  expect(validateWindowsBrowserSnapshotReceipt(fixture()).files).toHaveLength(3);
  for (const change of [value => { value.files[0].size++; }, value => { value.files[0].mtime_ms++; }, value => { value.files[0].path = win32.join(value.root.path, 'other.txt'); }, value => { value.files[1].stage.path = value.files[0].stage.path; }, value => { value.files[2].file = false; }, value => { value.files[1].expected.name = '..'; }, value => { value.file_content_reads = 1; }, value => { value.filesystem_mutations = 1; }]) {
    const receipt = fixture(); change(receipt); expect(() => validateWindowsBrowserSnapshotReceipt(receipt)).toThrow();
  }
});

test('native reparse attributes, foreign roots and changed snapshot type are never accepted as a cleanup authority', () => {
  for (const change of [value => { value.native_attributes[0].attributes |= 1024; }, value => { value.native_attributes[2].attributes |= 1024; }, value => { value.native_attributes[2].attributes |= 16; }, value => { delete value.native_attributes[0].attributes; }, value => { value.root.link = true; }, value => { value.root.path = 'C:\\Users\\muli\\Downloads\\nomifun-browser-upload-Ab12Cd'; }, value => { value.root.identity.file_id = '0'; }]) {
    const receipt = fixture(); change(receipt); expect(() => validateWindowsBrowserSnapshotReceipt(receipt)).toThrow();
  }
});

test('post-close observation requires the same successful Browser fixture and an explicit normal close of its exact App PID', async () => {
  const receipt = fixture();
  const browserReport = { scenario: 'browser', status: 'passed', native_case_evidence: [{ kind: 'native_upload_snapshot_metadata', receipt }], native_case_resources: { root: `C:\\fixture\\browser-cases-${receipt.fixture_nonce}` }, initial_native_announcement: { pid: 1234 } };
  await expect(observeWindowsBrowserSnapshotAfterAppClose({ browserReport, closeReport: { app_pid: 4321, normal_close_requested: true } })).rejects.toThrow();
  await expect(observeWindowsBrowserSnapshotAfterAppClose({ browserReport, closeReport: { app_pid: 1234, normal_close_requested: false } })).rejects.toThrow('normal native App close receipt');
  await expect(observeWindowsBrowserSnapshotAfterAppClose({ browserReport: { ...browserReport, status: 'failed' }, closeReport: { app_pid: 1234, normal_close_requested: true } })).rejects.toThrow();
});

function supplementalHost(){
  const sha='a'.repeat(64),exe='C:\\fixture\\nomifun-desktop.exe',parent='C:\\fixture\\owner';
  const browserReport={schema:'nomifun.global-os-native-regression.v2',platform:'win32',scenario:'browser',status:'passed',session_id:'browser-session',run_id:'browser-run',host:'http://127.0.0.1:13569',started_at:'2026-10-17T07:14:53.926Z',finished_at:'2026-10-17T07:15:16.232Z'};
  const process={pid:30552,state:'running',creation_filetime:'134366923802594258',image_path:exe},caller={platform:'windows',process_id:30552},announcement={host:'127.0.0.1',port:13569,pid:30552};
  const ready={schema:'nomifun.windows-global-os-fixture.v1',status:'ready',app_pid:30552,base_url:browserReport.host,parent,native_data_root:win32.join(parent,'native-data'),token_file:win32.join(parent,'owner-jwt'),announcement,exe,exe_sha256:sha,login_identity:{sid:'login-sid'},actual_app_identity:{sid:'login-sid',integrity_rid:8192,native_process:process},readonly_readiness_verification:{observed_at:'2026-10-17T06:34:19.779Z'},authenticated_native_caller:caller};
  const hostIdentity={schema:'nomifun.windows-browser-host-identity.v1',observed_at:'2026-10-17T07:18:00.000Z',browser_report_sha256:sha,session_id:browserReport.session_id,browser_run_id:browserReport.run_id,host:browserReport.host,browser_started_at:browserReport.started_at,browser_finished_at:browserReport.finished_at,native_ready_report_sha256:'b'.repeat(64),native_ready:ready,app_pid:30552,announcement,authenticated_native_caller:caller,authenticated_owner_id:'owner',owner:{principal_kind:'user',principal_id:'owner'},native_process_before:structuredClone(process),native_process_after:structuredClone(process),executable_sha256:sha,original_report_mutations:0,agent_calls_dispatched:0,snapshot_content_reads:0};
  return{browserReport,browserReportSha256:sha,hostIdentity};
}
test('an older Browser report can use an explicit later identity proof without rewriting its initial fields',()=>{
  const input=supplementalHost();expect(input.browserReport.initial_native_announcement).toBeUndefined();expect(validateWindowsBrowserHostIdentity(input).pid).toBe(30552);expect(input.browserReport.initial_native_announcement).toBeUndefined();
  for(const mutate of [x=>{x.browserReportSha256='c'.repeat(64);},x=>{x.hostIdentity.browser_run_id='other';},x=>{x.hostIdentity.host='http://127.0.0.1:9999';},x=>{x.hostIdentity.native_process_after.creation_filetime+='0';},x=>{x.hostIdentity.native_process_after.image_path='C:\\foreign.exe';},x=>{x.hostIdentity.native_process_after.state='exited';},x=>{x.hostIdentity.executable_sha256='c'.repeat(64);},x=>{x.hostIdentity.owner.principal_id='other';},x=>{x.hostIdentity.announcement.port=9999;},x=>{x.hostIdentity.native_ready.readonly_readiness_verification.observed_at='2026-10-17T07:16:00.000Z';},x=>{x.hostIdentity.observed_at='2026-10-17T07:14:00.000Z';},x=>{x.hostIdentity.agent_calls_dispatched=1;}]){const changed=structuredClone(supplementalHost());mutate(changed);expect(()=>validateWindowsBrowserHostIdentity(changed)).toThrow();}
});
test('supplemental identity still requires the exact saved product exit receipt before any gone observation',async()=>{
  const input=supplementalHost(),receipt=fixture();input.hostIdentity.native_ready_report_path='C:\\fixture\\ready.json';
  const browserReport={...input.browserReport,native_case_evidence:[{kind:'native_upload_snapshot_metadata',receipt}],native_case_resources:{root:`C:\\fixture\\browser-cases-${receipt.fixture_nonce}`}};
  const closeReport={app_pid:30552,normal_close_requested:true,native_close_receipt:{schema:'nomifun.windows-global-os-fixture-close.v1',app_pid:30552,status:'exited',path:'existing_tauri_process_exit',actual_pid_gone:true,fixture_report:input.hostIdentity.native_ready_report_path,requested_at:'2026-10-17T07:20:00.000Z'}};
  for(const change of [value=>{value.native_close_receipt.actual_pid_gone=false;},value=>{value.native_close_receipt.path='host_kill';},value=>{value.native_close_receipt.app_pid=1234;},value=>{value.native_close_receipt.requested_at='2026-10-17T07:14:00.000Z';},()=>{}]){
    const changed=structuredClone(closeReport);change(changed);await expect(observeWindowsBrowserSnapshotAfterAppClose({browserReport,browserReportSha256:input.browserReportSha256,hostIdentity:input.hostIdentity,closeReport:changed})).rejects.toThrow();
  }
});
test('a new successful native launch uses its current verified readiness without an inherited readback observation',()=>{
  const input=supplementalHost(),ready=input.hostIdentity.native_ready;
  delete ready.readonly_readiness_verification;ready.ready_verified_at='2026-10-17T06:34:19.779Z';
  expect(validateWindowsBrowserHostIdentity(input).pid).toBe(30552);
  ready.readonly_readiness_verification={observed_at:'2026-10-17T06:00:00.000Z'};
  expect(validateWindowsBrowserHostIdentity(input).pid).toBe(30552);
  const stale=structuredClone(input);stale.hostIdentity.native_ready.ready_verified_at='2026-10-17T06:00:00.000Z';stale.hostIdentity.native_ready.readonly_readiness_verification.observed_at='2026-10-17T06:34:19.779Z';
  expect(()=>validateWindowsBrowserHostIdentity(stale)).toThrow();
  for(const observed of [undefined,'invalid','2026-10-17T06:00:00.000Z','2026-10-17T07:16:00.000Z']){
    const changed=structuredClone(input);changed.hostIdentity.native_ready.ready_verified_at=observed;
    expect(()=>validateWindowsBrowserHostIdentity(changed)).toThrow();
  }
});
test('both initial and supplemental identity paths reject missing or altered saved exit evidence',async()=>{
  const input=supplementalHost(),receipt=fixture(),base={...input.browserReport,native_case_evidence:[{kind:'native_upload_snapshot_metadata',receipt}],native_case_resources:{root:`C:\\fixture\\browser-cases-${receipt.fixture_nonce}`}};
  for(const supplemented of [false,true]){
    const browserReport=supplemented?base:{...base,initial_native_announcement:{pid:30552}},extra=supplemented?{hostIdentity:input.hostIdentity,browserReportSha256:input.browserReportSha256}:{};
    for(const native_close_receipt of [undefined,{schema:'foreign'},{schema:'nomifun.windows-global-os-fixture-close.v1',app_pid:30552,status:'exited',path:'host_kill',actual_pid_gone:true}])await expect(observeWindowsBrowserSnapshotAfterAppClose({browserReport,...extra,closeReport:{app_pid:30552,normal_close_requested:true,native_close_receipt}})).rejects.toThrow();
  }
});
