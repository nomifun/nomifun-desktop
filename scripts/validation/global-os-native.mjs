#!/usr/bin/env node
// Runs the public default-Agent ingress of an already running native App.
// The local scripted provider supplies deterministic calls. The App, canonical
// Session, file/process owners and OS authentication remain real. This is native
// product-path evidence, never a claim about live-model reasoning or GUI review.
import assert from 'node:assert/strict';
import { constants } from 'node:fs';
import { createHash, randomUUID } from 'node:crypto';
import { createServer } from 'node:http';
import { execFile } from 'node:child_process';
import { lstat, mkdir, open, readFile, readdir, realpath, rmdir, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { basename, dirname, isAbsolute, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const option = key => { const i = process.argv.indexOf(key); return i < 0 ? undefined : process.argv[i + 1]; };
const required = key => { const value = option(key); if (!value) throw new Error(`${key} is required`); return value; };
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
const present = async path => lstat(path).then(() => true, error => { if (error.code === 'ENOENT') return false; throw error; });
const contained = (root, target) => { const path = relative(root, target); return path === '' || (!path.startsWith('..') && !isAbsolute(path)); };
const toolContent = message => typeof message.content === 'string' ? message.content : JSON.stringify(message.content);
const toolValue = raw => {
  if (raw.startsWith('[tool error]')) throw new Error(`Native tool rejected the call: ${raw}`);
  try { return JSON.parse(raw); } catch (error) { throw new Error(`Native tool result is not JSON: ${raw}`, { cause: error }); }
};
const requireExited = value => { assert.equal(value.state, 'exited'); assert.equal(value.exit_code, 0); assert.equal(value.cleanup.reaped, true); };
const executeReadOnly = promisify(execFile);
const observeProcessUid = async pid => {
  assert(Number.isSafeInteger(pid) && pid > 1, 'A real native process PID is required for read-only identity observation');
  try {
    const value = await executeReadOnly('/bin/ps', ['-p', String(pid), '-o', 'uid='], { encoding: 'utf8', timeout: 5000, maxBuffer: 4096 });
    return { pid, exit_code: 0, stdout: value.stdout, stderr: value.stderr };
  } catch (error) {
    if (!Number.isInteger(error.code)) throw error;
    return { pid, exit_code: error.code, stdout: error.stdout ?? '', stderr: error.stderr ?? '' };
  }
};
const processIsGone = pid => {
  try { process.kill(pid, 0); return false; }
  catch (error) { if (error.code === 'ESRCH') return true; throw error; }
};
const readOwnedPublicFixtureFile = async path => {
  const before = await lstat(path);
  assert(before.isFile() && !before.isSymbolicLink() && before.nlink === 1 && before.uid === process.getuid() && before.size <= 4096,
    `Expected the owned, bounded public fixture metadata file: ${path}`);
  const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const opened = await file.stat();
    assert(opened.dev === before.dev && opened.ino === before.ino && opened.size <= 4096, `Fixture metadata identity changed: ${path}`);
    const value = await file.readFile('utf8');
    assert(Buffer.byteLength(value) <= 4096, `Fixture metadata exceeded its bound: ${path}`);
    return value;
  } finally { await file.close(); }
};
const validateNativeDataset = async root => {
  assert(isAbsolute(root) && basename(root) === 'native-data', '--native-data-root must be the exact private fixture native-data directory');
  assert.equal(await realpath(root), root, 'Native fixture data root must be canonical without symlink aliases');
  const parent = dirname(root);
  assert(/^\/private\/tmp\/nomifun-global-os-owner-[A-Za-z0-9-]+$/.test(parent), 'Only the explicitly marked private OS acceptance dataset is allowed');
  for (const directory of [parent, root]) {
    const info = await lstat(directory);
    assert(info.isDirectory() && !info.isSymbolicLink() && info.uid === process.getuid() && (info.mode & 0o077) === 0,
      `Native fixture directory must belong to the login user and be private: ${directory}`);
  }
  assert.equal(await readOwnedPublicFixtureFile(join(parent, '.nomifun-global-os-fixture')), 'nomifun-global-os-native-acceptance-fixture.v1\n', 'The native test dataset marker is missing or invalid');
  return { root, parent, portPath: join(root, 'port.json') };
};
const readNativeAnnouncement = async dataset => {
  const value = JSON.parse(await readOwnedPublicFixtureFile(dataset.portPath));
  assert(['127.0.0.1', 'localhost', '::1'].includes(value.host), 'Native announcement must be loopback only');
  assert(Number.isInteger(value.port) && value.port > 0 && value.port <= 65535 && Number.isInteger(value.pid) && value.pid > 1, 'Native announcement must contain a valid port and actual App PID');
  return value;
};

// This continuation is deliberately limited to the one failed acceptance run
// named in this task. It is not a general Session recovery or permission API.
export const nativeAdministratorContinuationFixture = Object.freeze({
  sessionId: '01a145f4-894d-73c3-ac2a-27d4d2918f87',
  template: 'assistant.general',
  nativeDataRoot: '/private/tmp/nomifun-global-os-owner-self-focus-bb87b6152c63472dbde4849a8df149bd/native-data',
  sourceReport: '/private/tmp/nomifun-global-os-20261016/native-product/administrator-assistant.general-1792175015239-b09384ec-d84a-4978-a295-fa54bd313b4c.json',
  sourceSha256: 'f84cf9979a38e44b128aa56ee87ebeb003db1c456e6c98787a57e9b41e6759bf',
  fixture: '/private/tmp/nomifun-global-os-product-d39a4481-ff23-4b91-b00b-5cf68de483fe',
  retryReport: '/private/tmp/nomifun-global-os-20261016/native-product/administrator-continuation-assistant.general-1792178647845-82d524cc-01b9-46b0-8b7c-fd27caf51809.json',
  retrySha256: '8cd98ff6b9cba7ee6052663b70380142a06b01f43a9e38ffff5082ba77e6efc1',
  retryEventsSha256: '920192d1d090e1267053a099252b6521e6339f88efaf0e4fdf934e83b7018085',
});
const stableJson = value => JSON.stringify(value, (_, entry) => entry && typeof entry === 'object' && !Array.isArray(entry)
  ? Object.fromEntries(Object.keys(entry).sort().map(key => [key, entry[key]])) : entry);
const evidenceDigest = value => createHash('sha256').update(stableJson(value)).digest('hex');
const eventPayload = event => { assert.equal(event.payload?.storage, 'inline_json', 'Continuation requires readable canonical event payloads'); return event.payload.value; };

export function nativeContinuationOptions({ administrator, scenario, template, continueSession, continuationSourceReport, continuationRetryReport, nativeDataRoot,
  retainServiceForAppExit = false, desktopBackgroundFocus = false, protectedUserDirectory = false }) {
  if (continueSession === undefined && continuationSourceReport === undefined) {
    assert.equal(continuationRetryReport, undefined, 'Retry requires the complete original continuation tuple'); return false;
  }
  assert(administrator === true && scenario === 'files-process', 'Continuation is only available for the independent administrator scenario');
  assert(!retainServiceForAppExit && !desktopBackgroundFocus && !protectedUserDirectory, 'Continuation cannot combine independent fixture scenarios');
  const pin = nativeAdministratorContinuationFixture;
  assert.equal(continueSession, pin.sessionId, 'Only the explicitly pinned failed administrator Session may continue');
  assert.equal(template, pin.template, 'Continuation must preserve the source template');
  assert.equal(nativeDataRoot, pin.nativeDataRoot, 'Continuation requires the exact fourth marked native dataset');
  assert([pin.sourceReport, pin.sourceReport.replace('/private/tmp/', '/tmp/')].includes(continuationSourceReport), 'Only the explicitly pinned private failed report is accepted');
  if (continuationRetryReport !== undefined)
    assert([pin.retryReport, pin.retryReport.replace('/private/tmp/', '/tmp/')].includes(continuationRetryReport), 'Only the explicitly pinned zero-user-dispatch failure may authorize a retry');
  return true;
}

export function nativeContinuationCliArguments(argv) {
  const continued = argv.includes('--continue-session') || argv.includes('--continuation-source-report') || argv.includes('--continuation-retry-report');
  if (!continued) return false;
  for (const key of ['--continue-session', '--continuation-source-report', '--native-data-root', '--token-file']) {
    assert.equal(argv.filter(value => value === key).length, 1, `${key} must occur exactly once for continuation`);
    const index = argv.indexOf(key);
    assert(typeof argv[index + 1] === 'string' && !argv[index + 1].startsWith('--'), `${key} requires an explicit value`);
  }
  for (const key of ['--administrator', '--template', '--base-url', '--report-dir'])
    assert.equal(argv.filter(value => value === key).length, 1, `${key} must occur exactly once for continuation`);
  assert(argv.filter(value => value === '--scenario').length <= 1, '--scenario cannot be repeated for continuation');
  if (argv.includes('--continuation-retry-report')) {
    assert.equal(argv.filter(value => value === '--continuation-retry-report').length, 1, '--continuation-retry-report must occur exactly once');
    const value = argv[argv.indexOf('--continuation-retry-report') + 1];
    assert(typeof value === 'string' && !value.startsWith('--'), '--continuation-retry-report requires an explicit value');
  }
  return true;
}

export async function readNativeOwnedFixtureBytes(path, maximum = 1024 * 1024) {
  const before = await lstat(path);
  assert(before.isFile() && !before.isSymbolicLink() && before.nlink === 1 && before.uid === process.getuid()
    && (before.mode & 0o077) === 0 && before.size <= maximum, 'Expected a private, singly linked, owned and bounded fixture file');
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const opened = await handle.stat();
    assert(opened.dev === before.dev && opened.ino === before.ino && opened.size === before.size, 'Fixture file changed while opening');
    const bytes = await handle.readFile();
    const after = await handle.stat(), pathAfter = await lstat(path);
    assert(bytes.length <= maximum && bytes.length === before.size && after.size === before.size && after.mtimeMs === before.mtimeMs
      && pathAfter.dev === before.dev && pathAfter.ino === before.ino && pathAfter.nlink === 1, 'Fixture file changed while reading');
    return bytes;
  } finally { await handle.close(); }
}

export function nativeContinuationSettlement({ execution, head, operationId, sessionId }) {
  assert(execution && ['cancelled', 'failed'].includes(execution.state), 'The source operation must have an actual failed/cancelled terminal');
  assert.equal(execution.operation_id, operationId, 'The latest execution must be the exact source operation');
  assert.equal(execution.turn_state, execution.state, 'Source execution and Turn terminal facts disagree');
  for (const key of ['producer_lease_live', 'checkpoint_retained', 'recovery_blocked', 'pause_requested', 'automatic_replay_authorized'])
    assert.equal(execution[key], false, `Continuation rejects missing or active ${key}`);
  for (const key of ['pending_effects', 'unknown_effects']) assert.equal(execution[key], 0, `Continuation rejects unresolved ${key}`);
  for (const key of ['checkpoint_seq', 'checkpoint_digest', 'pause']) assert.equal(execution[key], null, `Continuation rejects retained or unknown ${key}`);
  assert.equal(head?.session_id, sessionId); assert.equal(head.status, 'ready', 'The original Session must be settled and ready');
  assert(head.active_turn_id == null, 'An active/paused Turn cannot continue as new work');
  return { operation_id: operationId, state: execution.state, pending_effects: 0, unknown_effects: 0,
    producer_lease_live: false, checkpoint_retained: false, head_status: head.status, head_last_seq: head.last_seq };
}
export function nativeContinuationCaller({ origin, announcement, caller, uidObservation, driverUid }) {
  const url = new URL(origin);
  assert.equal(url.protocol, 'http:'); assert(!url.username && !url.password);
  assert.equal(url.hostname.replace(/^\[|\]$/g, ''), announcement.host);
  assert.equal(Number(url.port), announcement.port);
  assert.equal(caller.platform, 'macos'); assert.equal(caller.process_id, announcement.pid);
  assert.equal(caller.app_bundle_identifier, 'com.nomifun.desktop');
  assert.equal(uidObservation.pid, announcement.pid);
  assert(uidObservation.exit_code === 0 && uidObservation.stdout.trim() === String(driverUid) && uidObservation.stderr.trim() === '', 'The native App UID must equal the ordinary fixture driver UID');
}
export function nativeContinuationSession({ observation, sourceEvidence, selection, projection }) {
  assert.equal(observation.session.agent_session_id, nativeAdministratorContinuationFixture.sessionId);
  assert.deepEqual(observation.session.owner_ref, { principal_kind: 'user', principal_id: sourceEvidence.source_owner_id });
  assert(!observation.session.remote_binding_provenance && !observation.session.parent_session_id, 'Remote or derived attempt Sessions are not continuation targets');
  assert.equal(selection.editable, true, 'An Attempt/automation/frozen Session cannot be continued');
  assert.equal(selection.binding_version, observation.session.agent_binding.binding_version);
  assert.equal(projection.session_purpose, 'conversation');
  assert.equal(projection.extra.workspace, join(nativeAdministratorContinuationFixture.fixture, 'workspace'));
  assert.deepEqual(projection.agent_snapshot.canonical_binding, observation.session.agent_binding);
}

