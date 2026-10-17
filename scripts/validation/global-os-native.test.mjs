import { expect, test } from 'bun:test';
import { nativeAdministratorInstructionContext, nativeAdministratorInstructionProof, nativeCompletionArguments } from './global-os-native.mjs';

function fixture() {
  const operationId = 'turn:test', root = '/private/tmp/fixture/admin-root', path = `${root}/AGENTS.md`, marker = 'NOMIFUN_ADMIN_INSTRUCTION_fixture';
  const context = `Repository instructions observed as administrator and scoped to ${path}:\n${marker}`;
  const messages = [{ role: 'system', content: context }];
  const modelContext = nativeAdministratorInstructionContext({ messages, path, marker });
  const notice = `Repository instruction body omitted from durable history (${modelContext.bytes} UTF-8 bytes, sha256:${modelContext.sha256}). Re-read current rules through the authorized workspace tool; this digest grants no authority.`;
  const event = (seq, value) => ({ seq, correlation_id: operationId, kind: 'runtime/progress-recorded', payload: { storage: 'inline_json', value: { event: value } } });
  const call = (seq, id, args) => event(seq, { event: 'tool_call_completed', step: 0, call: { call_id: id, name: 'read_file', arguments: { execution_identity: 'administrator', ...args } } });
  const done = (seq, id, is_error = false) => event(seq, { event: 'tool_completed', step: 0, result: { call_id: id, is_error } });
  const proposal = seq => event(seq, { event: 'tool_call_completed', step: 3, call: { call_id: 'successful-start', name: 'start_process', arguments: { execution_identity: 'administrator', cwd: root } } });
  const events = [call(1, 'scope', { format: 'instruction_scope', path: root }), done(2, 'scope'), call(3, 'text', { path, missing_ok: true }), done(4, 'text'), event(5, { event: 'instructions_updated', context: notice }), proposal(6)];
  return { input: { operationId, root, path, marker, modelContext, proposalCallId: 'successful-start', events }, messages, context, event, call, done, proposal };
}

test('admin instruction evidence binds successful start to its earlier proposal context and canonical reads', () => {
  const { input } = fixture();
  const proof = nativeAdministratorInstructionProof(input);
  expect(proof.proposal_seq).toBe(6);
  expect(proof.instructions_updated_seq).toBe(5);
  expect(proof.text_call_id).toBe('text');
  // No second instruction body or provider message collection is retained.
  expect(Object.keys(input.modelContext).sort()).toEqual(['bytes', 'matching_system_context_count', 'sha256']);
});

test('post-start instructions cannot certify a proposal emitted before their publication', () => {
  const { input, proposal } = fixture();
  expect(() => nativeAdministratorInstructionProof({ ...input, events: [proposal(0), ...input.events.slice(0, -1)] })).toThrow('before this exact successful start proposal');
  const before = nativeAdministratorInstructionContext({ messages: [], path: input.path, marker: input.marker });
  expect(() => nativeAdministratorInstructionProof({ ...input, modelContext: before })).toThrow('actual proposal request system context');
  expect(() => nativeAdministratorInstructionProof({ ...input, proposalCallId: 'different-proposal' })).toThrow();
});

test('user or assistant echoes, changed context and another Turn cannot prove system instruction delivery', () => {
  const { input, context } = fixture();
  for (const role of ['user', 'assistant', 'tool']) {
    const modelContext = nativeAdministratorInstructionContext({ messages: [{ role, content: context }], path: input.path, marker: input.marker });
    expect(() => nativeAdministratorInstructionProof({ ...input, modelContext })).toThrow();
  }
  expect(() => nativeAdministratorInstructionProof({ ...input, modelContext: { ...input.modelContext, sha256: '0'.repeat(64) } })).toThrow();
  expect(() => nativeAdministratorInstructionProof({ ...input, operationId: 'another-turn' })).toThrow();
});