export function nativeContinuationSource(report, { sessionId, template, driverUid, sha256 }) {
  const pin = nativeAdministratorContinuationFixture;
  assert.equal(sha256, pin.sourceSha256, 'The pinned source report bytes changed');
  assert.equal(report.schema, 'nomifun.global-os-native-regression.v2');
  assert.equal(report.status, 'failed', 'A passed or checkpoint report cannot authorize continuation');
  assert.equal(report.scenario, 'administrator'); assert.equal(report.administrator, true);
  assert.equal(report.session_id, sessionId); assert.equal(sessionId, pin.sessionId);
  assert.equal(report.template, template); assert.equal(template, pin.template);
  assert.equal(report.fixture, pin.fixture); assert.equal(report.workspace, join(pin.fixture, 'workspace'));
  assert(!report.continuation && Array.isArray(report.phase_checkpoints) && report.phase_checkpoints.length === 0, 'Only the original failed main report may be the source');
  assert(report.failure && report.failure_settlement?.cleanup_proven === true && !report.failure_cleanup, 'Source native cleanup is not proven');
  const admission = report.accepted;
  assert(admission?.agent_session_id === sessionId && typeof admission.operation_id === 'string' && typeof admission.message_id === 'string');
  for (const candidate of [report.failure_admission, report.failed_phase?.accepted]) {
    assert(candidate && candidate.agent_session_id === sessionId && candidate.operation_id === admission.operation_id
      && candidate.message_id === admission.message_id, 'Source admission identities disagree');
  }
  assert.equal(report.failed_phase.role, 'main');
  const ownerMatch = admission.operation_id.match(/^turn:user:([0-9a-f-]{36}):([0-9a-f-]{36}):[0-9a-f-]{36}$/);
  assert(ownerMatch && ownerMatch[2] === sessionId, 'The source has no exact user operation identity');
  const denial = report.checks?.find(check => check.id === 'ordinary-user-denied' && check.status === 'passed');
  assert(denial && denial.driver_uid === driverUid && driverUid > 0 && denial.fixture_uid === 0 && denial.fixture_mode === '700', 'Source/driver UID or administrator fixture proof differs');
  nativeContinuationSettlement({ execution: report.failure_settlement.execution, head: report.failure_settlement.head, operationId: admission.operation_id, sessionId });
  assert(Array.isArray(report.failure_settlement.events) && report.failure_settlement.events.length > 0);
  return { source_sha256: sha256, source_status: report.status, source_operation_id: admission.operation_id,
    source_message_id: admission.message_id, source_owner_id: ownerMatch[1], driver_uid: driverUid, template,
    source_events_sha256: evidenceDigest(report.failure_settlement.events), source_head_last_seq: report.failure_settlement.head.last_seq };
}
export function nativeContinuationRetryAncestry(report, { sourceEvidence, driverUid, sha256 }) {
  const pin = nativeAdministratorContinuationFixture;
  assert.equal(sha256, pin.retrySha256, 'The exact retry source bytes changed');
  assert.equal(report.schema, 'nomifun.global-os-native-regression.v2'); assert.equal(report.status, 'failed');
  assert.equal(report.scenario, 'administrator'); assert.equal(report.administrator, true);
  assert.equal(report.session_id, pin.sessionId); assert.equal(report.template, pin.template);
  assert.equal(report.workspace, join(pin.fixture, 'workspace'));
  const ancestor = report.continuation;
  assert(ancestor && !ancestor.retry, 'A retry cannot recursively promote another retry report');
  assert.equal(ancestor.source_report, pin.sourceReport); assert.equal(ancestor.source_sha256, sourceEvidence.source_sha256);
  assert.equal(ancestor.source_operation_id, sourceEvidence.source_operation_id); assert.equal(ancestor.source_events_sha256, sourceEvidence.source_events_sha256);
  assert.equal(ancestor.source_owner_id, sourceEvidence.source_owner_id); assert.equal(ancestor.driver_uid, driverUid);
  assert.equal(ancestor.original_session_id, pin.sessionId); assert.equal(ancestor.template, pin.template);
  assert.equal(ancestor.native_app_identity?.native_data_root, pin.nativeDataRoot);
  assert.equal(ancestor.native_app_identity?.bundle_identifier, 'com.nomifun.desktop');
  assert.equal(ancestor.read_tool_schema?.administrator_supported, true);
  assert.equal(ancestor.read_tool_schema?.default_identity, 'current_user');
  assert(/^[a-f0-9]{64}$/.test(ancestor.read_tool_schema?.schema_sha256));
  assert.deepEqual([...(ancestor.read_tool_schema?.formats ?? [])].sort(), ['image', 'instruction_scope', 'text']);
  assert.equal(ancestor.original_failure_preservation_after_new_turn?.original_operation_preserved, true);
  assert.equal(ancestor.original_failure_preservation_after_new_turn?.source_report_sha256, pin.sourceSha256);
  assert(Array.isArray(report.checks) && report.checks.length === 0 && report.observations && Object.keys(report.observations).length === 0,
    'A retry is forbidden once a user verification step has run');
  assert.equal(report.remaining_fixture?.administrator_root, false); assert.equal(report.remaining_fixture?.outside, false);
  const admission = report.accepted;
  assert(admission && admission.agent_session_id === pin.sessionId && admission.operation_id !== sourceEvidence.source_operation_id);
  for (const item of [report.failure_admission, report.failed_phase?.accepted])
    assert(item && item.operation_id === admission.operation_id && item.message_id === admission.message_id && item.agent_session_id === pin.sessionId);
  assert(admission.operation_id.startsWith(`turn:user:${sourceEvidence.source_owner_id}:${pin.sessionId}:`));
  assert.equal(report.failed_phase?.role, 'main'); assert.equal(report.failure_settlement?.cleanup_proven, true);
  nativeContinuationSettlement({ execution: report.failure_settlement.execution, head: report.failure_settlement.head, operationId: admission.operation_id, sessionId: pin.sessionId });
  const events = report.failure_settlement.events;
  assert(Array.isArray(events) && events.length > 0 && events.every(event => event.agent_session_id === pin.sessionId && event.correlation_id === admission.operation_id));
  return { admission, events };
}
export function nativeContinuationRetrySource(report, args) {
  const { admission, events } = nativeContinuationRetryAncestry(report, args);
  assert.equal(evidenceDigest(events), nativeAdministratorContinuationFixture.retryEventsSha256, 'The zero-user-dispatch canonical source changed');
  const dispatchProof = nativeContinuationZeroUserDispatch(events, { operationId: admission.operation_id, runId: report.run_id });
  return { retry_report_sha256: args.sha256, retry_status: 'failed', retry_operation_id: admission.operation_id, retry_message_id: admission.message_id,
    retry_events_sha256: evidenceDigest(events), retry_run_id: report.run_id, retry_head_last_seq: report.failure_settlement.head.last_seq,
    ...dispatchProof,
    safety_basis: 'only canonical internal read dispatches; root proposal held before owner admission; current settlement still required; no effect replay' };
}
export function nativeContinuationZeroUserDispatch(events, { operationId, runId }) {
  const records = events.filter(event => event.kind === 'runtime/progress-recorded').map(event => ({ seq: event.seq, value: eventPayload(event).event }));
  assert(!events.some(event => event.kind.startsWith('effect/')), 'Any source effect requires its own reconciliation, not this retry');
  const proposals = records.filter(record => record.value?.event === 'tool_call_completed' && record.value.step > 0);
  assert.equal(proposals.length, 1, 'Only the unexecuted first fixture proposal is retryable');
  const proposal = proposals[0].value.call;
  assert.equal(proposal.call_id, nativeRunCallId(runId, 'admin-fixture'));
  assert.equal(proposal.name, 'exec_command'); assert.equal(proposal.arguments?.execution_identity, 'administrator');
  const held = records.find(record => record.value?.event === 'tool_completed' && record.value.result?.call_id === proposal.call_id)?.value;
  assert(held?.step === 1 && held.runtime_hold === 'runtime_gate' && held.result.is_error === true, 'The original root proposal must have been held before owner admission');
  assert(!records.some(record => record.value?.event === 'tool_started' && record.value.step > 0), 'A started user operation cannot use the zero-dispatch retry');
  const dispatched = records.filter(record => ['host_tool_dispatch', 'host_resource_dispatch'].includes(record.value?.event));
  assert.equal(dispatched.length, 3, 'Only the three canonical internal instruction reads may have dispatched');
  for (const { value } of dispatched) {
    const dispatch = value.dispatch;
    assert(value.event === 'host_tool_dispatch' && dispatch?.capability_id === 'workspace.files' && dispatch.action_id === 'workspace.files/read'
      && dispatch.model_name === 'read_file' && ['agent-instructions:100', 'agent-instructions:101', 'agent-instructions:102'].includes(dispatch.call_id)
      && dispatch.operation_id === `${operationId}:tool:${dispatch.call_id}`, 'Any opaque/process/write/resource dispatch forbids this retry');
    assert(records.some(record => record.value?.event === 'tool_call_completed' && record.value.step === 0
      && record.value.call?.call_id === dispatch.call_id && record.value.call.name === 'read_file'));
  }
  assert.equal(new Set(dispatched.map(record => record.value.dispatch.call_id)).size, 3, 'The three distinct internal read dispatches must be proven');
  return { actual_user_effect_dispatch_count: 0, held_user_proposal_call_id: proposal.call_id, internal_read_dispatch_count: dispatched.length };
}
export function nativeContinuationCancellationTarget({ latestOperationId, admissionOperationId, acceptedOperationId, pendingAdmissionOperationId }) {
  if (!acceptedOperationId && latestOperationId === admissionOperationId) return 'prior_settled_not_admitted';
  assert(latestOperationId === (acceptedOperationId ?? pendingAdmissionOperationId) && latestOperationId,
    'No cancellation is authorized for an operation outside this run');
  return 'current_run';
}

export function nativeRunCallId(runId, suffix) { return `native-${runId}-${suffix}`; }
export function nativeToolReceiptDisposition({ id, runId, known, active }) {
  if (typeof id !== 'string' || !id.startsWith(`native-${runId}-`) || !known) return 'historical';
  return active ? 'current_turn' : 'prior_turn';
}
export function nativeAdministratorReadSchema(tools) {
  const read = tools?.find(tool => tool.function?.name === 'read_file')?.function;
  assert(read && read.parameters?.type === 'object' && read.parameters.additionalProperties === false, 'The actual read_file tool must expose its closed formal schema');
  const identity = read.parameters.properties?.execution_identity;
  assert(identity?.type === 'string' && identity.default === 'current_user'
    && Array.isArray(identity.enum) && stableJson([...identity.enum].sort()) === stableJson(['administrator', 'current_user']), 'The actual frozen read_file schema does not support administrator identity');
  assert.deepEqual([...read.parameters.properties.format.enum].sort(), ['image', 'instruction_scope', 'text']);
  return { tool_name: read.name, action_id: 'workspace.files/read', schema_sha256: evidenceDigest(read.parameters),
    administrator_supported: true, default_identity: identity.default, formats: [...read.parameters.properties.format.enum] };
}
export function nativeContinuationMutation(method, path, sessionId) {
  if (method === 'GET') return;
  assert((method === 'POST' && path === '/api/providers') || (method === 'PUT' && path === `/api/agent-sessions/${sessionId}/model`)
    || (method === 'POST' && [`/api/agent-sessions/${sessionId}/turns`, `/api/agent-sessions/${sessionId}/turns/cancel`, `/api/agent-sessions/${sessionId}/execution/resume`].includes(path)),
  'Continuation cannot create an Agent/Session or change grants/resources');
}
const sourceRevisionIntent = revision => {
  const { model_route_refs, chat_route_records, ...intent } = revision.document;
  return evidenceDigest(intent);
};
const fileMaterialization = revision => {
  const lock = revision.contribution_locks?.find(lock => lock.contribution_id === 'module:workspace.files');
  assert(lock && /^[a-f0-9]{64}$/.test(lock.contract_digest), 'The exact saved file materialization lock is required');
  return lock.contract_digest;
};
export function nativeContinuationEvolution({ before, after, sourceBinding, sourceRevision, beforeRevision, afterRevision, events, sourceLastSeq }) {
  const original = sourceBinding, left = before.session.agent_binding, right = after.session.agent_binding;
  assert.equal(after.session.agent_session_id, before.session.agent_session_id); assert.deepEqual(after.session.owner_ref, before.session.owner_ref);
  assert.deepEqual(left.typed_resource_bindings, original.typed_resource_bindings, 'The original Session resources changed before continuation');
  assert.deepEqual(right.typed_resource_bindings, left.typed_resource_bindings, 'Model preparation changed the original resources/workspace');
  assert.equal(sourceRevisionIntent(sourceRevision), sourceRevisionIntent(beforeRevision), 'The original Agent intent changed before continuation');
  assert.equal(sourceRevisionIntent(afterRevision), sourceRevisionIntent(beforeRevision), 'Continuation changed non-model Agent intent or grants');
  assert(Number.isSafeInteger(left.binding_version) && Number.isSafeInteger(right.binding_version) && right.binding_version > left.binding_version,
    'The public model command must advance this same Session binding');
  const transitions = events.filter(event => event.seq > sourceLastSeq && event.kind === 'session/agent-binding-changed'
    && event.idempotency_key?.startsWith('contract-evolution:')).map(event => {
    const value = eventPayload(event);
    assert.equal(value.handoff_mode, 'context_only'); assert.equal(value.completion_gate_inherited, false);
    assert(!value.handoff_payload_id && !value.handoff_payload_digest && !value.workspace_relocation);
    assert.equal(value.next_binding_ref.binding_version, value.previous_binding_ref.binding_version + 1);
    assert.notDeepEqual(value.previous_binding_ref.resolved_snapshot_ref, value.next_binding_ref.resolved_snapshot_ref);
    assert(/^[a-f0-9]{64}$/.test(value.request_digest));
    return { seq: event.seq, event_id: event.event_id, event_sha256: evidenceDigest(event), request_digest: value.request_digest,
      previous_binding_ref: value.previous_binding_ref, next_binding_ref: value.next_binding_ref,
      observed_during_model_command: event.seq > before.head.last_seq };
  });
  const sourceFiles = fileMaterialization(sourceRevision), beforeFiles = fileMaterialization(beforeRevision), afterFiles = fileMaterialization(afterRevision);
  const currentEvolved = transitions.some(event => event.observed_during_model_command) && beforeFiles !== afterFiles;
  const ancestorEvolved = transitions.some(event => !event.observed_during_model_command) && sourceFiles !== beforeFiles;
  return { same_session_id: after.session.agent_session_id, original_binding_ref: { ...original, typed_resource_bindings: undefined },
    before_binding_ref: { ...left, typed_resource_bindings: undefined }, after_binding_ref: { ...right, typed_resource_bindings: undefined },
    source_file_materialization_digest: sourceFiles, before_file_materialization_digest: beforeFiles, after_file_materialization_digest: afterFiles,
    canonical_contract_transitions: transitions, contract_evolution_observed: transitions.length > 0,
    file_materialization_evolution_observed: sourceFiles !== afterFiles,
    contract_evolution_during_model_command: transitions.some(event => event.observed_during_model_command),
    file_materialization_changed_during_model_command: beforeFiles !== afterFiles, ancestor_contract_evolution_observed: ancestorEvolved,
    read_schema_evolution: currentEvolved ? 'forward_contract_and_changed_file_materialization; advertised_admin_read_schema_still_required'
      : ancestorEvolved ? 'ancestor_forward_contract_preserved; no_new_read_schema_evolution_claimed; fresh_advertised_schema_still_required'
        : 'not_proven; model_change_alone_is_not_schema_evolution',
    preserved_resource_definitions_sha256: evidenceDigest(right.typed_resource_bindings), preserved_non_model_agent_intent_sha256: sourceRevisionIntent(afterRevision) };
}
const restoreContinuationWorkspace = async workspace => {
  const pin = nativeAdministratorContinuationFixture;
  assert.equal(workspace, join(pin.fixture, 'workspace'));
  assert.equal(await realpath(pin.fixture), pin.fixture);
  const parent = await lstat(pin.fixture);
  assert(parent.isDirectory() && !parent.isSymbolicLink() && parent.uid === process.getuid() && (parent.mode & 0o777) === 0o700, 'The original host fixture parent is not private and owned');
  let created = false;
  try { await lstat(workspace); }
  catch (error) { if (error.code !== 'ENOENT') throw error; await mkdir(workspace, { mode: 0o700 }); created = true; }
  const info = await lstat(workspace);
  assert(info.isDirectory() && !info.isSymbolicLink() && info.uid === process.getuid() && (info.mode & 0o777) === 0o700
    && await realpath(workspace) === workspace && (await readdir(workspace)).length === 0, 'The exact original fixture workspace must be owned, empty and private');
  return { path: workspace, created_empty_after_proven_absence: created, dev: info.dev, ino: info.ino, uid: info.uid, mode: '700',
    evidence_kind: 'host_fixture_preparation_not_Agent_result', original_session_resource_binding_changed: false };
};
export function nativeCompletionContract({ desktopOnly, tools = [], toolChoice }) {
  if (tools.some(tool => tool.function?.name === 'report_completion')) return 'report_completion';
  assert(desktopOnly, 'Non-desktop native fixtures require the actual advertised report_completion contract');
  assert(!tools.some(tool => tool.function?.name === 'update_plan'), 'An explicit plan-control gate cannot be bypassed with a direct answer');
  assert(toolChoice === undefined || toolChoice === 'auto' || toolChoice === 'none', 'An explicit required/specific tool choice cannot be bypassed with a direct answer');
  return 'direct_answer_non_ledger';
}

export function nativeCompletionArguments({ tool, observedCallIds, summary }) {
  const properties = tool.function.parameters.properties;
  const criterion = properties.criteria.items.properties;
  const eligible = new Set(criterion.evidence_call_ids?.items?.enum ?? []);
  const maximum = criterion.evidence_call_ids?.maxItems ?? 8;
  assert(maximum >= 1, 'No eligible completion evidence is advertised');
  const ids = [...observedCallIds].filter(id => eligible.has(id));
  assert(ids.length, 'No actual matching tool observation is eligible for completion');
  const criteria = [];
  for (let index = 0; index < ids.length; index += Math.min(maximum, 8)) {
    const selected = ids.slice(index, index + Math.min(maximum, 8));
    // Exact, per-run call identities belong in the typed evidence field. Their
    // length must never expand this bounded human-readable display label.
    criteria.push({ step: '本阶段已核验的本机工具结果', disposition: 'supported', rationale: '引用当前正式完成合同明确允许的真实调用；先前文件内容核验和进程输出保留在本次独立验收记录中。', evidence_call_ids: selected });
  }
  const result = { summary, criteria };
  for (const key of ['observed_tool_error_count', 'observed_command_failure_count']) if (properties[key]?.const !== undefined) result[key] = properties[key].const;
  return result;
}

// This proof uses current model system instructions and their existing
// canonical digest projection. Durable instruction bodies are intentionally
// omitted by the product; do not create another transcript to recover them.
export function nativeAdministratorInstructionContext({ messages, path, marker }) {
  const contexts = messages.filter(message => ['system', 'developer'].includes(message.role))
    .map(message => typeof message.content === 'string' ? message.content : '')
    .filter(content => content.includes(path) && content.includes(marker) && content.includes('observed as administrator'));
  return { matching_system_context_count: contexts.length,
    sha256: contexts.length === 1 ? createHash('sha256').update(contexts[0]).digest('hex') : undefined,
    bytes: contexts.length === 1 ? Buffer.byteLength(contexts[0]) : undefined };
}
export function nativeAdministratorInstructionProof({ modelContext, events, operationId, root, path, marker, proposalCallId }) {
  assert.equal(modelContext?.matching_system_context_count, 1, 'The actual proposal request system context must contain the privileged fixture instruction layer exactly once');
  assert(/^[a-f0-9]{64}$/.test(modelContext.sha256) && Number.isSafeInteger(modelContext.bytes) && modelContext.bytes > 0, 'Invalid bounded model context observation');
  const digest = modelContext.sha256;
  const notice = `Repository instruction body omitted from durable history (${modelContext.bytes} UTF-8 bytes, sha256:${digest}). Re-read current rules through the authorized workspace tool; this digest grants no authority.`;
  const records = events.filter(event => event.correlation_id === operationId && event.kind === 'runtime/progress-recorded' && event.payload?.storage === 'inline_json')
    .map(event => ({ seq: event.seq, event: event.payload.value?.event }));
  const updated = records.findLast(record => record.event?.event === 'instructions_updated' && record.event.context === notice);
  assert(updated, 'The actual model instruction context must match its canonical InstructionsUpdated digest');
  let proposal;
  if (proposalCallId !== undefined) {
    proposal = records.find(record => record.event?.event === 'tool_call_completed' && record.event.step > 0
      && record.event.call?.call_id === proposalCallId && record.event.call.name === 'start_process'
      && record.event.call.arguments?.execution_identity === 'administrator' && record.event.call.arguments.cwd === root);
    assert(proposal && proposal.seq > updated.seq, 'The privileged instructions must be published before this exact successful start proposal, never inferred from its later model request');
  }
  const observedRead = target => records.findLast(record => record.seq < updated.seq && record.event?.event === 'tool_call_completed' && record.event.step === 0
    && record.event.call?.name === 'read_file' && record.event.call.arguments?.execution_identity === 'administrator' && target(record.event.call.arguments)
    && records.some(done => done.seq > record.seq && done.seq < updated.seq && done.event?.event === 'tool_completed' && done.event.step === 0
      && done.event.result?.call_id === record.event.call.call_id && done.event.result.is_error === false));
  const scope = observedRead(arguments_ => arguments_.format === 'instruction_scope' && arguments_.path === root);
  const text = observedRead(arguments_ => arguments_.path === path && arguments_.missing_ok === true);
  assert(scope && text, 'The same canonical administrator read action must settle both root scope and required AGENTS text before context publication');
  return { instruction_path: path, marker, execution_identity: 'administrator', model_system_context_sha256: digest,
    model_system_context_bytes: modelContext.bytes, instructions_updated_seq: updated.seq, proposal_call_id: proposalCallId, proposal_seq: proposal?.seq,
    scope_call_id: scope.event.call.call_id, scope_seq: scope.seq, text_call_id: text.event.call.call_id, text_seq: text.seq };
}