test('ordinary identity, missing scope or failed privileged read cannot prove administrator acquisition', () => {
  const { input, call, done } = fixture();
  expect(() => nativeAdministratorInstructionProof({ ...input, events: input.events.map(value => value.seq === 3 ? call(3, 'text', { path: input.path, missing_ok: true, execution_identity: 'current_user' }) : value) })).toThrow();
  expect(() => nativeAdministratorInstructionProof({ ...input, events: input.events.map(value => value.seq === 4 ? done(4, 'text', true) : value) })).toThrow();
  expect(() => nativeAdministratorInstructionProof({ ...input, events: input.events.filter(value => value.seq !== 1) })).toThrow();
});

import { mkdtemp, writeFile, chmod, symlink, link, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { nativeAdministratorContinuationFixture as pin, nativeContinuationOptions, nativeContinuationCliArguments, nativeContinuationSource,
  nativeContinuationSettlement, nativeContinuationCaller, nativeContinuationRetryAncestry, nativeContinuationRetrySource, nativeContinuationZeroUserDispatch, nativeContinuationCancellationTarget, nativeContinuationSession, nativeContinuationEvolution,
  nativeAdministratorReadSchema, nativeContinuationMutation, nativeRunCallId, nativeToolReceiptDisposition,
  readNativeOwnedFixtureBytes } from './global-os-native.mjs';

const owner = '0190f5fe-7c00-7a00-8000-000000000001';
const sourceOperation = `turn:user:${owner}:${pin.sessionId}:0190f5fe-7c00-7a00-8000-000000000002`;
const copy = value => structuredClone(value);
function terminal() {
  return { operation_id: sourceOperation, state: 'cancelled', turn_state: 'cancelled', producer_lease_live: false,
    checkpoint_retained: false, recovery_blocked: false, pause_requested: false, automatic_replay_authorized: false,
    pending_effects: 0, unknown_effects: 0, checkpoint_seq: null, checkpoint_digest: null, pause: null };
}
function continuationSource() {
  const accepted = { agent_session_id: pin.sessionId, operation_id: sourceOperation, message_id: '0190f5fe-7c00-7a00-8000-000000000003' };
  return { schema: 'nomifun.global-os-native-regression.v2', status: 'failed', scenario: 'administrator', administrator: true,
    session_id: pin.sessionId, template: pin.template, fixture: pin.fixture, workspace: join(pin.fixture, 'workspace'),
    phase_checkpoints: [], failure: { message: 'preserved first refusal' }, accepted, failure_admission: copy(accepted),
    failed_phase: { role: 'main', accepted: copy(accepted) }, checks: [{ id: 'ordinary-user-denied', status: 'passed', driver_uid: 501, fixture_uid: 0, fixture_mode: '700' }],
    failure_settlement: { cleanup_proven: true, execution: terminal(), head: { session_id: pin.sessionId, status: 'ready', last_seq: 20 },
      events: [{ agent_session_id: pin.sessionId, seq: 20, kind: 'turn/cancelled', correlation_id: sourceOperation }] } };
}
const sourceArgs = { sessionId: pin.sessionId, template: pin.template, driverUid: 501, sha256: pin.sourceSha256 };

test('continuation options admit only the one complete pinned administrator tuple', () => {
  const options = { administrator: true, scenario: 'files-process', template: pin.template, continueSession: pin.sessionId,
    continuationSourceReport: pin.sourceReport, nativeDataRoot: pin.nativeDataRoot };
  expect(nativeContinuationOptions(options)).toBe(true);
  expect(nativeContinuationOptions({ administrator: false, scenario: 'files-process' })).toBe(false);
  for (const changed of [{ administrator: false }, { scenario: 'system' }, { scenario: 'desktop' }, { template: 'chat.minimal' },
    { continueSession: 'different' }, { continuationSourceReport: undefined }, { continueSession: undefined },
    { continuationSourceReport: pin.sourceReport + '.awaiting_app_exit.json' }, { nativeDataRoot: undefined },
    { nativeDataRoot: '/private/tmp/other/native-data' }, { protectedUserDirectory: true }, { retainServiceForAppExit: true }, { desktopBackgroundFocus: true }]) {
    expect(() => nativeContinuationOptions({ ...options, ...changed })).toThrow();
  }
});

test('source guard preserves the exact failed report, operation, UID and template', () => {
  const report = continuationSource();
  expect(nativeContinuationSource(report, sourceArgs).source_status).toBe('failed');
  for (const change of [r => { r.status = 'passed'; }, r => { r.status = 'awaiting_app_restart'; }, r => { r.scenario = 'files-process'; },
    r => { r.administrator = false; }, r => { r.template = 'chat.minimal'; }, r => { r.session_id = 'other'; },
    r => { r.failure_admission.operation_id = 'other'; }, r => { r.failed_phase.accepted.message_id = 'other'; },
    r => { r.checks[0].driver_uid = 502; }, r => { r.phase_checkpoints.push({ phase: 'checkpoint' }); },
    r => { r.continuation = {}; }, r => { r.failure_settlement.cleanup_proven = false; }]) {
    const invalid = copy(report); change(invalid); expect(() => nativeContinuationSource(invalid, sourceArgs)).toThrow();
  }
  expect(() => nativeContinuationSource(report, { ...sourceArgs, sha256: '0'.repeat(64) })).toThrow();
});

test('source settlement rejects unknown, pending, paused, checkpoint and live producer facts', () => {
  const valid = { execution: terminal(), head: { session_id: pin.sessionId, status: 'ready' }, operationId: sourceOperation, sessionId: pin.sessionId };
  expect(nativeContinuationSettlement(valid).state).toBe('cancelled');
  for (const key of ['producer_lease_live', 'checkpoint_retained', 'recovery_blocked', 'pause_requested', 'automatic_replay_authorized']) {
    for (const value of [true, null, undefined]) expect(() => nativeContinuationSettlement({ ...valid, execution: { ...valid.execution, [key]: value } })).toThrow();
  }
  for (const changed of [{ pending_effects: 1 }, { unknown_effects: 1 }, { pending_effects: null }, { checkpoint_seq: 1 },
    { checkpoint_digest: 'hash' }, { pause: {} }, { state: 'running' }, { state: 'paused' }, { state: 'unknown' },
    { operation_id: 'another' }, { turn_state: 'completed' }]) expect(() => nativeContinuationSettlement({ ...valid, execution: { ...valid.execution, ...changed } })).toThrow();
  expect(() => nativeContinuationSettlement({ ...valid, head: { ...valid.head, status: 'paused' } })).toThrow();
  expect(() => nativeContinuationSettlement({ ...valid, head: { ...valid.head, active_turn_id: 'other' } })).toThrow();
});

test('native caller proof requires the same formal App PID, origin and real login UID', () => {
  const valid = { origin: 'http://127.0.0.1:12345', announcement: { host: '127.0.0.1', port: 12345, pid: 123 },
    caller: { platform: 'macos', process_id: 123, app_bundle_identifier: 'com.nomifun.desktop' },
    uidObservation: { pid: 123, exit_code: 0, stdout: '501\n', stderr: '' }, driverUid: 501 };
  nativeContinuationCaller(valid);
  for (const changed of [{ origin: 'http://127.0.0.1:54321' }, { origin: 'http://localhost:12345' },
    { caller: { ...valid.caller, app_bundle_identifier: 'com.nomifun.desktop.dev' } }, { caller: { ...valid.caller, process_id: 124 } },
    { uidObservation: { ...valid.uidObservation, stdout: '0\n' } }, { uidObservation: { ...valid.uidObservation, exit_code: 1 } }])
    expect(() => nativeContinuationCaller({ ...valid, ...changed })).toThrow();
});

test('session guard rejects Remote, Attempt, foreign owner and changed workspace bindings', () => {
  const sourceEvidence = { source_owner_id: owner }, binding = { binding_version: 1 };
  const valid = { sourceEvidence, observation: { session: { agent_session_id: pin.sessionId, owner_ref: { principal_kind: 'user', principal_id: owner }, agent_binding: binding } },
    selection: { editable: true, binding_version: 1 }, projection: { session_purpose: 'conversation', extra: { workspace: join(pin.fixture, 'workspace') }, agent_snapshot: { canonical_binding: copy(binding) } } };
  nativeContinuationSession(valid);
  for (const change of [v => { v.observation.session.remote_binding_provenance = {}; }, v => { v.observation.session.parent_session_id = 'attempt'; },
    v => { v.selection.editable = false; }, v => { v.selection.binding_version = 2; }, v => { v.observation.session.owner_ref.principal_id = 'other'; },
    v => { v.projection.extra.workspace = '/other'; }, v => { v.projection.agent_snapshot.canonical_binding.binding_version = 2; }]) {
    const invalid = copy(valid); change(invalid); expect(() => nativeContinuationSession(invalid)).toThrow();
  }
});

test('private fixture reads reject symlinks, hardlinks, exposed files and oversized content', async () => {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-continuation-guard-'));
  try {
    const path = join(root, 'source'); await writeFile(path, 'bounded', { mode: 0o600 });
    expect((await readNativeOwnedFixtureBytes(path, 64)).toString()).toBe('bounded');
    await symlink(path, join(root, 'symlink')); await expect(readNativeOwnedFixtureBytes(join(root, 'symlink'))).rejects.toThrow();
    await link(path, join(root, 'hardlink')); await expect(readNativeOwnedFixtureBytes(path)).rejects.toThrow();
    await rm(join(root, 'hardlink')); await chmod(path, 0o644); await expect(readNativeOwnedFixtureBytes(path)).rejects.toThrow();
    await chmod(path, 0o600); await expect(readNativeOwnedFixtureBytes(path, 2)).rejects.toThrow();
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('per-run call namespaces cannot consume old same-step or completion receipts', () => {
  const old = nativeRunCallId('old-run', 'admin-process-start-1'), current = nativeRunCallId('new-run', 'admin-process-start-1');
  expect(current).not.toBe(old);
  const completed = new Set();
  for (const id of [old, nativeRunCallId('old-run', 'completion-1'), 'admin-process-start-1']) {
    const disposition = nativeToolReceiptDisposition({ id, runId: 'new-run', known: true, active: true });
    if (disposition === 'current_turn') completed.add('admin-process-start');
    expect(disposition).toBe('historical');
  }
  expect(completed.size).toBe(0);
  expect(nativeToolReceiptDisposition({ id: current, runId: 'new-run', known: true, active: false })).toBe('prior_turn');
  expect(nativeToolReceiptDisposition({ id: current, runId: 'new-run', known: true, active: true })).toBe('current_turn');
});

test('completion labels stay within the contract while exact long per-run evidence IDs remain intact', () => {
  const ids = Array.from({ length: 13 }, (_, index) => nativeRunCallId('924f3f2c-a9b6-4051-969f-32aa76bbc74f', `system-install-config-system-config-target-instruction-scope-${index}`));
  // Reproduce the native system fixture failure: eight valid call identities
  // overflow the optional display label if repeated as its prose.
  expect(`本机实际工具结果 ${ids.slice(0, 8).join(', ')}`.length).toBeGreaterThan(512);
  const properties = { criteria: { items: { properties: {
    step: { type: 'string', minLength: 1, maxLength: 512 },
    evidence_call_ids: { maxItems: 8, items: { enum: ids } },
  } } }, observed_tool_error_count: { const: 1 }, observed_command_failure_count: { const: 0 } };
  const tool = { function: { parameters: { properties } } };
  const input = { tool, observedCallIds: [...ids, 'old-run-ineligible-observation'], summary: '只汇报本阶段实际结果。' };
  const result = nativeCompletionArguments(input);
  expect(result.summary).toBe(input.summary);
  expect(result.criteria.map(criterion => criterion.evidence_call_ids.length)).toEqual([8, 5]);
  expect(result.criteria.flatMap(criterion => criterion.evidence_call_ids)).toEqual(ids);
  for (const criterion of result.criteria) {
    expect(criterion.step.length).toBeGreaterThanOrEqual(properties.criteria.items.properties.step.minLength);
    expect(criterion.step.length).toBeLessThanOrEqual(properties.criteria.items.properties.step.maxLength);
    expect(criterion.step).toBe('本阶段已核验的本机工具结果');
    expect(criterion.disposition).toBe('supported');
  }
  expect(result.observed_tool_error_count).toBe(1);
  expect(result.observed_command_failure_count).toBe(0);
  properties.criteria.items.properties.evidence_call_ids.maxItems = 3;
  expect(nativeCompletionArguments(input).criteria.map(criterion => criterion.evidence_call_ids.length)).toEqual([3, 3, 3, 3, 1]);
  expect(() => nativeCompletionArguments({ ...input, observedCallIds: ['old-run-ineligible-observation'] })).toThrow('No actual matching tool observation');
});

test('the observed read tool must expose the actual administrator enum and all read formats', () => {
  const tool = { type: 'function', function: { name: 'read_file', parameters: { type: 'object', additionalProperties: false, properties: {
    execution_identity: { type: 'string', enum: ['current_user', 'administrator'], default: 'current_user' }, format: { enum: ['text', 'image', 'instruction_scope'] } } } } };
  expect(nativeAdministratorReadSchema([tool]).administrator_supported).toBe(true);
  for (const change of [t => { delete t.function.parameters.properties.execution_identity; }, t => { t.function.parameters.properties.execution_identity.enum = ['current_user']; },
    t => { t.function.parameters.properties.execution_identity.default = 'administrator'; }, t => { t.function.parameters.properties.format.enum = ['text']; }]) {
    const invalid = copy(tool); change(invalid); expect(() => nativeAdministratorReadSchema([invalid])).toThrow();
  }
  expect(() => nativeAdministratorReadSchema([{ function: { name: 'ToolSearch' } }])).toThrow();
});

test('continuation cannot create an Agent/Session or modify grants/resources', () => {
  for (const [method, path] of [['POST', '/api/providers'], ['PUT', `/api/agent-sessions/${pin.sessionId}/model`], ['POST', `/api/agent-sessions/${pin.sessionId}/turns`]]) nativeContinuationMutation(method, path, pin.sessionId);
  for (const [method, path] of [['POST', '/api/agent-sessions'], ['POST', '/api/agent-presets/from-template/assistant.general'],
    ['PUT', `/api/agent-sessions/${pin.sessionId}/agent`], ['PUT', `/api/agent-sessions/${pin.sessionId}/capability-selection`], ['PUT', `/api/agent-sessions/${pin.sessionId}/workspace`]])
    expect(() => nativeContinuationMutation(method, path, pin.sessionId)).toThrow();
});

function evolutionFixture() {
  const ref = (id, digest) => ({ preset_id: id, revision: 1, revision_digest: digest.repeat(64) });
  const snapshot = digest => ({ snapshot_id: `resolved:${digest}`, snapshot_digest: digest.repeat(64) });
  const resources = [{ resource_kind: 'workspace', resource_id: 'same', operations: ['read', 'write'], typed_parameters: { workspace_root: join(pin.fixture, 'workspace') } }];
  const binding = { binding_version: 1, preset_revision_ref: ref('original', 'a'), resolved_snapshot_ref: snapshot('a'), typed_resource_bindings: resources };
  const before = { session: { agent_session_id: pin.sessionId, owner_ref: { principal_kind: 'user', principal_id: owner }, agent_binding: binding }, head: { last_seq: 20 } };
  const after = copy(before); after.session.agent_binding = { ...copy(binding), binding_version: 3, preset_revision_ref: ref('model-derived', 'c'), resolved_snapshot_ref: snapshot('c') };
  const revision = digest => ({ document: { persona: 'same', enabled_capabilities: [{ capability: { id: 'workspace.files' }, action_allowlist: ['workspace.files/read'] }], model_route_refs: {}, chat_route_records: {} }, contribution_locks: [{ contribution_id: 'module:workspace.files', contract_digest: digest.repeat(64) }] });
  const sourceRevision = revision('a'), afterRevision = revision('b');
  const events = [{ seq: 21, kind: 'session/agent-binding-changed', idempotency_key: 'contract-evolution:example', event_id: 'transition', payload: { storage: 'inline_json', value: {
    handoff_mode: 'context_only', completion_gate_inherited: false, request_digest: 'e'.repeat(64),
    previous_binding_ref: { binding_version: 1, preset_revision_ref: ref('original', 'a'), resolved_snapshot_ref: snapshot('a') },
    next_binding_ref: { binding_version: 2, preset_revision_ref: ref('contract-derived', 'b'), resolved_snapshot_ref: snapshot('b') } } } }];
  return { before, after, sourceBinding: copy(binding), sourceRevision, beforeRevision: copy(sourceRevision), afterRevision, events, sourceLastSeq: 20 };
}

test('a model-only binding change cannot masquerade as read schema evolution', () => {
  const value = evolutionFixture();
  expect(nativeContinuationEvolution(value).contract_evolution_observed).toBe(true);
  expect(nativeContinuationEvolution(value).file_materialization_evolution_observed).toBe(true);
  const modelOnly = { ...value, events: [], afterRevision: copy(value.sourceRevision) };
  const proof = nativeContinuationEvolution(modelOnly);
  expect(proof.contract_evolution_observed).toBe(false);
  expect(proof.file_materialization_evolution_observed).toBe(false);
  expect(proof.read_schema_evolution.startsWith('not_proven')).toBe(true);
  const changedGrant = copy(value); changedGrant.afterRevision.document.enabled_capabilities[0].action_allowlist.push('workspace.files/write');
  expect(() => nativeContinuationEvolution(changedGrant)).toThrow();
  const changedResource = copy(value); changedResource.after.session.agent_binding.typed_resource_bindings[0].operations.push('execute');
  expect(() => nativeContinuationEvolution(changedResource)).toThrow();
});

test('the actual CLI parser rejects repeated or incomplete continuation flags before token/API work', () => {
  for (const args of [
    ['--continue-session', pin.sessionId, '--continue-session', pin.sessionId],
    ['--continuation-source-report', pin.sourceReport],
    ['--continue-session', pin.sessionId, '--continuation-source-report', pin.sourceReport, '--native-data-root', pin.nativeDataRoot, '--token-file', '/not-a-token', '--administrator', '--administrator'],
  ]) expect(() => nativeContinuationCliArguments(args)).toThrow('must occur exactly once');
  expect(nativeContinuationCliArguments(['--administrator'])).toBe(false);
  expect(nativeContinuationCliArguments(['--continue-session', pin.sessionId, '--continuation-source-report', pin.sourceReport,
    '--native-data-root', pin.nativeDataRoot, '--token-file', '/not-a-token', '--administrator', '--template', pin.template,
    '--base-url', 'http://127.0.0.1:12345', '--report-dir', '/test-output'])).toBe(true);
});

const retryOperation = `turn:user:${owner}:${pin.sessionId}:0190f5fe-7c00-7a00-8000-000000000009`;
function retryAncestryFixture() {
  const source = nativeContinuationSource(continuationSource(), sourceArgs);
  const admission = { agent_session_id: pin.sessionId, operation_id: retryOperation, message_id: '0190f5fe-7c00-7a00-8000-000000000008' };
  return { source, report: { schema: 'nomifun.global-os-native-regression.v2', status: 'failed', scenario: 'administrator', administrator: true,
    session_id: pin.sessionId, template: pin.template, workspace: join(pin.fixture, 'workspace'), run_id: 'prior-run',
    continuation: { source_report: pin.sourceReport, source_sha256: source.source_sha256, source_operation_id: source.source_operation_id,
      source_events_sha256: source.source_events_sha256, source_owner_id: owner, driver_uid: 501, original_session_id: pin.sessionId, template: pin.template,
      native_app_identity: { native_data_root: pin.nativeDataRoot, bundle_identifier: 'com.nomifun.desktop' },
      read_tool_schema: { administrator_supported: true, default_identity: 'current_user', schema_sha256: 'a'.repeat(64), formats: ['text', 'image', 'instruction_scope'] },
      original_failure_preservation_after_new_turn: { original_operation_preserved: true, source_report_sha256: pin.sourceSha256 } },
    checks: [], observations: {}, remaining_fixture: { administrator_root: false, outside: false }, accepted: admission,
    failure_admission: copy(admission), failed_phase: { role: 'main', accepted: copy(admission) },
    failure_settlement: { cleanup_proven: true, execution: { ...terminal(), operation_id: retryOperation }, head: { session_id: pin.sessionId, status: 'ready', last_seq: 40 },
      events: [{ agent_session_id: pin.sessionId, correlation_id: retryOperation, kind: 'turn/cancelled', seq: 40 }] } } };
}

test('retry requires its exact report pin, original ancestry, UID/dataset and zero verified user steps', () => {
  const { source, report } = retryAncestryFixture(), args = { sourceEvidence: source, driverUid: 501, sha256: pin.retrySha256 };
  expect(nativeContinuationRetryAncestry(report, args).admission.operation_id).toBe(retryOperation);
  expect(() => nativeContinuationRetryAncestry(report, { ...args, sha256: '0'.repeat(64) })).toThrow();
  for (const change of [r => { r.status = 'passed'; }, r => { r.continuation.source_sha256 = '0'.repeat(64); },
    r => { r.session_id = 'other'; }, r => { r.continuation.source_owner_id = 'other'; }, r => { r.continuation.driver_uid = 502; },
    r => { r.continuation.native_app_identity.native_data_root = '/other'; }, r => { r.continuation.retry = {}; },
    r => { r.checks.push({ status: 'passed' }); }, r => { r.observations.actual = {}; }, r => { r.remaining_fixture.administrator_root = true; },
    r => { r.failure_admission.operation_id = sourceOperation; }, r => { r.failure_settlement.execution.unknown_effects = 1; },
    r => { r.failure_settlement.cleanup_proven = false; }]) {
    const invalid = copy(report); change(invalid); expect(() => nativeContinuationRetryAncestry(invalid, args)).toThrow();
  }
  // The combined guard also pins the exact real canonical event bytes; metadata alone cannot authorize retry.
  expect(() => nativeContinuationRetrySource(report, args)).toThrow('canonical source changed');
  const options = { administrator: true, scenario: 'files-process', template: pin.template, continueSession: pin.sessionId,
    continuationSourceReport: pin.sourceReport, nativeDataRoot: pin.nativeDataRoot };
  expect(nativeContinuationOptions({ ...options, continuationRetryReport: pin.retryReport })).toBe(true);
  expect(() => nativeContinuationOptions({ ...options, continuationRetryReport: '/other-failed.json' })).toThrow();
  expect(() => nativeContinuationOptions({ continuationRetryReport: pin.retryReport })).toThrow();
});

function zeroDispatchFixture() {
  const runId = 'prior-run', callId = nativeRunCallId(runId, 'admin-fixture');
  const event = value => ({ agent_session_id: pin.sessionId, correlation_id: retryOperation, kind: 'runtime/progress-recorded', payload: { storage: 'inline_json', value: { event: value } } });
  const events = [];
  for (const id of ['agent-instructions:100', 'agent-instructions:101', 'agent-instructions:102']) {
    events.push(event({ event: 'tool_call_completed', step: 0, call: { call_id: id, name: 'read_file' } }));
    events.push(event({ event: 'host_tool_dispatch', dispatch: { operation_id: `${retryOperation}:tool:${id}`, call_id: id,
      capability_id: 'workspace.files', action_id: 'workspace.files/read', model_name: 'read_file' } }));
  }
  events.push(event({ event: 'tool_call_completed', step: 1, call: { call_id: callId, name: 'exec_command', arguments: { execution_identity: 'administrator' } } }));
  events.push(event({ event: 'tool_completed', step: 1, runtime_hold: 'runtime_gate', result: { call_id: callId, is_error: true } }));
  return { events, runId, callId, event };
}

test('zero checks alone never authorize opaque/write/resource dispatch replay', () => {
  const { events, runId, callId, event } = zeroDispatchFixture();
  expect(nativeContinuationZeroUserDispatch(events, { operationId: retryOperation, runId }).actual_user_effect_dispatch_count).toBe(0);
  const unsafe = [
    [...events, event({ event: 'host_tool_dispatch', dispatch: { call_id: callId, capability_id: 'workspace.process', action_id: 'workspace.process/exec' } })],
    [...events, event({ event: 'host_resource_dispatch', dispatch: { call_id: callId } })],
    [...events, event({ event: 'tool_started', step: 1, call_id: callId })],
    [...events, { kind: 'effect/started' }],
  ];
  const write = copy(events); write[1].payload.value.event.dispatch.action_id = 'workspace.files/write'; unsafe.push(write);
  const noHold = copy(events); delete noHold.at(-1).payload.value.event.runtime_hold; unsafe.push(noHold);
  for (const invalid of unsafe) expect(() => nativeContinuationZeroUserDispatch(invalid, { operationId: retryOperation, runId })).toThrow();
});

test('retry admission distinguishes historical original from the exact latest settled retry', () => {
  const args = { execution: { ...terminal(), operation_id: retryOperation }, head: { session_id: pin.sessionId, status: 'ready' }, operationId: retryOperation, sessionId: pin.sessionId };
  expect(nativeContinuationSettlement(args).operation_id).toBe(retryOperation);
  expect(() => nativeContinuationSettlement({ ...args, execution: terminal() })).toThrow();
  expect(nativeContinuationCancellationTarget({ latestOperationId: retryOperation, admissionOperationId: retryOperation })).toBe('prior_settled_not_admitted');
  expect(() => nativeContinuationCancellationTarget({ latestOperationId: sourceOperation, admissionOperationId: retryOperation })).toThrow();
  expect(() => nativeContinuationCancellationTarget({ latestOperationId: 'another-owner-turn', admissionOperationId: retryOperation })).toThrow();
  expect(nativeContinuationCancellationTarget({ latestOperationId: 'new-owned-turn', admissionOperationId: retryOperation, pendingAdmissionOperationId: 'new-owned-turn' })).toBe('current_run');
});

test('already prepared ancestor schema is not falsely reported as a new retry model-command evolution', () => {
  const value = evolutionFixture();
  value.before.head.last_seq = 40;
  value.before.session.agent_binding.binding_version = 3;
  value.after.session.agent_binding.binding_version = 4;
  value.beforeRevision = copy(value.afterRevision);
  const proof = nativeContinuationEvolution(value);
  expect(proof.ancestor_contract_evolution_observed).toBe(true);
  expect(proof.contract_evolution_during_model_command).toBe(false);
  expect(proof.file_materialization_changed_during_model_command).toBe(false);
  expect(proof.read_schema_evolution).toContain('no_new_read_schema_evolution_claimed');
});