export async function runNativeRegression({ baseUrl, token, tokenFile, reportDirectory, administrator = false, scenario = 'files-process', template = 'chat.minimal', administratorDirectory, retainServiceForAppExit = false, appPid, nativeDataRoot, signal, protectedUserDirectory = false, desktopBackgroundFocus = false, continueSession, continuationSourceReport, continuationRetryReport }) {
  assert.equal(process.platform, 'darwin', 'This runner requires the actual macOS App host');
  assert.notEqual(process.getuid(), 0, 'Run the driver as the real ordinary login user');
  assert.equal(administratorDirectory, undefined, 'Arbitrary administrator directories are forbidden; use administrator=true / --administrator');
  assert(['files-process', 'system', 'desktop'].includes(scenario), 'Supported scenarios: files-process, system, desktop');
  assert(!administrator || scenario === 'files-process', 'Administrator verification is an independent scenario');
  assert(!retainServiceForAppExit || (scenario === 'system' && !administrator), 'Retained service verification is a separate system scenario');
  assert.equal(typeof desktopBackgroundFocus, 'boolean', '--desktop-background-focus is a boolean test option');
  assert(!desktopBackgroundFocus || (scenario === 'desktop' && !administrator && !retainServiceForAppExit), '--desktop-background-focus requires the independent desktop scenario');
  assert.equal(typeof protectedUserDirectory, 'boolean', '--protected-user-directory is a boolean test option');
  assert(!protectedUserDirectory || (scenario === 'files-process' && !administrator && !retainServiceForAppExit), '--protected-user-directory applies only to ordinary files-process verification');
  assert(retainServiceForAppExit || appPid === undefined, '--app-pid applies only to retained-service verification');
  const continuationRequested = nativeContinuationOptions({ administrator, scenario, template, continueSession, continuationSourceReport, continuationRetryReport, nativeDataRoot, retainServiceForAppExit, desktopBackgroundFocus, protectedUserDirectory });
  assert(retainServiceForAppExit || desktopBackgroundFocus || continuationRequested || nativeDataRoot === undefined, '--native-data-root applies only to retained-service, background-focus or pinned administrator continuation verification');
  assert(['chat.minimal', 'assistant.general'].includes(template), 'Supported default templates: chat.minimal, assistant.general');
  const origin = new URL(baseUrl);
  assert(['127.0.0.1', 'localhost', '[::1]'].includes(origin.hostname), 'Only an explicitly supplied local native App is accepted');
  assert.equal(origin.protocol, 'http:');
  assert(token, 'An owner login JWT is required; the native browser trust secret is not an API bearer token');
  const requestApi = async (method, path, body, targetOrigin = origin) => {
    const response = await fetch(new URL(path, targetOrigin), { method, headers: { Authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(30000) });
    const raw = await response.text(); let value;
    try { value = JSON.parse(raw); } catch { throw new Error(`${path}: HTTP ${response.status}, non-JSON response`); }
    assert(response.ok, `${path}: HTTP ${response.status}: ${JSON.stringify(value)}`); assert.notEqual(value.success, false);
    return value.data === undefined ? value : value.data;
  };
  const readEvents = async id => {
    const events = []; let after = 0;
    for (let page = 0; page < 20; page++) {
      const value = await requestApi('GET', `/api/agent-sessions/${id}/events?after_seq=${after}&limit=500`);
      events.push(...value.events); const next = value.next_cursor.seq;
      if (!value.events.length || next <= after) return events;
      after = next;
    }
    throw new Error('Canonical evidence exceeded bounded pagination');
  };
  let nativeDataset, initialAnnouncement, desktopHostIdentity, authenticatedNativeCaller, continuation;
  if (retainServiceForAppExit || desktopBackgroundFocus || continuationRequested) {
    assert(typeof nativeDataRoot === 'string', '--native-data-root is required for this native App identity check');
    nativeDataset = await validateNativeDataset(nativeDataRoot);
    initialAnnouncement = await readNativeAnnouncement(nativeDataset);
    assert.equal(Number(origin.port), initialAnnouncement.port, 'API origin must match this native dataset announcement');
    assert(!processIsGone(initialAnnouncement.pid), 'The announced native App must be alive before this test phase begins');
    if (retainServiceForAppExit) {
      assert(Number.isInteger(appPid) && appPid > 1 && appPid !== process.pid, '--app-pid must identify the actual running native App');
      assert.equal(initialAnnouncement.pid, appPid, 'The actual initial App PID must match the private dataset announcement');
    }
    if (continuationRequested) {
      assert.equal(tokenFile, join(nativeDataset.parent, 'owner-jwt'), 'Continuation requires the same private dataset owner JWT file');
      const ownerToken = (await readNativeOwnedFixtureBytes(tokenFile, 4096)).toString('utf8').trim();
      assert(ownerToken === token, 'A substituted login token is not permitted');
      assert(!origin.username && !origin.password, 'The native API URL must not contain credentials');
    }
    if (desktopBackgroundFocus || continuationRequested) {
      const response = await fetch(new URL('/api/system/permissions', origin), { headers: { Authorization: `Bearer ${token}` }, signal: AbortSignal.timeout(15000) });
      assert(response.ok, `Authenticated native caller identity request failed (HTTP ${response.status})`);
      const envelope = await response.json(), caller = envelope.data === undefined ? envelope : envelope.data;
      assert.notEqual(envelope.success, false, 'The native App rejected the authenticated caller identity request');
      assert.equal(caller.platform, 'macos'); assert.equal(caller.process_id, initialAnnouncement.pid, 'The actual API caller PID differs from the marked data announcement');
      assert.equal(caller.app_bundle_identifier, 'com.nomifun.desktop', 'Background focus requires the dedicated formally identified acceptance App');
      desktopHostIdentity = { pid: caller.process_id, bundle_identifier: caller.app_bundle_identifier, native_data_root: nativeDataset.root };
      authenticatedNativeCaller = caller;
    }
  }
  if (continuationRequested) {
    const pin = nativeAdministratorContinuationFixture;
    assert.equal((await readNativeOwnedFixtureBytes(join(nativeDataset.parent, '.nomifun-global-os-fixture'), 4096)).toString('utf8'), 'nomifun-global-os-native-acceptance-fixture.v1\n');
    const canonicalSource = await realpath(continuationSourceReport);
    assert.equal(canonicalSource, pin.sourceReport);
    for (const directory of [dirname(canonicalSource), dirname(dirname(canonicalSource))]) {
      const info = await lstat(directory);
      assert(info.isDirectory() && !info.isSymbolicLink() && info.uid === process.getuid() && (info.mode & 0o022) === 0, 'The source evidence parent must be owned and not writable by others');
    }
    const sourceBytes = await readNativeOwnedFixtureBytes(canonicalSource);
    const source = JSON.parse(sourceBytes.toString('utf8'));
    const sourceEvidence = nativeContinuationSource(source, { sessionId: continueSession, template, driverUid: process.getuid(), sha256: createHash('sha256').update(sourceBytes).digest('hex') });
    let retrySource, retryEvidence;
    if (continuationRetryReport !== undefined) {
      assert.equal(await realpath(continuationRetryReport), pin.retryReport);
      const bytes = await readNativeOwnedFixtureBytes(pin.retryReport);
      retrySource = JSON.parse(bytes.toString('utf8'));
      retryEvidence = nativeContinuationRetrySource(retrySource, { sourceEvidence, driverUid: process.getuid(), sha256: createHash('sha256').update(bytes).digest('hex') });
      assert.equal(retrySource.administrator_fixture?.path, join(retrySource.fixture, 'admin-root'));
      assert.equal(await present(retrySource.administrator_fixture.path), false, 'The failed continuation root target must still be absent');
    }
    const admissionOperationId = retryEvidence?.retry_operation_id ?? sourceEvidence.source_operation_id;
    assert.equal(tokenFile, join(nativeDataset.parent, 'owner-jwt'), 'Continuation requires the same private dataset owner JWT file');
    const ownerToken = (await readNativeOwnedFixtureBytes(tokenFile, 4096)).toString('utf8').trim();
    assert(ownerToken === token, 'A substituted login token is not permitted');
    const tokenParts = token.split('.'); assert.equal(tokenParts.length, 3, 'The marked file is not an owner login JWT');
    const claims = JSON.parse(Buffer.from(tokenParts[1], 'base64url').toString('utf8'));
    assert.equal(claims.user_id, sourceEvidence.source_owner_id, 'The login JWT principal differs from the source operation owner');
    assert.equal(origin.hostname.replace(/^\[|\]$/g, ''), initialAnnouncement.host, 'The API host differs from the exact native announcement');
    const uid = await observeProcessUid(initialAnnouncement.pid);
    assert(uid.exit_code === 0 && uid.stdout.trim() === String(process.getuid()) && uid.stderr.trim() === '', 'The formal App must run as this real login user');
    const before = await requestApi('GET', `/api/agent-sessions/${continueSession}`);
    assert.equal(before.session.agent_session_id, continueSession); assert.deepEqual(before.session.owner_ref, { principal_kind: 'user', principal_id: claims.user_id });
    assert(!before.session.remote_binding_provenance && !before.session.parent_session_id, 'Remote or derived attempt Sessions are not continuation targets');
    const execution = await requestApi('GET', `/api/agent-sessions/${continueSession}/execution`);
    const settlement = nativeContinuationSettlement({ execution, head: before.head, operationId: admissionOperationId, sessionId: continueSession });
    const selection = await requestApi('GET', `/api/agent-sessions/${continueSession}/capability-selection`);
    assert.equal(selection.editable, true, 'An Attempt/automation/frozen Session cannot be continued');
    assert.equal(selection.binding_version, before.session.agent_binding.binding_version);
    const projection = await requestApi('GET', `/api/agent-sessions/${continueSession}/projection`);
    assert.equal(projection.session_purpose, 'conversation'); assert.equal(projection.extra.workspace, source.workspace);
    assert.deepEqual(projection.agent_snapshot.canonical_binding, before.session.agent_binding);
    nativeContinuationCaller({ origin: origin.href, announcement: initialAnnouncement, caller: authenticatedNativeCaller, uidObservation: uid, driverUid: process.getuid() });
    nativeContinuationSession({ observation: before, sourceEvidence, selection, projection });
    const events = await readEvents(continueSession);
    const sourceEvents = events.filter(event => event.correlation_id === sourceEvidence.source_operation_id);
    assert.equal(evidenceDigest(sourceEvents), sourceEvidence.source_events_sha256, 'The original failed operation changed since its report');
    if (retryEvidence) {
      assert.equal(evidenceDigest(events.filter(event => event.correlation_id === retryEvidence.retry_operation_id)), retryEvidence.retry_events_sha256,
        'The zero-user-dispatch failed continuation changed in canonical history');
      const current = before.session.agent_binding;
      assert.deepEqual({ preset_revision_ref: current.preset_revision_ref, resolved_snapshot_ref: current.resolved_snapshot_ref, binding_version: current.binding_version },
        retrySource.continuation.evolution.after_binding_ref, 'The existing prepared binding differs from the exact failed continuation');
      for (const transition of retrySource.continuation.evolution.canonical_contract_transitions) {
        const actual = events.find(event => event.event_id === transition.event_id && event.seq === transition.seq);
        assert(actual && evidenceDigest(actual) === transition.event_sha256, 'The ancestor ContextOnly contract transition changed');
      }
    }
    const opening = events.find(event => event.seq === 1 && event.kind === 'session/opening');
    assert(opening && opening.agent_session_id === continueSession);
    const sourceBinding = eventPayload(opening).agent_binding;
    const started = sourceEvents.find(event => event.kind === 'turn/started');
    assert.equal(eventPayload(started).source_message_id, sourceEvidence.source_message_id);
    assert.deepEqual(eventPayload(started).admission.resolved_snapshot_ref, sourceBinding.resolved_snapshot_ref);
    assert.deepEqual(before.session.agent_binding.typed_resource_bindings, sourceBinding.typed_resource_bindings);
    for (const resource of sourceBinding.typed_resource_bindings.filter(resource => ['workspace', 'process_session'].includes(resource.resource_kind)))
      assert.equal(resource.typed_parameters?.workspace_root, source.workspace);
    const ref = before.session.agent_binding.preset_revision_ref, originalRef = sourceBinding.preset_revision_ref;
    const beforeRevision = await requestApi('GET', `/api/agent-presets/${ref.preset_id}/revisions/${ref.revision}`);
    const sourceRevision = await requestApi('GET', `/api/agent-presets/${originalRef.preset_id}/revisions/${originalRef.revision}`);
    assert.equal(sourceRevisionIntent(sourceRevision), sourceRevisionIntent(beforeRevision), 'The original Agent was replaced before continuation');
    const announcementAfter = await readNativeAnnouncement(nativeDataset);
    assert.deepEqual(announcementAfter, initialAnnouncement, 'The announced App changed during read-only continuation admission');
    assert(!processIsGone(initialAnnouncement.pid));
    assert.equal(await realpath(reportDirectory), dirname(canonicalSource), 'Continuation reports must stay in the pinned acceptance evidence directory');
    continuation = { source, sourceEvidence, retrySource, retryEvidence, admissionOperationId, sourceBinding, sourceRevision, beforeRevision, before, projection,
      report: { ...sourceEvidence, source_report: canonicalSource, original_session_id: continueSession, owner_jwt_reused: true,
        native_app_identity: desktopHostIdentity, native_app_uid_observation: uid, admission_operation_id: admissionOperationId, admission_settlement: settlement,
        source_settlement: retryEvidence ? nativeContinuationSettlement({ execution: source.failure_settlement.execution, head: source.failure_settlement.head,
          operationId: sourceEvidence.source_operation_id, sessionId: continueSession }) : settlement,
        retry: retryEvidence ? { ...retryEvidence, retry_report: pin.retryReport, latest_settlement: settlement,
          ancestor_read_tool_schema: retrySource.continuation.read_tool_schema, previous_administrator_root_still_absent: true } : undefined,
        before_binding_version: before.session.agent_binding.binding_version, before_snapshot_ref: before.session.agent_binding.resolved_snapshot_ref,
        before_canonical_head_seq: before.head.last_seq, source_original_revision_sha256: evidenceDigest(sourceRevision), api_mutations: [] } };
  }
  const interruption = new AbortController();
  const interrupted = () => interruption.abort(new Error('Native acceptance driver was interrupted; retain any service target until product cleanup is proven'));
  const forwardedAbort = () => interruption.abort(signal.reason ?? new Error('Native acceptance driver was cancelled'));
  if (signal?.aborted) forwardedAbort(); else signal?.addEventListener('abort', forwardedAbort, { once: true });
  process.once('SIGINT', interrupted); process.once('SIGTERM', interrupted);
  const checkInterruption = () => { if (interruption.signal.aborted) throw interruption.signal.reason; };
  await mkdir(reportDirectory, { recursive: true });
  // The host allocates the only writable test namespace. Never accept a caller's
  // daily root directory, and never reuse a fixture after a failed attempt.
  const allocated = `/private/tmp/nomifun-global-os-product-${randomUUID()}`;
  await mkdir(allocated, { mode: 0o700 });
  const fixture = await realpath(allocated);
  assert.equal(fixture, allocated);
  const workspace = continuation ? continuation.source.workspace : join(fixture, 'workspace');
  let protectedParent;
  if (protectedUserDirectory) {
    const requestedParent = join(homedir(), 'Documents');
    try {
      const canonicalParent = await realpath(requestedParent);
      const info = await lstat(canonicalParent);
      assert(info.isDirectory() && !info.isSymbolicLink(), 'The actual Documents target must already exist as a directory');
      protectedParent = { requested_parent: requestedParent, canonical_parent: canonicalParent, environment: { available: true }, host_created_parent: false, host_created_test_directory: false };
    } catch (error) {
      protectedParent = { requested_parent: requestedParent, environment: { available: false, reason: ['ENOENT', 'ENOTDIR'].includes(error.code) ? 'documents_directory_missing' : ['EACCES', 'EPERM'].includes(error.code) ? 'documents_metadata_denied' : 'documents_directory_unavailable', os_code: error.code, detail: error.message }, host_created_parent: false, host_created_test_directory: false };
    }
  }
  // For protected-user verification the host allocates only an absent name.
  // The Agent file owner creates and deletes this unique directory; the host
  // never prepares Documents, creates its test root, or writes a daily file.
  const outside = protectedParent ? join(protectedParent.canonical_parent ?? protectedParent.requested_parent, `nomifun-global-os-product-${randomUUID()}`) : join(fixture, 'outside');
  const administratorRoot = join(fixture, 'admin-root');
  const marker = join(administratorRoot, `marker-${randomUUID()}.txt`);
  const markerValue = `NOMIFUN_ADMIN_NATIVE_OK_${randomUUID()}`;
  const administratorInstructions = join(administratorRoot, 'AGENTS.md');
  const administratorInstructionMarker = `NOMIFUN_ADMIN_INSTRUCTION_${randomUUID()}`;
  const administratorInstructionContent = `NomiFun isolated administrator acceptance fixture.\nFixture marker: ${administratorInstructionMarker}\nApply only to this unique fixture. Preserve the requested verification and cleanup sequence.\n`;
  const administratorInstructionEvidence = { path: administratorInstructions, marker: administratorInstructionMarker, expected_uid: 0, expected_mode: '600', observations: [], proposal_context_observations: [], deferred_call_ids: [] };
  if (continuation) continuation.report.workspace_fixture_preparation = await restoreContinuationWorkspace(workspace);
  else await mkdir(workspace, { mode: 0o700 });
  assert(!contained(workspace, outside));
  assert.equal(await present(administratorRoot), false);
  const initial = 'NomiFun global native before\n';
  const changed = 'NomiFun global native after 中文\n';
  const file = join(outside, 'sample.txt');
  const checks = [], observations = new Map(), completedSteps = new Set(), callSteps = new Map(), attempts = new Map(), discoveryAttempts = new Map(), rawResults = new Map(), discoveries = new Map(), completions = new Map(), activeCompletionIds = new Set(), activeTurnCallIds = new Set();
  const runId = randomUUID();
  let processId, nativePid, providerFailure, sessionId = continueSession, accepted, pendingAdmissionOperationId, cursor = 0, sequence = 0, completionAccepted = false, readinessText = '', activeTurnRole = 'main', completionAttempts = 0, currentCompletionMode = 'report_completion', directAnswerCount = 0;
  const administratorStartProposalContexts = new Map();
  let providerRequestSequence = 0;
  let administratorProcessId, administratorPid, administratorCursor = 0, administratorOutput = '', administratorExitVerified = false;
  const administratorProcessEvidence = { expected_uid: 0, expected_cwd: administratorRoot, expected_login_home: homedir(), identity_and_exit_observations: [] };
  const verifyAdministratorPidGone = async phase => {
    const observed = await observeProcessUid(administratorPid);
    administratorProcessEvidence.identity_and_exit_observations.push({ phase, ...observed });
    assert(observed.exit_code === 1 && observed.stdout.trim() === '' && observed.stderr.trim() === '', 'The exact administrator process PID must be absent after proven helper cleanup');
    administratorExitVerified = true;
  };
  let systemScenario, desktopScenario;
  if (scenario === 'system') {
    const { allocateLoopbackPort, createMacSystemScenario } = await import('./global-os-system-cases.mjs');
    systemScenario = createMacSystemScenario({ fixture, workspace, uid: process.getuid(), loopbackPort: await allocateLoopbackPort(), preserveServiceForHostExit: retainServiceForAppExit });
  }
  if (scenario === 'desktop') {
    const { createMacDesktopScenario } = await import('./global-os-desktop-cases.mjs');
    desktopScenario = createMacDesktopScenario({ fixture, workspace, uid: process.getuid(), desktopBackgroundFocus, desktopHostIdentity });
  }
  const acceptanceScenario = systemScenario ?? desktopScenario;
  const safeFailure = error => acceptanceScenario?.sanitizeFailure?.(error) ?? { name: error?.name ?? 'Error', message: error.message, stack: error.stack };
  const safeReport = value => acceptanceScenario?.sanitizeReport?.(value) ?? value;
  const steps = acceptanceScenario?.steps ?? (administrator ? [
    { id: 'admin-fixture', name: 'exec_command', args: { command: '/bin/sh', args: ['-c', 'set -eu; umask 077; /bin/mkdir -m 700 "$1"; [ ! -e "$2" ] && [ ! -L "$2" ]; set -C; printf "%s" "$3" > "$2"; /usr/bin/stat -f "%u:%Lp" "$1" "$2"', 'nomifun-admin-prepare', administratorRoot, administratorInstructions, administratorInstructionContent], execution_identity: 'administrator' }, verify: async value => {
      requireExited(value); assert.equal(value.output.text, '0:700\n0:600\n');
      const info = await lstat(administratorRoot);
      assert(info.isDirectory() && !info.isSymbolicLink()); assert.equal(info.uid, 0); assert.equal(info.mode & 0o777, 0o700);
      const deniedMarker = join(administratorRoot, `ordinary-must-fail-${randomUUID()}`);
      await assert.rejects(writeFile(deniedMarker, 'must not be written', { flag: 'wx', mode: 0o600 }), error => ['EACCES', 'EPERM'].includes(error.code));
      checks.push({ id: 'ordinary-user-denied', status: 'passed', driver_uid: process.getuid(), path: deniedMarker, fixture_uid: info.uid, fixture_mode: '700' });
    } },
    { id: 'admin-write-read', name: 'exec_command', args: { command: '/bin/sh', args: ['-c', 'set -eu; umask 077; [ ! -e "$2" ] && [ ! -L "$2" ]; set -C; printf "%s\\n" "$1" > "$2"; /usr/bin/id -u; /bin/cat "$2"; /usr/bin/stat -f "%u:%Lp" "$2"', 'nomifun-admin-marker', markerValue, marker], execution_identity: 'administrator' }, verify: value => {
      requireExited(value); assert.equal(value.output.text, `0\n${markerValue}\n0:600\n`);
    } },
    { id: 'admin-process-start', name: 'start_process', repeatable: true, maxAttempts: 3, allowInstructionRefresh: true, args: { command: '/bin/sh', args: ['-c', 'set -eu; /usr/bin/id -u; /bin/pwd; printf "%s\\n" "$HOME"; printf "%s\\n" "NOMIFUN_ADMIN_WAITING"; i=0; while [ "$i" -lt 60 ]; do /bin/sleep 1; i=$((i + 1)); done', 'nomifun-admin-wait'], execution_identity: 'administrator', cwd: administratorRoot, wait_ms: 0, timeout_ms: 60000 }, verify: async (value, context) => {
      assert.equal(value.state, 'running'); administratorProcessId = value.process_id; administratorPid = value.pid;
      assert(typeof administratorProcessId === 'string' && administratorProcessId.length > 0); assert(Number.isSafeInteger(administratorPid) && administratorPid > 1);
      administratorCursor = value.output.next_cursor; administratorOutput = value.output.text;
      Object.assign(administratorProcessEvidence, { process_id: administratorProcessId, pid: administratorPid, execution_identity: 'administrator', timeout_ms: 60000 });
      // Retain the actual owned PID even if the independent context proof fails.
      administratorInstructionEvidence.observations.push({ phase: 'successful_start_proposal_context', ...await verifyAdministratorInstructions(administratorStartProposalContexts.get(context.callId), context.callId) });
    } },
    { id: 'admin-process-poll-running', name: 'poll_process', repeatable: true, maxAttempts: 12, args: () => ({ process_id: administratorProcessId, cursor: administratorCursor, wait_ms: 1000 }), verify: async value => {
      assert.equal(value.state, 'running'); administratorCursor = value.output.next_cursor; administratorOutput += value.output.text;
      const expected = `0\n${administratorRoot}\n${homedir()}\nNOMIFUN_ADMIN_WAITING\n`;
      if (!administratorOutput.includes('NOMIFUN_ADMIN_WAITING\n')) return false;
      assert.equal(administratorOutput, expected, 'The administrator child must report uid 0, its actual root fixture cwd, and the real login HOME');
      const observed = await observeProcessUid(administratorPid);
      administratorProcessEvidence.identity_and_exit_observations.push({ phase: 'running', ...observed });
      assert.equal(observed.exit_code, 0); assert.equal(observed.stdout.trim(), '0'); assert.equal(observed.stderr.trim(), '');
      administratorProcessEvidence.actual_output_verified = true;
    } },
    { id: 'admin-process-cancel', name: 'cancel_process', args: () => ({ process_id: administratorProcessId }), verify: value => {
      assert.equal(value.state, 'cancelled'); assert.equal(value.cleanup.reaped, true); administratorCursor = value.output.next_cursor;
      administratorProcessEvidence.cancel_receipt = { state: value.state, cleanup: value.cleanup };
    } },
    { id: 'admin-process-poll-exit', name: 'poll_process', allowCancelledObservation: true, args: () => ({ process_id: administratorProcessId, cursor: administratorCursor, wait_ms: 0 }), verify: async value => {
      assert.equal(value.state, 'cancelled'); assert.equal(value.cleanup.reaped, true);
      administratorProcessEvidence.terminal_poll_receipt = { state: value.state, cleanup: value.cleanup };
      // Ordinary-user signal 0 may return EPERM for a live root PID. Use the
      // native read-only exact-PID query instead; no host signal is sent.
      await verifyAdministratorPidGone('terminal_poll');
    } },
    { id: 'admin-clean', name: 'exec_command', args: { command: '/bin/sh', args: ['-c', 'set -eu; /bin/rm -- "$1" "$2"; /bin/rmdir -- "$3"', 'nomifun-admin-clean', marker, administratorInstructions, administratorRoot], execution_identity: 'administrator' }, verify: async value => {
      assert(administratorExitVerified, 'Root fixture cleanup must follow actual root-process exit verification');
      requireExited(value); assert.equal(await present(administratorRoot), false);
    } },
  ] : [
    { id: 'discover-file-instructions', name: 'read_file', args: { path: file, format: 'instruction_scope' }, verify: value => { assert.equal(value.observation_kind, 'instruction_scope'); assert.equal(value.complete, true); } },
    { id: 'create', name: 'write_file', args: { path: file, content: initial }, verify: async () => assert.equal(await readFile(file, 'utf8'), initial) },
    { id: 'read-before', name: 'read_file', args: { path: file }, verify: value => assert(JSON.stringify(value).includes(initial.trimEnd())) },
    { id: 'patch', name: 'apply_patch', args: { files: [{ path: file, expected_source: { kind: 'existing', sha256: createHash('sha256').update(initial).digest('hex') }, hunks: [{ old_start: 1, old_lines: 1, new_start: 1, new_lines: 1, lines: [{ kind: 'remove', text: initial.trimEnd() }, { kind: 'add', text: changed.trimEnd() }] }] }] }, verify: async () => assert.equal(await readFile(file, 'utf8'), changed) },
    { id: 'read-after', name: 'read_file', args: { path: file }, verify: value => assert(JSON.stringify(value).includes(changed.trimEnd())) },
    { id: 'search', name: 'search_files', args: { path: outside, query: changed.trimEnd() }, verify: value => assert(JSON.stringify(value).includes('sample.txt')) },
    { id: 'start', name: 'start_process', args: { command: '/bin/sh', args: ['-c', 'pwd; /bin/cat "$1"; printf "WAITING\\n"; while :; do /bin/sleep 1; done', 'nomifun-global-fixture', 'sample.txt'], cwd: outside, wait_ms: 0, timeout_ms: 60000 }, verify: value => {
      assert.equal(value.state, 'running'); processId = value.process_id; nativePid = value.pid; cursor = value.output.next_cursor; readinessText = value.output.text;
      assert(processId); assert(Number.isInteger(nativePid)); process.kill(nativePid, 0);
    } },
    { id: 'poll-running', name: 'poll_process', repeatable: true, maxAttempts: 12, args: () => ({ process_id: processId, cursor, wait_ms: 1000 }), verify: value => {
      assert.equal(value.state, 'running'); readinessText += value.output.text;
      cursor = value.output.next_cursor; process.kill(nativePid, 0);
      return readinessText.includes(outside) && readinessText.includes(changed.trimEnd()) && readinessText.includes('WAITING');
    } },
    { id: 'cancel', name: 'cancel_process', args: () => ({ process_id: processId }), verify: value => { assert.equal(value.state, 'cancelled'); assert.equal(value.cleanup.reaped, true); cursor = value.output.next_cursor; } },
    { id: 'poll-exit', name: 'poll_process', allowCancelledObservation: true, args: () => ({ process_id: processId, cursor, wait_ms: 0 }), verify: value => {
      assert.equal(value.state, 'cancelled'); assert.equal(value.cleanup.reaped, true); assert.throws(() => process.kill(nativePid, 0), { code: 'ESRCH' });
    } },
    { id: 'delete', name: 'delete_path', args: { path: outside }, verify: async () => assert.equal(await present(outside), false) },
  ]);
  let activeSteps = steps;
  const api = async (method, path, body, targetOrigin = origin) => {
    if (continuation) {
      nativeContinuationMutation(method, path, continueSession);
      if (method !== 'GET') continuation.report.api_mutations.push({ method, path });
    }
    return requestApi(method, path, body, targetOrigin);
  };
  const currentCompletionSummary = () => {
    const stepObservation = stepId => [...observations].findLast(([callId]) => callSteps.get(callId)?.id === stepId)?.[1];
    let systemSummary;
    if (systemScenario) {
      const target = systemScenario.resources.target;
      const preparation = `本阶段只完成本次隔离配置的文件写入、patch和读回，以及唯一用户工具 ${systemScenario.resources.userToolName} 的 install -m755、裸命令 literal argv/task PATH 核验和卸载；独立 metadata/内容核验通过。配置及本次 source 目录按计划保留，尚未声称服务或网络阶段完成。`;
      const service = `本阶段只对本次随机 CFPreferences domain 原生 write/read/delete并确认恢复不存在，再注册启动唯一用户 launchd 服务 ${target}，实际原生状态和同 PID 标记核验通过。该服务按本次明确意图保留给后续阶段；尚未执行本机网络或最终卸载。`;
      const network = `本阶段通过同一 Turn 的受管进程在 127.0.0.1:${systemScenario.resources.loopbackPort} 建立唯一 nc 监听，独立 running poll核验监听，实际 curl 响应为 ${stepObservation('system-network-request')?.output?.text?.trim()}；请求输出、取消及末次 poll 的进程清理均核验通过。先前 launchd 服务仍按计划保留；最终配置恢复和服务卸载待下一阶段。`;
      const finalization = `本阶段已通过文件工具恢复并读回本次配置。${retainServiceForAppExit ? `按明确要求保留唯一 launchd 服务 ${target} 和本次文件；App 退出后的存续、重启及最终清理仍待独立验证。` : `已通过原生路径停止并卸载唯一服务 ${target}，确认原生状态不存在和已观察 PID 退出，再删除本次测试文件；本次所有分阶段验收事实分别记录，此阶段本身不冒充 App 退出存续验证。`}`;
      systemSummary = ({ 'system-install-config': preparation, 'system-service': service, 'system-network': network, 'system-finalize': finalization })[activeTurnRole]
        ?? `本次隔离系统操作的真实结果已记录。唯一服务为 ${target}；只有当前阶段实际核验的调用由本次完成回执支持。`;
    }
    if (desktopScenario) return activeTurnRole === 'main' ? desktopScenario.completionSummary() : `原桌面验收失败仍保留。本次新 Turn 只通过现役 Computer 工具清理先前由本夹具新启动的 Calculator PID ${desktopScenario.resources.pid}；退出确认=${desktopScenario.resources.quit_confirmed}，不操作任何既有应用、用户文件或剪贴板。`;
    if (activeTurnRole === 'retained-cleanup') return `此前主阶段按要求保留了本次服务；宿主已独立确认原 App PID ${appPid} 退出、同一服务真实存续，再以原 owner 登录身份在同数据根的新 App 继续同一 Session。本轮原生停止唯一服务 ${systemScenario.resources.target}，确认已观察进程退出和服务不存在，并删除 ${systemScenario.resources.root}。具体退出和重启证据保留在验收记录中。`;
    if (activeTurnRole === 'cleanup') return `原验收失败保留。本次新 Turn 只清理本次唯一服务 ${systemScenario.resources.target}，确认原生状态不存在并删除 ${systemScenario.resources.root}；恢复清理不把首次失败改写为通过。`;
    if (systemScenario) return systemSummary;
    if (administrator) return `root:0600 的 ${administratorInstructions} 含本次随机指令标记；管理员 instruction_scope/AGENTS read 与 canonical InstructionsUpdated 摘要已匹配实际模型 system context，指令变化后使用新调用身份。隔离夹具 ${administratorRoot} 由系统执行创建为 root:0700；普通用户写入实际被 EACCES/EPERM 拒绝。系统 uid=0 写读标记 ${marker}，内容 ${markerValue}、权限 root:0600。随后同系统路径启动实际等待进程 PID ${administratorPid}，独立poll核对uid0、cwd=${administratorRoot}、登录用户HOME=${homedir()}，宿主只读ps证明UID0；XPC取消和terminal poll为cancelled、cleanup.reaped=true，独立ps确认确切PID不存在后才删除标记和root目录。本地脚本供应器不代表外网模型推理验收。`;
    return `文件工具在工作区外 ${file} 创建并读取文本，再patch为 ${changed.trimEnd()}，搜索返回sample.txt。等待进程实际cwd=${outside}，输出修改后文本和WAITING；独立poll观察running，再取消为cancelled、cleanup.reaped=true，末次poll及独立PID核验确认退出，${outside}已删除。此为本机脚本供应器驱动的原生产品路径实测。`;
  };
  const completionArguments = tool => nativeCompletionArguments({ tool, observedCallIds: observations.keys(), summary: currentCompletionSummary() });
  const server = createServer(async (request, response) => {
    try {
      assert.equal(request.method, 'POST'); assert.equal(request.url, '/v1/chat/completions');
      const buffers = []; for await (const bytes of request) buffers.push(bytes);
      const body = JSON.parse(Buffer.concat(buffers).toString('utf8'));
      const messages = body.messages;
      providerRequestSequence++;
      for (const message of messages.filter(message => message.role === 'tool')) {
        const id = message.tool_call_id, raw = toolContent(message);
        const disposition = nativeToolReceiptDisposition({ id, runId, known: callSteps.has(id) || discoveries.has(id) || completions.has(id), active: activeTurnCallIds.has(id) });
        if (disposition === 'historical') continue;
        if (rawResults.has(id)) continue;
        rawResults.set(id, raw); // Preserve actual error text before parsing or asserting.
        // A prior Turn can first expose a cancelled/unobserved proposal while
        // reading history. Its recorded result is data for that prior Turn,
        // never a new failure or verification callback for this Turn's steps.
        if (disposition === 'prior_turn') {
          if (completions.has(id)) completions.set(id, raw);
          continue;
        }
        const step = callSteps.get(id);
        if (step) {
          try {
          if (administrator && step.allowInstructionRefresh === true && raw.startsWith('[tool error]')
              && raw.includes('Requested calls deferred; earlier calls may already have run. Repository instructions changed or a new directory scope was discovered. Read the current scoped instructions, reconsider the calls, and submit new call identities.')) {
            assert.equal(step.id, 'admin-process-start');
            assert(!administratorProcessId && !administratorPid, 'A preflight deferral cannot follow a native process start');
            const proof = await verifyAdministratorInstructions(nativeAdministratorInstructionContext({ messages, path: administratorInstructions, marker: administratorInstructionMarker }));
            administratorInstructionEvidence.observations.push({ phase: 'preflight_deferred_next_model_request', ...proof });
            administratorInstructionEvidence.deferred_call_ids.push(id);
            checks.push({ id, step: step.id, status: 'deferred_for_fresh_instruction_context', instructions_updated_seq: proof.instructions_updated_seq });
            continue; // Keep this real refusal; the next bounded attempt gets a new call identity.
          }
          const value = toolValue(raw);
          // A read-only poll of an intentionally cancelled process reports
          // success=false (it did not exit normally). Its actual cancelled
          // terminal and reap receipt are the expected observation here.
          if (value.success === false) {
            assert(step.allowCancelledObservation === true && value.state === 'cancelled' && value.cleanup?.reaped === true,
              `${id}: ${raw}`);
          }
          const complete = await step.verify(value, { messages, callId: id, toolMessage: message }); observations.set(id, value);
          assert(complete !== false || step.repeatable, `${step.id} has no bounded repeat contract`);
          if (complete !== false) completedSteps.add(step.id);
          checks.push({ id, step: step.id, status: complete === false ? 'waiting_for_remaining_output' : 'passed' });
          } catch (error) {
            if (desktopScenario && error && typeof error === 'object' && /^desktop-[a-z0-9-]+$/.test(step.id)) error.safe_step = step.id;
            throw error;
          }
        } else if (discoveries.has(id)) {
          const found = toolValue(raw); assert(Array.isArray(found), `ToolSearch failed: ${raw}`);
          assert(found.some(item => item.name === discoveries.get(id)), `ToolSearch did not expose the requested default tool: ${raw}`);
          checks.push({ id, status: 'passed', discovered: discoveries.get(id) });
        } else if (completions.has(id)) {
          // A prior Turn's final completion receipt can first become visible
          // when this new Turn reads canonical history. It remains evidence of
          // that earlier account, never completion authority for the new Turn.
          if (!activeCompletionIds.has(id)) { completions.set(id, raw); continue; }
          assert(!raw.startsWith('[tool error]'), `Completion was rejected: ${raw}`);
          if (raw.startsWith('Completion account recorded,')) completionAccepted = true;
          else assert(raw.startsWith('Candidate account recorded,'), `Unexpected completion receipt: ${raw}`);
          completions.set(id, raw);
        }
      }
      const next = activeSteps.find(step => !completedSteps.has(step.id));
      let name, id, args, directAnswer;
      if (continuation && body.tools?.some(tool => tool.function?.name === 'read_file')) {
        const observed = nativeAdministratorReadSchema(body.tools);
        if (continuation.report.read_tool_schema) assert.equal(observed.schema_sha256, continuation.report.read_tool_schema.schema_sha256, 'The frozen read schema changed during this Turn');
        continuation.report.read_tool_schema ??= { ...observed, provider_request_sequence: providerRequestSequence,
          resolved_snapshot_ref: continuation.report.evolution.after_binding_ref.resolved_snapshot_ref };
      }
      if (continuation && !continuation.report.read_tool_schema) {
        const key = 'continuation-read-schema'; const attempt = (discoveryAttempts.get(key) ?? 0) + 1; discoveryAttempts.set(key, attempt);
        assert(attempt <= 2 && body.tools?.some(tool => tool.function?.name === 'ToolSearch'), 'The continuation cannot observe an actual administrator-capable read_file schema');
        name = 'ToolSearch'; id = nativeRunCallId(runId, `continuation-read-schema-${attempt}`); args = { query: 'read_file' }; discoveries.set(id, 'read_file');
      } else if (next) {
        if (body.tools?.some(tool => tool.function?.name === next.name)) {
          name = next.name;
          const attempt = (attempts.get(next.id) ?? 0) + 1; attempts.set(next.id, attempt);
          assert(attempt <= (next.maxAttempts ?? 1), `${next.id} did not establish its expected native result within the bounded attempts`);
          id = nativeRunCallId(runId, `${activeTurnRole === 'main' ? '' : `${activeTurnRole}-`}${next.id}${next.repeatable ? `-${attempt}` : ''}`);
          args = typeof next.args === 'function' ? next.args() : next.args; callSteps.set(id, next);
          if (administrator && next.id === 'admin-process-start') {
            // Observe the request that proposes this exact call. Keep hashes
            // and counts only; a later response cannot supply missing context.
            const observed = nativeAdministratorInstructionContext({ messages, path: administratorInstructions, marker: administratorInstructionMarker });
            administratorStartProposalContexts.set(id, observed);
            administratorInstructionEvidence.proposal_context_observations.push({ call_id: id, provider_request_sequence: providerRequestSequence, ...observed });
          }
        } else {
          assert(body.tools?.some(tool => tool.function?.name === 'ToolSearch'), `Default ${template} has neither ${next.name} nor ToolSearch`);
          const discoveryKey = `${activeTurnRole}:${next.id}`;
          const discoveryAttempt = (discoveryAttempts.get(discoveryKey) ?? 0) + 1; discoveryAttempts.set(discoveryKey, discoveryAttempt);
          assert(discoveryAttempt <= 2, `${next.name} remained unavailable after actual ToolSearch`);
          name = 'ToolSearch'; id = nativeRunCallId(runId, `discover-${next.id}-${++sequence}`); args = { query: next.name }; discoveries.set(id, next.name);
        }
      } else if (!completionAccepted) {
        currentCompletionMode = nativeCompletionContract({ desktopOnly: Boolean(desktopScenario), tools: body.tools, toolChoice: body.tool_choice });
        if (currentCompletionMode === 'direct_answer_non_ledger') {
          assert(++directAnswerCount <= 2, 'The non-ledger direct answer did not reach its normal canonical terminal');
          directAnswer = currentCompletionSummary();
        } else {
          const tool = body.tools.find(tool => tool.function?.name === 'report_completion');
          assert(++completionAttempts <= 4, 'Completion review did not converge within four accounts');
          name = 'report_completion'; id = nativeRunCallId(runId, `completion-${++sequence}`); args = completionArguments(tool); completions.set(id, null); activeCompletionIds.add(id);
        }
      }
      if (name) activeTurnCallIds.add(id);
      const delta = name ? { tool_calls: [{ index: 0, id, type: 'function', function: { name, arguments: JSON.stringify(args) } }] } : { content: directAnswer ?? currentCompletionSummary() };
      response.writeHead(200, { 'content-type': 'text/event-stream' });
      response.end(`data: ${JSON.stringify({ id: 'global-native', choices: [{ index: 0, delta, finish_reason: null }] })}\n\ndata: ${JSON.stringify({ id: 'global-native', choices: [{ index: 0, delta: {}, finish_reason: name ? 'tool_calls' : 'stop' }] })}\n\ndata: [DONE]\n\n`);
    } catch (error) {
      providerFailure ??= error;
      response.writeHead(500, { 'content-type': 'application/json' }); response.end(JSON.stringify({ error: { message: safeFailure(error).message } }));
    }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const providerUrl = `http://127.0.0.1:${server.address().port}/v1`;
  const report = { schema: 'nomifun.global-os-native-regression.v2', run_id: runId, session_id: continueSession, started_at: new Date().toISOString(), host: origin.origin, fixture, workspace, outside, template, scenario: administrator ? 'administrator' : scenario,
    evidence_scope: continuation ? 'Real native App / same originally selected Agent and Session / same private owner JWT / public model command and canonical contract transition / new isolated administrator fixture. No new Agent, grants or resources. Original failure remains failed. Local scripted provider is not live-model reasoning evidence.' : 'Real native App / unmodified default template / owner login JWT / local scripted provider / real native owners. App signing identity and GUI review require separate driver evidence. No live-model reasoning claim.', continuation: continuation?.report, administrator, administrator_fixture: administrator ? { path: administratorRoot, marker } : undefined, administrator_instructions: administrator ? administratorInstructionEvidence : undefined, administrator_process: administrator ? administratorProcessEvidence : undefined, protected_user_directory: protectedParent ? { ...protectedParent, allocated_test_directory: outside } : undefined, checks, status: 'running', system_resources: systemScenario?.resources, desktop_resources: desktopScenario?.resources, desktop_mode: desktopScenario ? (desktopBackgroundFocus ? 'background_focus' : 'full') : undefined, desktop_host_identity: desktopHostIdentity, pending_acceptance: acceptanceScenario?.pendingAcceptance, native_system_evidence: systemScenario?.nativeEvidence, retain_service_for_app_exit: retainServiceForAppExit, native_dataset: nativeDataset, initial_native_announcement: initialAnnouncement, phase_checkpoints: [] };
  const reportPath = join(reportDirectory, `${administrator ? 'administrator' : scenario}${continuation ? '-continuation' : ''}-${template}-${Date.now()}-${randomUUID()}.json`);
  const phaseCheckpoint = async (phase, extra = {}) => {
    const checkpointPath = reportPath.replace(/\.json$/, `.${phase}.json`);
    report.status = phase; Object.assign(report, extra);
    const checkpoint = { phase, path: checkpointPath, at: new Date().toISOString() };
    report.phase_checkpoints.push(checkpoint);
    await writeFile(checkpointPath, `${JSON.stringify(safeReport({ ...report, observations: Object.fromEntries(observations), raw_tool_results: Object.fromEntries(rawResults), completion_receipts: Object.fromEntries(completions) }), null, 2)}\n`, { flag: 'wx', mode: 0o600 });
    console.log(JSON.stringify({ phase, checkpoint: checkpointPath, session_id: sessionId, service_target: systemScenario?.resources.target, ...extra }));
  };
  const waitForPhase = async (phase, deadline, probe) => {
    let lastProgress = Date.now();
    while (Date.now() < deadline) {
      checkInterruption();
      const result = await probe();
      checkInterruption();
      if (result) return result;
      if (Date.now() - lastProgress >= 30000) {
        console.log(JSON.stringify({ phase, status: 'waiting_for_root_controlled_App_lifecycle', deadline: new Date(deadline).toISOString(), session_id: sessionId, service_target: systemScenario.resources.target }));
        lastProgress = Date.now();
      }
      await wait(250);
    }
    throw new Error(`Ten-minute retained-service lifecycle deadline elapsed during ${phase}; preserve the exact service target until product cleanup succeeds`);
  };
  const sessionEvents = async () => {
    const events = []; let after = 0;
    for (let page = 0; page < 20; page++) {
      const value = await api('GET', `/api/agent-sessions/${sessionId}/events?after_seq=${after}&limit=500`);
      events.push(...value.events); const next = value.next_cursor.seq;
      if (!value.events.length || next <= after) break;
      after = next;
      if (page === 19) throw new Error('Canonical evidence exceeded bounded pagination');
    }
    return events;
  };
  const verifyAdministratorInstructions = async (modelContext, proposalCallId) => nativeAdministratorInstructionProof({ modelContext, proposalCallId, events: await sessionEvents(),
    operationId: accepted.operation_id, root: administratorRoot, path: administratorInstructions, marker: administratorInstructionMarker });
  const canonicalEvidence = async () => {
    const current = (await sessionEvents()).filter(event => event.correlation_id === accepted.operation_id);
    const progress = current.filter(event => event.kind === 'runtime/progress-recorded' && event.payload?.storage === 'inline_json').map(event => event.payload.value?.event);
    return { events: current, cleanup_proven: progress.some(event => event?.event === 'host_cleanup_proven'), completion_report: progress.findLast(event => event?.event === 'completion_reported')?.report, completion_delivered: progress.some(event => event?.event === 'completion_delivered') };
  };
  const awaitCleanup = async () => {
    const until = Date.now() + 90000;
    while (Date.now() < until) {
      const state = await api('GET', `/api/agent-sessions/${sessionId}/execution`);
      const session = await api('GET', `/api/agent-sessions/${sessionId}`);
      const evidence = await canonicalEvidence();
      if (state && !['running', 'paused'].includes(state.state) && !session.head.active_turn_id && state.pending_effects === 0 && state.unknown_effects === 0 && evidence.cleanup_proven) {
        if (nativePid) assert.throws(() => process.kill(nativePid, 0), { code: 'ESRCH' });
        if (administrator && administratorPid) await verifyAdministratorPidGone('canonical_cleanup');
        return { execution: state, head: session.head, ...evidence };
      }
      await wait(250);
    }
    throw new Error('Canonical cancellation did not establish actual native cleanup; retain fixture and unknown effects');
  };
  const cancelAndSettle = async () => {
    // A timed-out admission response is not proof that no Turn started. This
    // disposable Session has one driver; recover the exact accepted source
    // from public canonical facts before issuing cancellation.
    const execution = await api('GET', `/api/agent-sessions/${sessionId}/execution`);
    const cancellationTarget = continuation && execution ? nativeContinuationCancellationTarget({ latestOperationId: execution.operation_id,
      admissionOperationId: continuation.admissionOperationId, acceptedOperationId: accepted?.operation_id, pendingAdmissionOperationId }) : undefined;
    if (cancellationTarget === 'prior_settled_not_admitted') {
      const current = await api('GET', `/api/agent-sessions/${sessionId}`);
      nativeContinuationSettlement({ execution, head: current.head, operationId: execution.operation_id, sessionId });
      return { not_admitted: true, head: current.head, prior_failed_operation_untouched: true, prior_operation_id: continuation.admissionOperationId };
    }
    if (!execution) {
      const session = await api('GET', `/api/agent-sessions/${sessionId}`);
      assert(!session.head.active_turn_id, 'An active Turn has no readable execution identity');
      return { not_admitted: true, head: session.head };
    }
    if (accepted?.operation_id !== execution.operation_id) {
      const started = (await sessionEvents()).find(event => event.kind === 'turn/started' && event.correlation_id === execution.operation_id);
      assert(started?.payload?.storage === 'inline_json' && started.payload.value?.source_message_id, 'Cannot recover exact admission identity; no guessed cancellation was issued');
      accepted = { operation_id: execution.operation_id, message_id: started.payload.value.source_message_id, recovered_from_public_canonical_facts: true };
    }
    const request = await api('POST', `/api/agent-sessions/${sessionId}/turns/cancel`, { idempotency_key: randomUUID(), expected_turn_id: accepted.message_id });
    return { request, settlement: await awaitCleanup(), accepted };
  };
  const driveTurn = async (content, targetSteps, role) => {
    checkInterruption();
    activeSteps = targetSteps; activeTurnRole = role; completionAccepted = false; completionAttempts = 0; providerFailure = undefined; currentCompletionMode = 'report_completion'; directAnswerCount = 0;
    completedSteps.clear(); attempts.clear(); activeCompletionIds.clear(); activeTurnCallIds.clear();
    const admissionKey = randomUUID();
    if (continuation) pendingAdmissionOperationId = `turn:user:${continuation.sourceEvidence.source_owner_id}:${sessionId}:${admissionKey}`;
    accepted = await api('POST', `/api/agent-sessions/${sessionId}/turns`, { idempotency_key: admissionKey, input: { content } });
    if (continuation) assert.equal(accepted.operation_id, pendingAdmissionOperationId, 'The accepted Turn does not belong to this run');
    const admission = accepted;
    const until = Date.now() + 300000;
    let execution;
    const resumedPauses = new Set();
    while (Date.now() < until) {
      checkInterruption();
      if (providerFailure) throw providerFailure;
      execution = await api('GET', `/api/agent-sessions/${sessionId}/execution`);
      if (execution) assert.equal(execution.operation_id, admission.operation_id, 'Execution inspection changed to another accepted Turn');
      if (execution?.state === 'completed') break;
      if (execution?.state === 'paused' && execution.pause?.reason === 'EXECUTION_DISPATCH_WINDOW') {
        const pause = execution.pause;
        assert(execution.turn_state === 'running' && execution.checkpoint_retained === true && pause.cleanup_proven === true
          && execution.producer_lease_live === false && execution.pending_effects === 0 && execution.unknown_effects === 0
          && execution.recovery_blocked === false && execution.pause_requested === false,
        'Dispatch-window continuation requires the exact nonterminal checkpoint and proven cleanup without unresolved effects');
        assert(Number.isSafeInteger(pause.revision) && pause.revision > 0
          && Number.isSafeInteger(execution.checkpoint_revision) && execution.checkpoint_revision > 0
          && pause.checkpoint_revision === execution.checkpoint_revision
          && pause.checkpoint_digest === execution.checkpoint_digest && /^[a-f0-9]{64}$/.test(execution.checkpoint_digest)
          && pause.execution_fence === execution.execution_fence,
        'Dispatch-window pause/checkpoint/fence metadata does not match');
        const current = await api('GET', `/api/agent-sessions/${sessionId}`);
        assert.equal(current.head.status, 'paused'); assert.equal(current.head.active_turn_id, admission.operation_id);
        const pauseKey = `${admission.operation_id}:${pause.revision}:${execution.checkpoint_revision}:${execution.checkpoint_digest}`;
        if (!resumedPauses.has(pauseKey)) {
          assert(resumedPauses.size < 8, 'The bounded native acceptance did not converge after eight dispatch-window continuations');
          const request = { operation_id: admission.operation_id, idempotency_key: randomUUID(), expected_pause_revision: pause.revision,
            expected_checkpoint_revision: execution.checkpoint_revision, expected_checkpoint_digest: execution.checkpoint_digest, budget: {} };
          const record = { role, reason: pause.reason, observed_execution_fence: execution.execution_fence, request,
            automatic_effect_replay_authorized: false, started_at: new Date().toISOString() };
          (report.native_resumes ??= []).push(record);
          // The formal request does not accept a fence field. Preparation and
          // commit recheck the checkpoint's actual fence and exact head in the
          // canonical owner transaction; this grants no effect replay or budget.
          const receipt = await api('POST', `/api/agent-sessions/${sessionId}/execution/resume`, request);
          assert.equal(receipt.operation_id, admission.operation_id); assert.equal(typeof receipt.authorization_event_id, 'string');
          assert(Number.isSafeInteger(receipt.seq) && receipt.seq > 0);
          record.receipt = receipt; record.accepted_at = new Date().toISOString(); resumedPauses.add(pauseKey);
        }
        await wait(250); continue;
      }
      assert(execution === null || execution.state === 'running', `Native Agent stopped before completion: ${JSON.stringify(execution)}`); await wait(250);
    }
    assert.equal(execution?.state, 'completed', 'Native product deadline elapsed'); assert.equal(completedSteps.size, targetSteps.length);
    const settlement = await awaitCleanup();
    if (currentCompletionMode === 'report_completion') {
      assert(settlement.completion_report, 'No accepted canonical completion report exists');
      assert(settlement.completion_delivered, 'No canonical completion delivery exists');
      assert(settlement.completion_report.criteria.every(criterion => criterion.disposition === 'supported'), 'The accepted report has unresolved work');
    } else {
      assert(desktopScenario && directAnswerCount > 0 && activeCompletionIds.size === 0, 'Direct completion must come from this actual non-ledger desktop request');
    }
    // Raw /messages exposes event projections, while the public UI history
    // includes the store-derived exact Turn outcome. Use that formal consumer
    // contract rather than inventing a turn_summary row in the raw stream.
    let summary;
    for (let page = 1; page <= 32; page++) {
      const history = await api('GET', `/api/agent-sessions/${sessionId}/message-history?page=${page}&page_size=100`);
      summary = history.items.find(message => message.content?.turn_summary === true && message.content.turn_id === admission.message_id);
      if (summary || !history.has_more) break;
    }
    assert(summary, 'No exact canonical Turn summary exists'); assert.equal(summary.content.turn_state, 'completed'); assert.equal(summary.content.turn_outcome?.kind, 'answered'); assert(summary.content.turn_outcome.message_ids.length > 0);
    return { accepted: admission, execution, settlement, outcome: summary.content.turn_outcome, completion_contract: currentCompletionMode };
  };
  const continueAfterRootControlledAppRestart = async () => {
    const originalSession = await api('GET', `/api/agent-sessions/${sessionId}`);
    assert.equal(originalSession.session.agent_session_id, sessionId);
    assert(!originalSession.head.active_turn_id, 'The retained-service main phase must have fully settled before App exit');
    const originalOwner = JSON.stringify(originalSession.session.owner_ref);
    const deadline = Date.now() + 10 * 60 * 1000;
    await phaseCheckpoint('awaiting_app_exit', {
      retained_service_phase: 'main Turn completed; only the explicitly requested unique service and its files are retained; App-exit survival is not yet verified',
      original_app_pid: appPid, native_restart_deadline: new Date(deadline).toISOString(),
    });
    await waitForPhase('awaiting_app_exit', deadline, () => processIsGone(appPid));
    const survival = await systemScenario.observeAfterAppExit({ appPid });
    const pending = systemScenario.pendingAcceptance.find(item => item.id === 'launchd-survives-native-App-exit');
    if (pending) Object.assign(pending, { status: 'passed', reason: 'The actual old App PID disappeared; the same unique launchd service, observed PID and marker were independently verified after that exit.' });
    await phaseCheckpoint('awaiting_app_restart', { app_exit_survival: { ...survival, observed_at: new Date().toISOString() } });
    const restarted = await waitForPhase('awaiting_app_restart', deadline, async () => {
      let announcement;
      try { announcement = await readNativeAnnouncement(nativeDataset); }
      catch (error) { if (error.code === 'ENOENT') return undefined; throw error; }
      if (announcement.pid === appPid || processIsGone(announcement.pid)) return undefined;
      const host = announcement.host === '::1' ? '[::1]' : announcement.host;
      const candidate = new URL(`http://${host}:${announcement.port}`);
      let response;
      try { response = await fetch(new URL(`/api/agent-sessions/${sessionId}`, candidate), { headers: { Authorization: `Bearer ${token}` }, signal: AbortSignal.timeout(3000) }); }
      catch (error) { report.restart_last_not_ready = { at: new Date().toISOString(), reason: error.message, pid: announcement.pid, port: announcement.port }; return undefined; }
      if ([401, 403].includes(response.status)) throw new Error(`The original owner login JWT was rejected after App restart (HTTP ${response.status}); no new identity or login was substituted`);
      if (!response.ok) { report.restart_last_not_ready = { at: new Date().toISOString(), http_status: response.status, pid: announcement.pid, port: announcement.port }; return undefined; }
      const envelope = await response.json(); const value = envelope.data === undefined ? envelope : envelope.data;
      assert.notEqual(envelope.success, false, 'The restarted App rejected the original owner request');
      assert.equal(value.session.agent_session_id, sessionId, 'Restart must continue the exact original Session');
      assert.equal(JSON.stringify(value.session.owner_ref), originalOwner, 'The Session owner changed across the native App restart');
      const confirmed = await readNativeAnnouncement(nativeDataset);
      if (confirmed.pid !== announcement.pid || confirmed.port !== announcement.port || processIsGone(announcement.pid)) return undefined;
      return { announcement, candidate, session: value };
    });
    const oldOrigin = origin.origin; origin.href = restarted.candidate.href;
    report.restart = { original_app_pid: appPid, new_app_pid: restarted.announcement.pid, old_origin: oldOrigin, new_origin: origin.origin, native_announcement: restarted.announcement, owner_jwt_reused: true, same_session_id: sessionId, observed_at: new Date().toISOString() };
    // Read the old exact Turn on the restarted App before admitting new work.
    report.restart_prior_turn_settlement = await awaitCleanup();
    assert.equal(report.restart_prior_turn_settlement.execution.operation_id, report.accepted.operation_id, 'Unexpected work replaced the original settled Turn before cleanup');
    report.app_restart_service_survival = { ...await systemScenario.observeAfterAppExit({ appPid }), observed_after_new_app_pid: restarted.announcement.pid, observed_at: new Date().toISOString() };
    await phaseCheckpoint('restarted_app_authenticated', { restart: report.restart });
    report.retained_service_cleanup = await driveTurn(`本次唯一 launchd 服务 ${systemScenario.resources.target} 是先前阶段明确要求保留的测试产物。宿主已独立确认原 App PID ${appPid} 正常退出后该服务仍存续，且以同一 owner 登录身份在同数据根的新 App 继续同一 Session。现在仅通过现役工具清理本次随机系统偏好（如已使用）、停止并卸载该唯一服务，读回原生状态并确认已观察进程退出，再删除 ${systemScenario.resources.root}。不重跑主验收、不操作其他服务或既有文件。`, systemScenario.cleanupSteps, 'retained-cleanup');
    assert.equal(await present(systemScenario.resources.root), false, 'Retained-service files remain after product cleanup');
    report.retained_service_final_status = 'stopped_unregistered_and_fixture_deleted';
  };
  const driveSystemPhases = async () => {
    const serviceStart = steps.findIndex(step => step.id === 'system-preferences-write');
    const networkStart = steps.findIndex(step => step.id === 'system-network-start');
    const networkEnd = steps.findIndex(step => step.id === 'system-network-confirm-exit');
    assert(serviceStart > 0 && networkStart > serviceStart && networkEnd >= networkStart && networkEnd < steps.length - 1,
      'The formal system scenario does not expose the expected isolated phase boundaries');
    const resource = systemScenario.resources;
    // Explicit user-scoped phases retain only the requested launchd artifact.
    // Every transient nc lifecycle is completed in one short Turn; no native
    // process handle or desktop observation is carried across a checkpoint.
    const phases = [
      { role: 'system-install-config', steps: steps.slice(0, serviceStart), prompt: `只在本次唯一 ${resource.root} 完成本阶段：用文件工具写入、patch、读回隔离配置；写入唯一用户工具 source，通过原生 install -m755 安装到本次 bin，核对裸命令、literal argv、task PATH及元数据，再用文件工具卸载该工具。只确认本次唯一服务 ${resource.target} 当前不存在，不注册服务、不监听网络。保留本次配置/source给后续阶段；本阶段完成不代表后续服务和网络通过。` },
      { role: 'system-service', steps: steps.slice(serviceStart, networkStart), prompt: `只执行本阶段：本次随机 CFPreferences domain ${resource.preferencesDomain} 已独立证明事前不存在，原生写入、读回并删除恢复不存在；用文件工具写入 ${resource.plist}，注册并启动唯一用户 launchd 服务 ${resource.target}，核对真实标记、PID与原生状态。按明确意图保留该服务和本次文件给后续阶段，不监听网络，不声称最终卸载已完成。` },
      { role: 'system-network', steps: steps.slice(networkStart, networkEnd + 1), prompt: `只执行本阶段的完整本机网络生命周期：在 ${resource.root} 以受管进程启动 127.0.0.1:${resource.loopbackPort} 的唯一 nc 监听，独立 running poll确认同 PID监听，再写入本次 HTTP响应，以 curl --noproxy '*'请求唯一 ${resource.networkPath}，核对响应与请求输出，取消并末次 poll确认该临时进程退出。不得沿用其他 Turn 的 process_id；保留先前服务 ${resource.target} 及文件，不在此阶段做最终卸载。` },
      { role: 'system-finalize', steps: steps.slice(networkEnd + 1), prompt: `只执行本阶段：通过文件工具恢复并读回 ${resource.config}。${retainServiceForAppExit ? `按明确意图继续保留唯一服务 ${resource.target} 和本次文件用于 App 退出存续测试；不要声称该后续测试已完成。` : `原生停止并卸载本次唯一服务 ${resource.target}，确认原生状态不存在及已观察 PID已退出，再通过文件工具删除 ${resource.root}。`}不重跑先前安装、配置修改或网络阶段；不操作任何既有服务和文件。` },
    ];
    report.system_phases = [];
    for (const phase of phases) {
      const result = await driveTurn(phase.prompt, phase.steps, phase.role);
      report.system_phases.push({ role: phase.role, step_ids: phase.steps.map(step => step.id), ...result });
      Object.assign(report, result);
    }
    assert.equal(report.system_phases.length, phases.length);
  };
  const verifyContinuationSourcePreserved = async () => {
    const bytes = await readNativeOwnedFixtureBytes(nativeAdministratorContinuationFixture.sourceReport);
    const sha256 = createHash('sha256').update(bytes).digest('hex');
    assert.equal(sha256, continuation.sourceEvidence.source_sha256, 'The original failed report must remain byte-for-byte unchanged');
    const sourceEvents = (await sessionEvents()).filter(event => event.correlation_id === continuation.sourceEvidence.source_operation_id);
    const digest = evidenceDigest(sourceEvents);
    assert.equal(digest, continuation.sourceEvidence.source_events_sha256, 'The original failed operation must remain unchanged');
    let retry;
    if (continuation.retryEvidence) {
      const retryBytes = await readNativeOwnedFixtureBytes(nativeAdministratorContinuationFixture.retryReport);
      const retrySha256 = createHash('sha256').update(retryBytes).digest('hex');
      assert.equal(retrySha256, continuation.retryEvidence.retry_report_sha256, 'The failed continuation report must remain byte-for-byte unchanged');
      const retryEventsDigest = evidenceDigest((await sessionEvents()).filter(event => event.correlation_id === continuation.retryEvidence.retry_operation_id));
      assert.equal(retryEventsDigest, continuation.retryEvidence.retry_events_sha256, 'The failed continuation operation changed');
      retry = { retry_report_sha256: retrySha256, retry_status_retained: 'failed', retry_operation_id: continuation.retryEvidence.retry_operation_id,
        retry_operation_events_sha256: retryEventsDigest, retry_operation_preserved: true, retry_file_overwritten: false };
    }
    return { source_report_sha256: sha256, source_status_retained: 'failed', original_operation_id: continuation.sourceEvidence.source_operation_id,
      original_operation_events_sha256: digest, original_operation_preserved: true, source_file_overwritten: false, retry };
  };
  try {
    if (protectedParent) {
      assert(protectedParent.environment.available, `Protected-user environment is unavailable: ${JSON.stringify(protectedParent.environment)}`);
      assert.equal(dirname(outside), protectedParent.canonical_parent, 'The protected test target must be a direct child of the actual Documents directory');
      assert(/^nomifun-global-os-product-[0-9a-f-]{36}$/.test(basename(outside)), 'Protected test directory must use the host-allocated unique name');
      assert.equal(await present(outside), false, 'The protected test target already exists; do not touch it');
    }
    const provider = await api('POST', '/api/providers', { platform: 'custom', name: `Global OS local acceptance ${randomUUID()}`, base_url: providerUrl, auth_scheme: 'bearer', credentials: { api_keys: ['local-fixture-only'] }, enabled: true,
      initial_model: { model: 'global-os-local', enabled: true, capabilities: [{ task: 'chat', traits: acceptanceScenario?.modelTraits ?? [], protocol: 'openai.chat_text', connection_role: 'default', output_limit: 4096 }] } });
    const model = { provider_id: provider.provider_id, model: 'global-os-local' };
    if (continuation) {
      report.session_id = sessionId;
      await verifyContinuationSourcePreserved();
      const immediate = await api('GET', `/api/agent-sessions/${sessionId}`);
      assert.deepEqual(immediate.session.agent_binding, continuation.before.session.agent_binding, 'The Session binding changed after continuation admission');
      nativeContinuationSettlement({ execution: await api('GET', `/api/agent-sessions/${sessionId}/execution`), head: immediate.head,
        operationId: continuation.admissionOperationId, sessionId });
      // This public command prepares the normal bundled contract transition,
      // then derives the selected model. It never authors a replacement Agent.
      await api('PUT', `/api/agent-sessions/${sessionId}/model`, model);
      const after = await api('GET', `/api/agent-sessions/${sessionId}`);
      nativeContinuationSettlement({ execution: await api('GET', `/api/agent-sessions/${sessionId}/execution`), head: after.head,
        operationId: continuation.admissionOperationId, sessionId });
      const afterProjection = await api('GET', `/api/agent-sessions/${sessionId}/projection`);
      assert.equal(afterProjection.extra.workspace, workspace);
      assert.deepEqual(afterProjection.agent_snapshot.enabled_capability_actions, continuation.projection.agent_snapshot.enabled_capability_actions,
        'The public model preparation must not add or remove Action grants');
      const ref = after.session.agent_binding.preset_revision_ref;
      const afterRevision = await api('GET', `/api/agent-presets/${ref.preset_id}/revisions/${ref.revision}`);
      continuation.report.evolution = nativeContinuationEvolution({ before: continuation.before, after, sourceBinding: continuation.sourceBinding,
        sourceRevision: continuation.sourceRevision, beforeRevision: continuation.beforeRevision, afterRevision,
        events: await sessionEvents(), sourceLastSeq: continuation.sourceEvidence.source_head_last_seq });
      const original = continuation.sourceBinding.preset_revision_ref;
      assert.equal(evidenceDigest(await api('GET', `/api/agent-presets/${original.preset_id}/revisions/${original.revision}`)),
        continuation.report.source_original_revision_sha256, 'The original selected Agent revision changed');
      continuation.report.after_binding_version = after.session.agent_binding.binding_version;
      continuation.report.after_snapshot_ref = after.session.agent_binding.resolved_snapshot_ref;
      continuation.report.model_command = { method: 'PUT', path: `/api/agent-sessions/${sessionId}/model`, provider_id: model.provider_id, model: model.model };
      continuation.report.original_failure_preservation_before_new_turn = await verifyContinuationSourcePreserved();
    } else {
      const preset = await api('POST', `/api/agent-presets/from-template/${template}`, { reuse_existing: false, display_name: 'Global OS native regression', model });
      // Do not add capabilities, resources or bindings. This is the same selected
      // default template and standard Session resource preparation used by the UI.
      const session = await api('POST', '/api/agent-sessions', { preset_id: preset.preset.preset_id, model, workspace });
      sessionId = session.agent_session_id; report.session_id = sessionId;
    }
    const content = acceptanceScenario?.prompt ?? (administrator ?
      `Only operate on this host-allocated fixture ${fixture}. Through the native administrator execution path, create ${administratorRoot} as root:0700 and a benign root:0600 AGENTS.md inside it; independently prove an ordinary-user write is refused. Preserve the actual instruction preflight: if new rules defer a call, read the updated model instructions and submit a fresh call identity. Write the absent random marker ${marker}, then verify uid, contents and root:0600 metadata. Next use start_process with execution_identity=administrator and cwd=${administratorRoot} to run the bounded 60-second sh wait loop, printing real uid, pwd and login HOME (expected ${homedir()}). Make a distinct running poll and independently read exact PID UID through ps; use the same helper/XPC process cancellation and terminal poll, require cancelled plus cleanup.reaped, and independently prove the exact PID absent through ps. Only then remove this marker and the fixture AGENTS.md and rmdir this root fixture. No host signal, sudo, PTY, stdin expansion, additional authentication method or unrelated target.` :
      `请在当前工作区之外创建独立测试目录 ${outside}，用文件工具写入并修改一份文本，再在该目录启动一个本机等待进程，输出实际工作目录和文本内容；独立 poll 核对进程仍在运行及输出，随后停止并确认该进程已退出，最后删除本次创建的测试目录，报告实际结果。`);
    if (systemScenario) await driveSystemPhases();
    else Object.assign(report, await driveTurn(content, steps, 'main'));
    assert.equal(await present(administrator ? administratorRoot : outside), false);
    if (retainServiceForAppExit) await continueAfterRootControlledAppRestart();
    report.status = 'passed';
  } catch (error) {
    report.status = 'failed'; report.failure = safeFailure(error);
    report.failed_phase = { role: activeTurnRole, accepted };
    report.accepted ??= accepted;
    if (sessionId) {
      try {
        const cleanup = await cancelAndSettle();
        report.failure_cleanup_request = cleanup.request;
        report.failure_settlement = cleanup.settlement;
        report.failure_admission = cleanup.accepted ?? { not_admitted: cleanup.not_admitted, head: cleanup.head };
      } catch (cleanupError) { report.failure_cleanup = safeFailure(cleanupError); }
    }
    const scenarioCleanupNeeded = acceptanceScenario?.cleanupNeeded ? acceptanceScenario.cleanupNeeded() : systemScenario ? await present(systemScenario.resources.root) : false;
    if (acceptanceScenario && report.failure_settlement && !interruption.signal.aborted && scenarioCleanupNeeded) {
      try {
        // This is a new explicitly scoped recovery Turn. The first failure and
        // its exact cancellation/cleanup proof remain separate in the report.
        const cleanupPrompt = desktopScenario ? `原桌面验收失败，先前 Turn 已取消且临时资源与 pending/unknown 效果均已结算。只通过现役 Computer 工具取得新观察、聚焦并退出本次新启动的 Calculator PID ${desktopScenario.resources.pid}，确认该 PID 已退出。不重跑验收，不操作其他应用、用户文件或剪贴板。` : `原隔离验收失败，先前 Turn 已取消且临时进程清理、pending/unknown 效果均已结算。现在只清理本次随机偏好和唯一服务 ${systemScenario.resources.target}：通过系统原生状态确认，存在则 bootout；确认该服务及已观察 PID 退出后，再用文件工具删除 ${systemScenario.resources.root}。不重跑验收、不修改其他对象。`;
        report.recovery_cleanup = await driveTurn(cleanupPrompt, acceptanceScenario.cleanupSteps, 'cleanup');
      } catch (cleanupError) {
        report.recovery_cleanup_failure = { ...safeFailure(cleanupError), accepted };
        try {
          const cleanup = await cancelAndSettle();
          report.recovery_cancel_request = cleanup.request;
          report.recovery_failure_settlement = cleanup.settlement;
        } catch (settlementError) { report.recovery_cleanup_unproven = safeFailure(settlementError).message; }
      }
    }
    if (acceptanceScenario?.sanitizeFailure) { const safe = safeFailure(error); const failure = new Error(safe.message); failure.name = safe.name; throw failure; }
    throw error;
  } finally {
    let continuationIntegrityFailure;
    if (continuation) {
      try {
        continuation.report.original_failure_preservation_after_new_turn = await verifyContinuationSourcePreserved();
        assert(continuation.report.read_tool_schema || report.status === 'failed', 'No actual administrator-capable read schema was advertised');
        continuation.report.original_workspace_retained = true;
      } catch (error) {
        continuationIntegrityFailure = error; report.status = 'failed'; report.continuation_integrity_failure = safeFailure(error);
      }
    }
    report.finished_at = new Date().toISOString(); report.observations = Object.fromEntries(observations); report.raw_tool_results = Object.fromEntries(rawResults); report.completion_receipts = Object.fromEntries(completions);
    if (acceptanceScenario) report.cleanup_intent = { ...acceptanceScenario.cleanupIntent, temporary_process: acceptanceScenario.cleanupIntent.temporary_process() };
    report.remaining_fixture = { outside: await present(outside).catch(error => ({ state: 'unknown', os_code: error.code, detail: safeFailure(error).message })), administrator_root: await present(administratorRoot), system_root: systemScenario ? await present(systemScenario.resources.root) : undefined };
    // Only remove known empty user-owned fixture directories. Never recursively
    // erase failure evidence or an administrator object with unknown effects.
    for (const directory of [...(continuation ? [] : [workspace]), fixture]) try { await rmdir(directory); } catch (error) { if (!['ENOENT', 'ENOTEMPTY', 'EACCES', 'EPERM'].includes(error.code)) report.fixture_cleanup_error ??= error.message; }
    await writeFile(reportPath, `${JSON.stringify(safeReport(report), null, 2)}\n`, { flag: 'wx', mode: 0o600 });
    process.removeListener('SIGINT', interrupted); process.removeListener('SIGTERM', interrupted); signal?.removeEventListener('abort', forwardedAbort);
    server.closeIdleConnections(); server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
    console.log(JSON.stringify({ status: report.status, report: reportPath, session_id: sessionId }));
    if (continuationIntegrityFailure) throw continuationIntegrityFailure;
  }
  return safeReport(report);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  if (process.argv.includes('--help')) console.log('node scripts/validation/global-os-native.mjs --base-url http://127.0.0.1:PORT --token-file /ABS/ISOLATED-OWNER-LOGIN-JWT --report-dir /ABS/EVIDENCE [--template chat.minimal|assistant.general] [--scenario files-process|system|desktop] [--administrator [--continue-session PINNED_ID --continuation-source-report /ABS/PINNED_FAILED_REPORT [--continuation-retry-report /ABS/PINNED_ZERO_DISPATCH_FAILURE] --native-data-root /PRIVATE_MARKED_PARENT/native-data]] [--protected-user-directory] [--desktop-background-focus] [--retain-service-for-app-exit --app-pid ACTUAL_APP_PID --native-data-root /PRIVATE_MARKED_PARENT/native-data]. Run against an already running signed native App with its own isolated test dataset. Obtain the token by normal owner setup/login; the native browser trust secret is not an owner JWT. Run ordinary, system/network, desktop and administrator cases separately. --protected-user-directory is a boolean ordinary files-process option: the actual existing homedir/Documents parent is resolved read-only, only an absent nomifun-global-os-product-UUID name is allocated there, and all creation/patch/search/cwd/process-cancel/deletion effects use the Agent; the workspace stays in the private /tmp fixture. Missing or denied Documents metadata is recorded, never prepared by the host. --desktop-background-focus requires --scenario desktop and --native-data-root. The marked private data announcement and authenticated permission caller must identify the same live formal App; only that minimal identity is passed to the read-only window verifier. The Agent establishes background setup and performs all focus effects through Computer; no reviewer focus mutation is used. Desktop defaults to assistant.general with actual screenshot input capability on its local fixture model; all UI effects use the existing Agent Computer tools and reports omit private pixels/AX/global window text. Retained-service verification implies --scenario system: the driver emits an awaiting_app_exit checkpoint, waits at most ten minutes for the reviewer to quit and restart the App, independently observes service survival, authenticates the same owner JWT against the new private port announcement, then continues the same Session/provider with a new cleanup Turn. The driver never quits the App or kills a raw PID; SIGINT/SIGTERM cancel its wait and preserve any unproven service target. Administrator continuation is limited to the exact pinned private failed report, original Session, owner JWT and fourth dataset. It uses only the public model command to prepare current bundled contracts, preserves Agent intent/resources/workspace and original failure, allocates a fresh root fixture, and records source/event/schema digests. An optional retry accepts only the one pinned failed continuation whose canonical records prove no user effect dispatch, and requires that exact settled latest operation while retaining both old failures. It never creates a new Agent/Session or changes grants; prior-run receipts cannot satisfy new-run calls. Arbitrary administrator paths are forbidden. OS authentication stays in native UI. The local scripted provider is deterministic product-path evidence, not live-model or visual acceptance.');
  else {
    assert(!process.argv.includes('--administrator-directory'), '--administrator-directory is forbidden; use --administrator with a host-allocated fixture');
    const retained = process.argv.includes('--retain-service-for-app-exit');
    const background = process.argv.includes('--desktop-background-focus');
    const continued = nativeContinuationCliArguments(process.argv);
    assert(retained || !process.argv.includes('--app-pid'), '--app-pid requires --retain-service-for-app-exit');
    assert(retained || background || continued || !process.argv.includes('--native-data-root'), '--native-data-root requires retained-service, background-focus or pinned administrator continuation verification');
    const selectedScenario = option('--scenario') ?? (retained ? 'system' : 'files-process');
    const selectedTemplate = option('--template') ?? (selectedScenario === 'desktop' ? 'assistant.general' : 'chat.minimal');
    const nativeDataRoot = retained || background || continued ? required('--native-data-root') : undefined;
    const continueSession = option('--continue-session'), continuationSourceReport = option('--continuation-source-report'), continuationRetryReport = option('--continuation-retry-report');
    const administrator = process.argv.includes('--administrator');
    nativeContinuationOptions({ administrator, scenario: selectedScenario, template: selectedTemplate, continueSession, continuationSourceReport, continuationRetryReport, nativeDataRoot,
      retainServiceForAppExit: retained, desktopBackgroundFocus: background, protectedUserDirectory: process.argv.includes('--protected-user-directory') });
    const tokenFile = required('--token-file');
    if (continued) assert.equal(tokenFile, join(dirname(nativeAdministratorContinuationFixture.nativeDataRoot), 'owner-jwt'), 'Continuation requires its exact private owner JWT file');
    const token = (continued ? (await readNativeOwnedFixtureBytes(tokenFile, 4096)).toString('utf8') : await readFile(tokenFile, 'utf8')).trim();
    await runNativeRegression({ baseUrl: required('--base-url'), token, tokenFile, reportDirectory: required('--report-dir'), administrator, scenario: selectedScenario, template: selectedTemplate,
      retainServiceForAppExit: retained, appPid: retained ? Number(required('--app-pid')) : undefined, nativeDataRoot,
      protectedUserDirectory: process.argv.includes('--protected-user-directory'), desktopBackgroundFocus: background, continueSession, continuationSourceReport, continuationRetryReport });
  }
}
