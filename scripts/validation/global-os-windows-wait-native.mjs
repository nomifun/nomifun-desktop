#!/usr/bin/env bun
// Disposable scripted provider for a real, already-running Windows App.
// Gate writes are external fixture control, never a human answer, OS approval,
// canonical Effect, or authority to reconcile/replay an unknown operation.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { createServer } from 'node:http';
import { execFile } from 'node:child_process';
import { createReadStream } from 'node:fs';
import { lstat, mkdir, readFile, realpath, readdir, rmdir, unlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { promisify } from 'node:util';

const ACK = 'Input request recorded. This is a pending input handoff, not a user answer or task-completion receipt.';
const delay = ms => new Promise(done => setTimeout(done, ms));
const execute = promisify(execFile);
const stable = value => JSON.stringify(value, (_, item) => item && typeof item === 'object' && !Array.isArray(item)
  ? Object.fromEntries(Object.keys(item).sort().map(key => [key, item[key]])) : item);
const hash = value => createHash('sha256').update(value).digest('hex');
const progress = (events, operation) => events.filter(event => event.correlation_id === operation && event.kind === 'runtime/progress-recorded')
  .map(record => ({ record, seq: record.seq, event: record.payload?.value?.event }));
const modelSteps = (events, operation) => progress(events, operation).filter(row => row.event?.event === 'model_step_started').length;
const absent = path => lstat(path).then(() => false, error => { if (error.code === 'ENOENT') return true; throw error; });
const rawTool = message => typeof message.content === 'string' ? message.content : JSON.stringify(message.content);
function toolValue(raw) { assert(!raw.startsWith('[tool error]'), 'A real tool refusal is not a successful fixture observation'); return JSON.parse(raw); }
function slot(record, pointer, kind, expected, session) {
  assert.equal(record.agent_session_id, session);
  const envelope = record.payload?.value;
  const value = pointer.slice(1).split('/').reduce((item, key) => item?.[key], envelope);
  if (value !== null && value !== undefined) { assert.deepEqual(value, expected); return; }
  const shared = envelope?._canonical_shared_content;
  assert.equal(shared?.version, 1);
  const refs = shared.references.filter(ref => ref.pointer === pointer); assert.equal(refs.length, 1);
  const ref = refs[0], bytes = Buffer.from(stable(expected));
  assert.equal(ref.kind, kind); assert.equal(ref.digest, hash(bytes)); assert.equal(ref.byte_len, bytes.length);
  assert.equal(ref.payload_id, `shared-tool:${session}:${kind === 'tool_arguments' ? 'arguments' : 'output'}:${ref.digest}`);
}
function one(values, label) { assert.equal(values.length, 1, `Expected one exact ${label}`); return values[0]; }
function handoffProof({ events, operation, session, accepted, requestId, plan, projection, processId, pid }) {
  assert(events.every(event => event.agent_session_id === session), 'Handoff evidence belongs to the exact Session');
  assert.equal(plan.conversation_id, session); assert.equal(plan.turn_id, accepted.message_id);
  assert.equal(plan.turn_status, 'completed'); assert.equal(plan.turn_outcome?.kind, 'input_required');
  assert.equal(plan.turn_outcome.request_id, requestId);
  assert.equal(projection.runtime?.state, 'idle'); assert.equal(projection.runtime.active_turn_id, null);
  assert.equal(projection.runtime.can_send_message, true); assert.equal(projection.runtime.is_processing, false);
  assert(!Object.hasOwn(projection.runtime, 'can_steer'), 'Retired input-routing fields cannot return');
  const rows = progress(events, operation), current = events.filter(event => event.correlation_id === operation);
  const started = one(current.filter(event => event.kind === 'turn/started'), 'original Turn start');
  assert.equal(started.payload?.value?.source_message_id, accepted.message_id);
  const requested = one(rows.filter(row => row.event?.event === 'user_input_requested' && row.event.request?.request_id === requestId), 'original input request');
  const handoff = one(rows.filter(row => row.event?.event === 'turn_input_required' && row.event.request?.request_id === requestId), 'finite input handoff');
  const cleanup = one(rows.filter(row => row.event?.event === 'host_cleanup_proven'), 'Turn cleanup with Session process retention');
  const retained = cleanup.event.session_process_retention;
  assert.equal(retained?.agent_session_id, session); assert.match(retained?.owner_generation, /^[0-9a-f-]{36}$/);
  const process = one(retained.processes.filter(item => item.process_id === processId), 'same retained background process');
  assert.equal(process.pid, pid); assert.equal(process.creator_turn_id, operation);
  const terminal = one(current.filter(event => ['turn/completed', 'turn/cancelled', 'turn/failed'].includes(event.kind)), 'finite canonical terminal');
  assert.equal(terminal.kind, 'turn/completed');
  assert(started.seq < requested.seq && requested.seq < handoff.seq && handoff.seq < cleanup.seq && cleanup.seq < terminal.seq);
  assert(!rows.some(row => ['turn_waiting', 'turn_wait_resolved', 'turn_failed', 'turn_paused', 'turn_cancelled'].includes(row.event?.event)),
    'A human handoff closes computation and must not park or fail its original Turn');
  return { request_seq: requested.seq, handoff_seq: handoff.seq, cleanup_seq: cleanup.seq, terminal_seq: terminal.seq,
    request_id: requestId, model_step_count: modelSteps(events, operation), turn_id: accepted.message_id, operation_id: operation,
    retained_process: process, owner_generation: retained.owner_generation };
}
function continuedProcessProof({ events, operation, initialOperation, session, accepted, call, output, processId, cancelled }) {
  assert(events.every(event => event.agent_session_id === session), 'Continuation evidence belongs to the exact Session');
  assert.notEqual(operation, initialOperation, 'A normal reply starts a distinct computational Turn');
  const rows = progress(events, operation), started = one(events.filter(event => event.correlation_id === operation && event.kind === 'turn/started'), 'reply Turn claim');
  assert.equal(started.payload?.value?.source_message_id, accepted.message_id);
  assert.equal(call.args.process_id, processId); assert.equal(call.name, cancelled ? 'cancel_process' : 'poll_process');
  const proposed = one(rows.filter(row => row.event?.event === 'tool_call_completed' && row.event.call?.call_id === call.id), 'same-handle control proposal');
  assert.equal(proposed.event.call.name, call.name); slot(proposed.record, '/event/call/arguments', 'tool_arguments', call.args, session);
  const completed = one(rows.filter(row => row.event?.event === 'tool_completed' && row.event.result?.call_id === call.id), 'actual process control receipt');
  assert.equal(completed.event.result.is_error, false); slot(completed.record, '/event/result/output', 'tool_output', [{ type: 'text', text: output }], session);
  const value = toolValue(output); assert.equal(value.process_id, processId); assert.equal(value.state, cancelled ? 'cancelled' : 'exited');
  if (!cancelled) assert.equal(value.exit_code, 0);
  assert.equal(value.cleanup?.reaped, true);
  if (cancelled) assert(Array.isArray(value.cleanup.errors) && value.cleanup.errors.every(item => typeof item === 'string'));
  else assert.deepEqual(value.cleanup.errors, []);
  assert(!rows.some(row => row.event?.event === 'tool_started' && row.event.action_id === 'workspace.process/start'), 'The reply cannot relaunch the original command');
  assert(!rows.some(row => ['turn_wait_resolved', 'steering_inputs'].includes(row.event?.event)), 'The reply is a new input, not a synthetic old-Turn wake');
  return { source_message_id: accepted.message_id, operation_id: operation, initial_operation_id: initialOperation,
    control_call_id: call.id, proposed_seq: proposed.seq, completed_seq: completed.seq, actual_result: value };
}
function uniqueLaunchProof({ events, operation, session, call, output }) {
  assert(events.every(event => event.agent_session_id === session), 'Effect and Runtime evidence must belong to the exact Session');
  const rows = progress(events, operation);
  const started = one(rows.filter(row => row.event?.event === 'tool_started' && row.event.action_id === 'workspace.process/start'), 'dispatched process start');
  assert.equal(started.event.call_id, call.id);
  const proposed = one(rows.filter(row => row.event?.event === 'tool_call_completed' && row.event.call?.call_id === call.id), 'accepted start proposal');
  assert.equal(proposed.event.call.name, 'start_process'); slot(proposed.record, '/event/call/arguments', 'tool_arguments', call.args, session);
  const completed = one(rows.filter(row => row.event?.event === 'tool_completed' && row.event.result?.call_id === call.id), 'actual start tool result');
  assert.equal(completed.event.result.is_error, false); slot(completed.record, '/event/result/output', 'tool_output', [{ type: 'text', text: output }], session);
  const effect = one(events.filter(event => event.kind === 'effect/started' && event.payload?.value?.operation_id === `${operation}:tool:${call.id}`), 'original start Effect');
  const original = effect.payload.value, effectId = original.effect_id;
  assert.equal(effect.correlation_id, effectId, 'Effect correlation follows its own immutable identity, not the Runtime operation');
  assert.equal(original.turn_id, operation); assert.equal(original.owner_domain, 'workspace');
  assert.equal(original.capability_module, 'workspace.process'); assert.equal(original.action_id, 'workspace.process/start');
  assert.match(original.input_digest, /^[0-9a-f]{64}$/);
  const terminal = one(events.filter(event => event.payload?.value?.effect_id === effectId && event.kind !== 'effect/started'), 'start Effect settlement');
  assert.equal(terminal.kind, 'effect/succeeded'); assert.equal(terminal.correlation_id, effectId);
  assert.equal(terminal.causation_event_id, effect.event_id);
  for (const key of ['strategy', 'effect_id', 'turn_id', 'operation_id', 'owner_domain', 'capability_module', 'action_id', 'input_digest', 'resource_binding_id', 'resource_key'])
    assert.deepEqual(terminal.payload.value[key], original[key], 'Effect settlement must preserve its exact admitted input and resource identity');
  assert.deepEqual(terminal.payload.value.result, toolValue(output), 'The start Effect result must match its actual canonical tool observation');
  assert(proposed.seq < started.seq && started.seq < effect.seq && terminal.seq > effect.seq && completed.seq > terminal.seq);
  assert(!events.some(event => event.kind === 'effect/uncertain' || event.kind === 'effect/reconciled'), 'Unknown/reconciled effects cannot qualify for an automatic fixture retry');
  return { invocation_count: 1, call_id: call.id, proposal_seq: proposed.seq, tool_started_seq: started.seq,
    effect_id: effectId, effect_started_seq: effect.seq, effect_terminal_seq: terminal.seq, tool_completed_seq: completed.seq };
}
function cancelledOwnerCleanupProof({ events, operation, session, accepted, execution, cancellation }) {
  assert(events.every(event => event.agent_session_id === session), 'Cancellation observations must belong to the exact Session');
  if (!cancellation) return undefined;
  assert.equal(cancellation.agent_session_id, session); assert.equal(cancellation.target_operation_id, operation);
  assert.equal(cancellation.status, 'cancelled'); assert.equal(cancellation.duplicate, false);
  assert.equal(cancellation.cursor?.agent_session_id, session);
  assert.equal(execution.operation_id, operation); assert.equal(execution.state, 'cancelled');
  assert.equal(execution.pending_effects, 0); assert.equal(execution.unknown_effects, 0);
  assert.equal(execution.producer_lease_live, false); assert.equal(execution.checkpoint_retained, false);
  assert.equal(execution.automatic_replay_authorized, false);
  assert(Number.isSafeInteger(execution.execution_generation) && execution.execution_generation > 0);
  const current = events.filter(event => event.correlation_id === operation), rows = progress(events, operation);
  const started = one(current.filter(event => event.kind === 'turn/started'), 'cancelled Turn start');
  assert.equal(started.payload?.value?.source_message_id, accepted.message_id);
  const terminals = current.filter(event => ['turn/completed', 'turn/cancelled', 'turn/failed'].includes(event.kind));
  if (!terminals.length) return undefined;
  const cancelled = one(terminals, 'Session-owner cancellation');
  assert.equal(cancelled.kind, 'turn/cancelled'); assert.equal(cancelled.producer_id, 'session_api');
  assert.equal(cancelled.event_id, cancellation.message_id); assert.equal(cancelled.seq, cancellation.cursor.seq);
  assert.equal(cancelled.payload?.value?.target_operation_id, operation); assert(cancelled.seq > started.seq);
  const cleanups = rows.filter(row => row.event?.event === 'host_cleanup_proven');
  const closed = rows.filter(row => row.event?.event === 'turn_cancelled');
  assert(closed.length <= 1, 'The cancelled Runtime must have one exact final closure');
  if (!cleanups.length) {
    assert.equal(closed.length, 0, 'A Runtime cancellation closure without its cleanup witness is invalid');
    return undefined;
  }
  const cleanup = one(cleanups, 'same-owner cancellation cleanup');
  assert.equal(cleanup.record.producer_id, 'runtime_supervisor'); assert.equal(cleanup.event.operation, operation);
  assert.equal(cleanup.event.epoch, execution.execution_generation);
  const original = one(rows.filter(row => row.event?.event === 'turn_started'), 'original Runtime binding');
  assert(original.seq < cancelled.seq); assert.equal(original.event.turn_operation_id, operation);
  assert.match(original.event.binding?.build_digest, /^[0-9a-f]{64}$/);
  assert.equal(cleanup.event.binding?.build_id, original.event.binding.build_id);
  assert.equal(cleanup.event.binding?.build_digest, original.event.binding.build_digest);
  assert(cleanup.seq > cancelled.seq, 'Owner cancellation commits before this original owner finishes native cleanup');
  if (!closed.length) return undefined;
  const runtime = closed[0]; assert.equal(runtime.record.producer_id, 'runtime_supervisor');
  assert(runtime.seq > cleanup.seq, 'The Runtime closure must follow its actual cleanup witness');
  assert.equal(runtime.event.model_steps, execution.model_progress?.used_model_steps);
  return { cancellation_event_id: cancelled.event_id, cancellation_seq: cancelled.seq,
    cleanup_seq: cleanup.seq, runtime_cancelled_seq: runtime.seq, execution_generation: execution.execution_generation,
    original_build_id: original.event.binding.build_id, original_build_digest: original.event.binding.build_digest };
}
function terminalDispositionProof({ events, operation, session, accepted, plan, cancelled, completionMode, directAnswers = 0, execution, cancellation }) {
  assert(events.every(event => event.agent_session_id === session && event.correlation_id === operation),
    'Only the exact accepted operation window can supply terminal evidence');
  const started = one(events.filter(event => event.kind === 'turn/started'), 'canonical Turn start');
  assert.equal(started.payload?.value?.source_message_id, accepted.message_id);
  assert.equal(plan.conversation_id, session); assert.equal(plan.turn_id, accepted.message_id);
  assert.equal(plan.turn_status, cancelled ? 'cancelled' : 'completed');
  assert.equal(plan.turn_outcome?.kind, cancelled ? 'cancelled' : 'answered');
  if (!cancelled) assert(Array.isArray(plan.turn_outcome.message_ids) && plan.turn_outcome.message_ids.length > 0,
    'A completed operation without a sealed public answer is not this successful task outcome');
  const terminal = one(events.filter(event => ['turn/completed', 'turn/cancelled', 'turn/failed'].includes(event.kind)), 'irreversible canonical Turn terminal');
  assert.equal(terminal.kind, cancelled ? 'turn/cancelled' : 'turn/completed'); assert(terminal.seq > started.seq);
  const rows = progress(events, operation);
  const cleanup = one(rows.filter(row => row.event?.event === 'host_cleanup_proven'), 'canonical host cleanup proof');
  assert(cleanup.seq > started.seq);
  if (!cancelled) assert(cleanup.seq < terminal.seq);
  const proof = { started_seq: started.seq, terminal_seq: terminal.seq, cleanup_seq: cleanup.seq,
    exact_turn_id: accepted.message_id, exact_operation_id: operation, turn_outcome: plan.turn_outcome };
  if (cancelled) {
    const closed = cancelledOwnerCleanupProof({ events, operation, session, accepted, execution, cancellation });
    assert(closed, 'A logical cancellation alone does not prove the original native owner finished cleanup');
    proof.cancelled_owner_cleanup = closed;
    assert(!rows.some(row => ['completion_reported', 'completion_delivered'].includes(row.event?.event)),
      'Exact owner cancellation must not be projected as task completion');
  } else if (completionMode === 'report_completion') {
    const reports = rows.filter(row => row.event?.event === 'completion_reported'); assert(reports.length > 0);
    const reported = reports.at(-1), report = reported.event.report;
    assert(Array.isArray(report?.criteria) && report.criteria.length > 0 && report.criteria.every(item => item.disposition === 'supported'),
      'The final formal completion account must have no unresolved criterion');
    const delivered = one(rows.filter(row => row.event?.event === 'completion_delivered'), 'canonical formal completion delivery');
    assert(reported.seq < delivered.seq && delivered.seq < cleanup.seq);
    Object.assign(proof, { completion_contract: completionMode, completion_reported_seq: reported.seq,
      completion_delivered_seq: delivered.seq, final_completion_report: report });
  } else {
    assert.equal(completionMode, 'direct_answer_non_ledger'); assert(directAnswers > 0);
    assert(!rows.some(row => row.event?.event === 'tool_call_completed' && ['report_completion', 'update_plan'].includes(row.event.call?.name)),
      'An advertised or used accounting gate cannot be bypassed by a direct answer');
    proof.completion_contract = completionMode;
  }
  return proof;
}
function parseOptions(argv) {
  if (argv.length === 1 && argv[0] === '--self-test') return { selfTest: true };
  const known = new Set(['--execute', '--fixture-report', '--report-root', '--repo-root', '--expected-source-head', '--expected-exe-sha256']);
  const values = {};
  for (let index = 0; index < argv.length; index++) {
    const flag = argv[index]; assert(known.has(flag) && !Object.hasOwn(values, flag), 'Unknown or duplicate CLI option');
    if (flag === '--execute') values[flag] = true;
    else { assert(argv[index + 1] && !argv[index + 1].startsWith('--')); values[flag] = argv[++index]; }
  }
  assert.equal(values['--execute'], true, 'Actual native work requires the explicit --execute flag; --self-test is read-only pure verification');
  for (const flag of known) if (flag !== '--execute') assert.equal(typeof values[flag], 'string', `${flag} is required`);
  assert.match(values['--expected-source-head'], /^[0-9a-f]{40}$/); assert.match(values['--expected-exe-sha256'], /^[0-9a-f]{64}$/);
  return values;
}
async function executableHash(path) { const digest = createHash('sha256'); for await (const bytes of createReadStream(path)) digest.update(bytes); return digest.digest('hex'); }
async function run(options) {
  assert.equal(process.platform, 'win32', 'This entry is native Windows acceptance only');
  const repo = await realpath(resolve(options['--repo-root']));
  const load = name => import(pathToFileURL(join(repo, 'scripts', 'validation', name)).href);
  const common = await load('global-os-native.mjs');
  const windows = await load('global-os-windows-cases.mjs');
  const { windowsIntegrityObservationScript } = await load('global-os-windows-desktop-cases.mjs');
  const { beginWindowsProcessExitObservation } = await load('global-os-windows-process-exit-observer.mjs');
  const source = (await execute('git', ['rev-parse', 'HEAD'], { cwd: repo, windowsHide: true })).stdout.trim();
  assert.equal(source, options['--expected-source-head'], 'The checkout must match the explicitly selected merged source anchor');
  const dirty = (await execute('git', ['diff', '--name-only', 'HEAD'], { cwd: repo, windowsHide: true })).stdout.trim().split(/\r?\n/).filter(Boolean);
  const untracked = (await execute('git', ['ls-files', '--others', '--exclude-standard'], { cwd: repo, windowsHide: true })).stdout.trim().split(/\r?\n/).filter(Boolean);
  assert([...dirty, ...untracked].every(path => path.startsWith('docs/') || path.startsWith('scripts/validation/')),
    'Uncommitted production inputs cannot inherit the selected source anchor');
  const readyPath = await realpath(resolve(options['--fixture-report']));
  const readyBytes = await readFile(readyPath), ready = JSON.parse(readyBytes);
  assert.equal(ready.schema, 'nomifun.windows-global-os-fixture.v1'); assert.equal(ready.status, 'ready');
  assert.equal(ready.exe_sha256, options['--expected-exe-sha256']);
  const data = await common.validateNativeDataset(ready.native_data_root);
  assert.equal(ready.parent, data.parent); assert.equal(ready.token_file, join(data.parent, 'owner-jwt'));
  const token = (await windows.readWindowsOwnedFixtureBytes(ready.token_file, 4096)).toString('utf8').trim();
  assert(token.split('.').length === 3, 'Expected the private real owner login JWT');
  const claims = JSON.parse(Buffer.from(token.split('.')[1], 'base64url').toString('utf8'));
  assert(typeof claims.user_id === 'string' && claims.user_id.length > 0);
  const announcement = await common.readNativeAnnouncement(data);
  assert.equal(announcement.pid, ready.app_pid);
  const origin = new URL(ready.base_url); assert.equal(origin.protocol, 'http:');
  assert(['127.0.0.1', 'localhost', '[::1]'].includes(origin.hostname) && !origin.username && !origin.password);
  assert.equal(Number(origin.port), announcement.port);
  const login = await windows.windowsLoginIdentity();
  const appNative = windows.windowsProcessObservation(ready.app_pid, { includeImagePath: true });
  assert.equal(appNative.state, 'running');
  assert.equal(appNative.creation_filetime, ready.actual_app_identity?.native_process?.creation_filetime);
  assert.equal((await realpath(appNative.image_path)).toLowerCase(), ready.exe.toLowerCase());
  assert.equal(await executableHash(ready.exe), options['--expected-exe-sha256']);
  assert.equal(await windows.windowsObservation(windowsIntegrityObservationScript(ready.app_pid)), 8192);
  assert.equal(ready.actual_app_identity.sid, login.sid); assert.equal(ready.actual_app_identity.session_id, login.session_id);
  let dispatchedMutation = false;
  const api = async (method, path, body) => {
    assert(path.startsWith('/api/') && !path.includes('..'));
    if (method !== 'GET') dispatchedMutation = true;
    const response = await fetch(new URL(path, origin), { method, headers: { Authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(30000) });
    assert(response.ok, `Native API request ${method} ${path} returned HTTP ${response.status}; private body omitted`);
    const bytes = await response.arrayBuffer(); assert(bytes.byteLength <= 16 * 1024 * 1024, 'Native response exceeded evidence budget');
    const value = JSON.parse(Buffer.from(bytes).toString('utf8')); assert.notEqual(value.success, false, 'Native API returned a failed envelope'); return value.data ?? value;
  };
  const caller = await api('GET', '/api/system/permissions'); assert.equal(caller.platform, 'windows'); assert.equal(caller.process_id, ready.app_pid);
  const reportRoot = await realpath(resolve(options['--report-root'])); assert((await lstat(reportRoot)).isDirectory());
  const runId = randomUUID(), fixture = join(await realpath(tmpdir()), `nomifun-global-os-product-${runId}`), workspace = join(fixture, 'workspace');
  assert(await absent(fixture)); await mkdir(fixture);
  const fixtureAcl = await windows.initializeWindowsPrivateEmptyFixture(fixture, login); await mkdir(workspace);
  const reportPath = join(reportRoot, `current-main-wait-native-${runId}.json`);
  const report = { schema: 'nomifun.windows-current-main-wait-native.v2', status: 'running', run_id: runId, source_head: source,
    actual_exe_sha256: options['--expected-exe-sha256'], fixture_report: readyPath, fixture_report_sha256: hash(readyBytes),
    app_pid: ready.app_pid, app_creation_filetime: appNative.creation_filetime, native_data_root: data.root,
    login_identity: login, fixture, workspace, fixture_acl: fixtureAcl, started_at: new Date().toISOString(), cases: [],
    evidence_scope: 'Real native App/default assistant.general/public canonical API/local scripted provider. External UUID gate writes are test infrastructure, never human answers, OS authentication or canonical task effects. No live-model reasoning claim.',
    automatic_replay_authorized: false, reconciliation_submitted: false, host_process_signals_sent: false };
  let sessionId, active, providerFailure, providerCount = 0, server, interrupted = false;
  const interrupt = () => { interrupted = true; };
  process.once('SIGINT', interrupt); process.once('SIGTERM', interrupt);
  const eventsFor = async () => {
    const result = []; let after = 0;
    for (let page = 0; page < 24; page++) {
      const value = await api('GET', `/api/agent-sessions/${sessionId}/events?after_seq=${after}&limit=500`);
      for (const event of value.events) { assert.equal(event.agent_session_id, sessionId); assert(event.seq > after); result.push(event); }
      const next = value.next_cursor.seq; if (!value.events.length || next <= after) return result; after = next;
    }
    throw new Error('Canonical evidence exceeded bounded pagination');
  };
  const checkpoint = async () => { await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`); };
  const currentFacts = async () => ({ session: await api('GET', `/api/agent-sessions/${sessionId}`),
    execution: await api('GET', `/api/agent-sessions/${sessionId}/execution`), events: await eventsFor() });
  const requireKnownSettlement = async (expected, deadline = Date.now() + 90000) => {
    while (Date.now() < deadline) {
      if (providerFailure) throw providerFailure;
      const facts = await currentFacts(), execution = facts.execution;
      assert.equal(execution?.operation_id, active.accepted.operation_id);
      if (execution.pending_effects > 0 || execution.unknown_effects > 0) {
        if (!['running'].includes(execution.state)) throw new Error('Original native result remains unsettled; do not replay or erase this fixture');
      }
      if (execution.state === expected && !facts.session.head.active_turn_id && execution.pending_effects === 0 && execution.unknown_effects === 0) {
        if (expected === 'cancelled' && !cancelledOwnerCleanupProof({ events: facts.events, operation: active.accepted.operation_id,
          session: sessionId, accepted: active.accepted, execution, cancellation: active.cancellation_receipt })) {
          // The Session-owner receipt closes admission first. Keep observing
          // its original native owner until cleanup and Runtime closure exist.
          await delay(250); continue;
        }
        assert(progress(facts.events, active.accepted.operation_id).some(row => row.event?.event === 'host_cleanup_proven'));
        return facts;
      }
      assert(['running', expected].includes(execution.state), 'Unexpected paused/failed/unknown state requires investigation, never automatic resume');
      await delay(250);
    }
    throw new Error('Canonical terminal/cleanup was not established within the bounded fixture deadline');
  };
  const submitMessage = async content => {
    const body = { idempotency_key: randomUUID(), input: { content } };
    active.accepted = undefined;
    const accepted = await api('POST', `/api/agent-sessions/${sessionId}/messages`, body);
    assert.equal(accepted.agent_session_id, sessionId); assert.equal(accepted.status, 'accepted');
    const replay = await api('POST', `/api/agent-sessions/${sessionId}/messages`, body);
    assert.equal(replay.message_id, accepted.message_id); assert.equal(replay.replayed, true);
    const deadline = Date.now() + 60000;
    while (Date.now() < deadline) {
      const claims = (await eventsFor()).filter(event => event.kind === 'turn/started' && event.payload?.value?.source_message_id === accepted.message_id);
      if (claims.length) { const claim = one(claims, 'durable input claim'); return { ...accepted, operation_id: claim.correlation_id }; }
      await delay(50);
    }
    throw new Error('Saved input was not claimed by a distinct Turn within the bounded test observation');
  };
  const unchangedComputation = async (original, providerBefore, milliseconds = 1500) => {
    const before = Date.now(); await delay(milliseconds);
    assert.equal(providerCount, providerBefore, 'Background output or exit must not create a model request');
    const events = await eventsFor(); assert.equal(modelSteps(events, original.operation_id), original.model_step_count);
    assert(!events.some(event => event.seq > original.terminal_seq && event.kind === 'turn/started'), 'No new Turn without a newly saved user message');
    assert(!progress(events, original.operation_id).some(row => row.seq > original.terminal_seq), 'The original terminal computation cannot restart');
    assert(!(await api('GET', `/api/agent-sessions/${sessionId}`)).head.active_turn_id);
    return { observation_ms: Date.now() - before, provider_requests: providerCount, old_model_step_count: original.model_step_count };
  };
  const ensureLiveIdentity = async value => {
    assert.equal(value.state, 'running'); assert.equal(value.process_id, active.processId); assert.equal(value.pid, active.pid);
    active.output += value.output.text; active.cursor = value.output.next_cursor;
    if (!active.output.includes('NOMIFUN_WAIT_READY\n') && !active.output.includes('NOMIFUN_WAIT_READY\r\n')) return false;
    const lines = active.output.replaceAll('\r\n', '\n').trim().split('\n'); assert.equal(lines.length, 2); assert.equal(lines[1], 'NOMIFUN_WAIT_READY');
    const identity = JSON.parse(lines[0]); assert.equal(identity.pid, active.pid); assert.equal(identity.sid, login.sid);
    assert.equal(identity.elevated, active.administrator); assert.equal(identity.session_id, login.session_id);
    assert.equal(identity.profile.toLowerCase(), login.profile.toLowerCase());
    const processNative = windows.windowsProcessObservation(active.pid, { includeImagePath: true }); assert.equal(processNative.state, 'running');
    assert.equal((await realpath(processNative.image_path)).toLowerCase(), windows.powershell.toLowerCase());
    const integrity = await windows.windowsObservation(windowsIntegrityObservationScript(active.pid));
    assert.equal(integrity, active.administrator ? 12288 : 8192);
    const owner = await windows.windowsObservation(`$p=Get-CimInstance Win32_Process -Filter 'ProcessId=${active.pid}';if($null-eq$p){throw 'Expected original process'};$o=Invoke-CimMethod -InputObject $p -MethodName GetOwnerSid;[pscustomobject]@{pid=$p.ProcessId;sid=$o.Sid;return_value=$o.ReturnValue;session_id=$p.SessionId}|ConvertTo-Json -Compress`);
    assert.equal(owner.return_value, 0); assert.equal(owner.sid, login.sid); assert.equal(owner.session_id, login.session_id);
    active.independent_identity = { native: processNative, integrity_rid: integrity, owner, process_output: identity };
    active.observer = await beginWindowsProcessExitObservation({ pid: active.pid, creationFiletime: processNative.creation_filetime, timeoutMs: 60000 });
    active.retained_observer_ready = active.observer.ready;
    return true;
  };
  const recordRaw = async (call, raw, messages) => {
    assert.equal(call.case_id, active.id, 'A prior Turn result must not settle this Turn');
    call.raw = raw;
    if (raw.startsWith('[tool error]')) {
      assert.equal(call.kind, 'start', 'Only a proven pre-dispatch instruction/path hold may receive a fresh proposal');
      const events = await eventsFor();
      const proof = common.nativeInstructionDeferral({ raw, callId: call.id, callName: call.name, callArguments: call.args, events,
        operationId: active.accepted.operation_id, sessionId });
      const redirect = proof ? undefined : common.nativeCanonicalPathReconsideration({ raw, callId: call.id, callArguments: call.args, events,
        operationId: active.accepted.operation_id, sessionId });
      assert(proof || redirect, 'A dispatched, unknown or ordinary failed start cannot be replayed');
      assert(!active.processId); assert(++active.deferrals <= 4);
      if (redirect) { assert.equal(redirect.requested, active.launch.cwd); assert.equal((await realpath(redirect.canonical)).toLowerCase(), workspace.toLowerCase()); active.launch.cwd = redirect.canonical; }
      active.preflight_deferrals.push(proof ?? redirect); return;
    }
    if (call.kind === 'question') { assert.equal(raw, ACK); active.question_ack = { call_id: call.id, text: raw }; return; }
    if (call.kind === 'completion') {
      assert(raw.startsWith('Completion account recorded,') || raw.startsWith('Candidate account recorded,'));
      if (raw.startsWith('Completion account recorded,')) active.completionAccepted = true; return;
    }
    const value = toolValue(raw);
    if (call.kind === 'discover') { assert(Array.isArray(value) && value.some(item => item.name === call.query)); return; }
    if (call.kind === 'plan') { assert(['updated', 'unchanged'].includes(value.status)); assert.equal(value.needs_replan, false); return; }
    if (call.kind === 'start') {
      assert(!active.processId); assert.equal(value.state, 'running'); assert(typeof value.process_id === 'string'); assert(Number.isSafeInteger(value.pid) && value.pid > 1);
      active.processId = value.process_id; active.pid = value.pid; active.cursor = value.output.next_cursor; active.output = value.output.text;
      active.actual_start_call = call; active.actual_start_result = value; active.stage = 'ready_poll';
    } else if (call.kind === 'ready_poll') {
      if (await ensureLiveIdentity(value)) active.stage = 'question';
    } else if (call.kind === 'terminal_poll' || call.kind === 'process_cancel') {
      assert(active.reply_admitted); if (call.kind === 'terminal_poll') assert(active.gate_written); assert.equal(value.process_id, active.processId);
      assert.equal(value.state, call.kind === 'process_cancel' ? 'cancelled' : 'exited');
      if (call.kind === 'terminal_poll') { assert.equal(value.exit_code, 0); assert(value.output.text.includes(`NOMIFUN_WAIT_CONFIRMED:${active.nonce}`)); }
      assert.equal(value.cleanup?.reaped, true);
      if (call.kind === 'process_cancel') assert(Array.isArray(value.cleanup.errors) && value.cleanup.errors.every(item => typeof item === 'string'));
      else assert.deepEqual(value.cleanup.errors, []);
      assert.equal(value.output.dropped_bytes, 0); assert.equal(value.output.decode_errors, 0);
      active.terminal_control = { call_id: call.id, result: value }; active.stage = 'completion';
    } else throw new Error('Unexpected provider control kind');
    active.evidence_ids.add(call.id);
  };
  const selectCall = async body => {
    assert(active?.accepted, 'No provider request is admitted without the exact public Turn receipt');
    if (active.question_id) assert(active.reply_admitted, 'A finite human handoff cannot sample again without a new user message');
    for (const message of body.messages.filter(message => message.role === 'tool')) {
      const call = active.calls.get(message.tool_call_id); if (!call || call.raw !== undefined) continue;
      await recordRaw(call, rawTool(message), body.messages);
    }
    if (active.stage === 'question' && active.turn_cancel) {
      active.ready_for_turn_cancel = true;
      while (!active.cancellation_receipt && !interrupted) await delay(10);
      return { content: 'The original computational Turn was explicitly cancelled; this response is not completion evidence.' };
    }
    if (active.completionAccepted) return { content: active.summary };
    let kind = active.stage, name, args;
    if (kind === 'start') { name = 'start_process'; args = active.launch; }
    else if (kind === 'ready_poll') { assert(++active.ready_polls <= 8); name = 'poll_process'; args = { process_id: active.processId, cursor: active.cursor, wait_ms: 1000 }; }
    else if (kind === 'question') {
      name = 'request_user_input'; args = { question: '本次隔离验收的原进程已就绪，请在外部完成操作后回复，或直接回复取消。测试 gate 由外部控制器管理，不是实际账号授权。',
        remaining_work: '本轮先结束并保留原 process_id。用户再次发消息后，在新一轮中检查或取消同一进程，不重启。' };
    } else if (kind === 'terminal_poll') { name = 'poll_process'; args = { process_id: active.processId, cursor: active.cursor, wait_ms: 0 }; }
    else if (kind === 'process_cancel') { name = 'cancel_process'; args = { process_id: active.processId }; }
    else if (kind === 'completion') {
      const events = await eventsFor();
      const control = common.nativeCompletionPlanControl({ tools: body.tools, events, operationId: active.accepted.operation_id, sessionId });
      if (control) { assert(++active.plan_controls <= 2); name = 'update_plan'; kind = 'plan'; args = control.arguments; }
      else {
        const mode = common.nativeCompletionContract({ tools: body.tools, toolChoice: body.tool_choice });
        active.completion_mode = mode;
        if (mode === 'direct_answer_non_ledger') { assert(++active.direct_answers <= 2); return { content: active.summary }; }
        assert(++active.completion_attempts <= 4); name = 'report_completion'; kind = 'completion';
        args = common.nativeCompletionArguments({ tool: body.tools.find(tool => tool.function?.name === name), observedCallIds: active.evidence_ids, summary: active.summary });
      }
    } else throw new Error('No current scripted fixture stage');
    if (!body.tools?.some(tool => tool.function?.name === name)) {
      assert(['start_process', 'poll_process', 'cancel_process'].includes(name), 'A required formal control was not exposed by the frozen default contract');
      assert(body.tools.some(tool => tool.function?.name === 'ToolSearch')); assert(++active.discovery_attempts <= 4);
      kind = 'discover'; args = { query: name }; name = 'ToolSearch';
    }
    const id = common.nativeRunCallId(runId, `${active.id}-${++active.sequence}`), call = { id, name, kind, args: structuredClone(args), case_id: active.id };
    if (kind === 'discover') call.query = args.query;
    if (kind === 'question') { assert(!active.question_id); active.question_id = id; active.question_provider_sequence = providerCount; active.stage = active.cancel_process ? 'process_cancel' : 'terminal_poll'; }
    active.calls.set(id, call);
    return { tool_calls: [{ index: 0, id, type: 'function', function: { name, arguments: JSON.stringify(args) } }] };
  };
  try {
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
    server = createServer(async (request, response) => {
      try {
        assert.equal(request.method, 'POST'); assert.equal(request.url, '/v1/chat/completions');
        const chunks = []; let count = 0; for await (const chunk of request) { count += chunk.length; assert(count <= 4 * 1024 * 1024); chunks.push(chunk); }
        const body = JSON.parse(Buffer.concat(chunks).toString('utf8')); assert(Array.isArray(body.messages));
        providerCount++; assert(providerCount <= 64);
        const until = Date.now() + 5000; while (!active?.accepted && Date.now() < until) await delay(10);
        if (interrupted) throw new Error('Fixture interrupted');
        const delta = await selectCall(body);
        response.writeHead(200, { 'content-type': 'text/event-stream' });
        response.end(`data: ${JSON.stringify({ id: `wait-${runId}`, choices: [{ index: 0, delta, finish_reason: null }] })}\n\ndata: ${JSON.stringify({ id: `wait-${runId}`, choices: [{ index: 0, delta: {}, finish_reason: delta.tool_calls ? 'tool_calls' : 'stop' }] })}\n\ndata: [DONE]\n\n`);
      } catch (error) { providerFailure ??= error; response.writeHead(500, { 'content-type': 'application/json' }); response.end(JSON.stringify({ error: { message: 'Strict native wait fixture rejected the request; inspect the bounded validation report.' } })); }
    });
    await new Promise(done => server.listen(0, '127.0.0.1', done));
    const provider = await api('POST', '/api/providers', { platform: 'custom', name: `Current main native wait ${runId}`,
      base_url: `http://127.0.0.1:${server.address().port}/v1`, auth_scheme: 'bearer', credentials: { api_keys: ['local-fixture-only'] }, enabled: true,
      initial_model: { model: 'current-main-native-wait', enabled: true, capabilities: [{ task: 'chat', traits: [], protocol: 'openai.chat_text', connection_role: 'default', output_limit: 4096 }] } });
    const model = { provider_id: provider.provider_id, model: 'current-main-native-wait' };
    const preset = await api('POST', '/api/agent-presets/from-template/assistant.general', { reuse_existing: false, display_name: 'Current main native wait acceptance', model });
    const opened = await api('POST', '/api/agent-sessions', { preset_id: preset.preset.preset_id, model, workspace }); sessionId = opened.agent_session_id;
    report.session_id = sessionId; report.provider_id = provider.provider_id; report.default_template = 'assistant.general';
    const before = await api('GET', `/api/agent-sessions/${sessionId}`), frozenBinding = before.session.agent_binding, owner = before.session.owner_ref;
    assert.deepEqual(owner, { principal_kind: 'user', principal_id: claims.user_id }); report.frozen_binding_sha256 = hash(stable(frozenBinding));
    for (const definition of [{ id: 'ordinary-owner-first', administrator: false },
      { id: 'administrator-owner-first', administrator: true },
      { id: 'ordinary-process-cancel', administrator: false, cancel_process: true },
      { id: 'ordinary-turn-cancel', administrator: false, turn_cancel: true, cancel_process: true }]) {
      assert(!interrupted); providerFailure = undefined;
      active = { ...definition, stage: 'start', nonce: randomUUID(), calls: new Map(), evidence_ids: new Set(), sequence: 0, cursor: 0,
        output: '', ready_polls: 0, deferrals: 0, preflight_deferrals: [], discovery_attempts: 0, completion_attempts: 0, plan_controls: 0, direct_answers: 0 };
      active.gate = join(fixture, `gate-${active.id}-${active.nonce}.txt`); assert(await absent(active.gate));
      const script = `$i=[Security.Principal.WindowsIdentity]::GetCurrent();$p=New-Object Security.Principal.WindowsPrincipal($i);[pscustomobject]@{pid=$PID;sid=$i.User.Value;elevated=$p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator);session_id=(Get-Process -Id $PID).SessionId;profile=$env:USERPROFILE}|ConvertTo-Json -Compress;[Console]::WriteLine('NOMIFUN_WAIT_READY');[Console]::Out.Flush();while(-not[IO.File]::Exists(${windows.psLiteral(active.gate)})){Start-Sleep -Milliseconds 100};if([IO.File]::ReadAllText(${windows.psLiteral(active.gate)})-cne${windows.psLiteral(active.nonce)}){throw 'Fixture gate differs'};[Console]::WriteLine(${windows.psLiteral(`NOMIFUN_WAIT_CONFIRMED:${active.nonce}`)});[Console]::Out.Flush();exit 0`;
      active.launch = { command: windows.powershell, args: windows.psArguments(script), cwd: workspace,
        execution_identity: active.administrator ? 'administrator' : 'current_user', wait_ms: 0 };
      active.summary = `本次 ${active.id} 只启动原进程一次。会话上下文保留；计算回合在需要用户时完成交接或被明确取消。后台退出不触发模型。新消息在新回合中${active.cancel_process ? '明确取消' : '验证'}同一 process_id，实际终态与 cleanup.reaped 已确认。仅覆盖 scripted-provider 产品路径。`;
      const prompt = `隔离 Windows 等待验收 ${active.id}：启动 ${JSON.stringify(active.launch)}，检查原 process_id 到 READY，然后请求用户外部操作。不要写 gate。需要用户时结束本轮并保留原进程；后续普通消息决定继续验证或取消。系统 UAC 仍由用户在 Windows 中完成，不接收聊天凭据。`;
      active.accepted = await submitMessage(prompt); active.initial_accepted = active.accepted;
      const caseReport = { id: active.id, administrator: active.administrator, process_cancel: !!active.cancel_process, turn_cancel: !!active.turn_cancel,
        accepted: active.accepted, status: 'running', gate: active.gate, process_deadline: null,
        gate_actor: 'external disposable fixture controller; gate writes and native exit never represent human messages', started_at: new Date().toISOString() };
      active.caseReport = caseReport; report.cases.push(caseReport); await checkpoint();
      let initialFacts, original;
      if (active.turn_cancel) {
        const until = Date.now() + 180000;
        while (!active.ready_for_turn_cancel && Date.now() < until) { if (providerFailure) throw providerFailure; assert(!interrupted); await delay(100); }
        assert(active.ready_for_turn_cancel && active.processId && active.observer, 'Original process must be independently observed before active Turn cancellation');
        active.cancellation_receipt = await api('POST', `/api/agent-sessions/${sessionId}/turns/cancel`, { idempotency_key: randomUUID(), expected_turn_id: active.accepted.message_id });
        initialFacts = await requireKnownSettlement('cancelled');
        caseReport.turn_cancellation = cancelledOwnerCleanupProof({ events: initialFacts.events, operation: active.accepted.operation_id,
          session: sessionId, accepted: active.accepted, execution: initialFacts.execution, cancellation: active.cancellation_receipt });
        const current = initialFacts.events.filter(event => event.correlation_id === active.accepted.operation_id);
        original = { operation_id: active.accepted.operation_id, terminal_seq: Math.max(...current.map(event => event.seq)), model_step_count: modelSteps(initialFacts.events, active.accepted.operation_id) };
        const cleanup = one(progress(initialFacts.events, original.operation_id).filter(row => row.event?.event === 'host_cleanup_proven'), 'cancelled Turn retention');
        assert(cleanup.event.session_process_retention?.processes.some(item => item.process_id === active.processId && item.pid === active.pid), 'Cancelling computation must retain its Session-owned process');
      } else {
        initialFacts = await requireKnownSettlement('completed');
        const plan = await api('GET', `/api/agent-sessions/${sessionId}/task-plan`), projection = await api('GET', `/api/agent-sessions/${sessionId}/projection`);
        original = handoffProof({ events: initialFacts.events, operation: active.accepted.operation_id, session: sessionId,
          accepted: active.accepted, requestId: active.question_id, plan, projection, processId: active.processId, pid: active.pid });
        caseReport.handoff = original;
      }
      assert(active.processId && active.observer); const beforeCount = providerCount;
      caseReport.handoff_idle = await unchangedComputation(original, beforeCount);
      const live = windows.windowsProcessObservation(active.pid); assert.equal(live.state, 'running');
      assert.equal(live.creation_filetime, active.independent_identity.native.creation_filetime);
      Object.assign(caseReport, { original_process: active.independent_identity, retained_observer_ready: active.retained_observer_ready,
        process_id: active.processId, pid: active.pid });
      if (!active.cancel_process) {
        await writeFile(active.gate, active.nonce, { flag: 'wx' }); active.gate_written = true;
        caseReport.external_gate = { path: active.gate, sha256: hash(active.nonce), released_at: new Date().toISOString(), human_reply_submitted: false, os_authentication_claimed: false };
        const exit = await active.observer.completion; assert.equal(exit.exit_code, 0); caseReport.exact_process_exit = exit;
        caseReport.background_exit_idle = await unchangedComputation(original, beforeCount);
      } else assert(await absent(active.gate));
      active.reply_admitted = true; active.evidence_ids = new Set(); active.plan_controls = 0; active.stage = active.cancel_process ? 'process_cancel' : 'terminal_poll';
      active.accepted = await submitMessage(active.cancel_process ? '请取消刚才那个原进程，确认实际清理结果。' : '我已完成外部操作，请检查刚才原进程的实际结果。');
      assert.notEqual(active.accepted.operation_id, active.initial_accepted.operation_id); assert.notEqual(active.accepted.message_id, active.initial_accepted.message_id);
      caseReport.reply = active.accepted;
      const settled = await requireKnownSettlement('completed');
      const exit = await active.observer.completion; if (!active.cancel_process) assert.equal(exit.exit_code, 0);
      const after = settled.session; assert.deepEqual(after.session.agent_binding, frozenBinding); assert.deepEqual(after.session.owner_ref, owner);
      assert.equal(await executableHash(ready.exe), options['--expected-exe-sha256']);
      assert.equal(modelSteps(settled.events, original.operation_id), original.model_step_count, 'A user reply cannot reopen the original finite computation');
      assert(!progress(settled.events, original.operation_id).some(row => row.seq > original.terminal_seq), 'All successor work belongs to its new Turn');
      const operationEvents = settled.events.filter(event => event.correlation_id === active.accepted.operation_id);
      const finalPlan = await api('GET', `/api/agent-sessions/${sessionId}/task-plan`);
      const disposition = terminalDispositionProof({ events: operationEvents, operation: active.accepted.operation_id, session: sessionId,
        accepted: active.accepted, plan: finalPlan, cancelled: false, completionMode: active.completion_mode, directAnswers: active.direct_answers });
      const launchProof = uniqueLaunchProof({ events: settled.events, operation: active.initial_accepted.operation_id, session: sessionId,
        call: active.actual_start_call, output: active.actual_start_call.raw });
      const control = active.calls.get(active.terminal_control.call_id);
      caseReport.successor_control = continuedProcessProof({ events: settled.events, operation: active.accepted.operation_id,
        initialOperation: active.initial_accepted.operation_id, session: sessionId, accepted: active.accepted,
        call: control, output: control.raw, processId: active.processId, cancelled: !!active.cancel_process });
      if (active.question_id) {
        const question = one(progress(settled.events, active.initial_accepted.operation_id).filter(row => row.event?.event === 'tool_completed' && row.event.result?.call_id === active.question_id), 'formal input ACK');
        assert.equal(question.event.result.is_error, false); slot(question.record, '/event/result/output', 'tool_output', [{ type: 'text', text: ACK }], sessionId);
      }
      assert.equal(settled.events.filter(event => event.kind === 'turn/started' && [active.initial_accepted.message_id, active.accepted.message_id].includes(event.payload?.value?.source_message_id)).length, 2);
      Object.assign(caseReport, { status: 'passed', launch_proof: launchProof, typed_terminal: disposition, final_task_plan: finalPlan, exact_process_exit: exit,
        execution: settled.execution, canonical_event_count: settled.events.length, canonical_events_sha256: hash(stable(settled.events)),
        canonical_events: settled.events, preflight_deferrals: active.preflight_deferrals, owner_preserved: true, frozen_binding_preserved: true,
        cleanup_proven: true, completed_at: new Date().toISOString() }); await checkpoint();
    }
    assert.equal(report.cases.length, 4); assert(report.cases.every(item => item.status === 'passed'));
    const finalAnnouncement = await common.readNativeAnnouncement(data); assert.equal(finalAnnouncement.pid, announcement.pid); assert.equal(finalAnnouncement.port, announcement.port);
    report.final_native_announcement = finalAnnouncement; report.status = 'passed';
    report.host_fixture_cleanup = [];
    for (const item of report.cases) if (item.external_gate) {
      const info = await lstat(item.gate); assert(info.isFile() && !info.isSymbolicLink() && info.nlink === 1);
      const bytes = await windows.readWindowsOwnedFixtureBytes(item.gate, 256); assert.equal(hash(bytes), item.external_gate.sha256);
      await unlink(item.gate); report.host_fixture_cleanup.push({ path: item.gate, removed_known_external_gate: true });
    }
    for (const directory of [workspace, fixture]) {
      try { assert.deepEqual(await readdir(directory), []); await rmdir(directory); report.host_fixture_cleanup.push({ path: directory, removed_known_empty_directory: true }); }
      catch (error) { report.host_fixture_cleanup.push({ path: directory, retained: true, os_code: error.code ?? null,
        reason: 'Known directory was busy or nonempty; retain it for review and nonrecursive cleanup after App exit' }); break; }
    }
  } catch (error) {
    report.status = 'failed'; report.failure = { name: error.name, message: String(error.message).replaceAll(token, '[REDACTED]').slice(0, 4096) };
    if (active?.caseReport) active.caseReport.status = 'failed';
    if (sessionId && active?.accepted) {
      try {
        const facts = await currentFacts(); report.failure_canonical_observation = facts;
        if (facts.session.head.active_turn_id === active.accepted.operation_id && facts.execution?.operation_id === active.accepted.operation_id) {
          active.cancellation_receipt = await api('POST', `/api/agent-sessions/${sessionId}/turns/cancel`, { idempotency_key: randomUUID(), expected_turn_id: active.accepted.message_id });
          report.failure_exact_cancel = active.cancellation_receipt;
          const heldFailure = providerFailure; providerFailure = undefined;
          try { report.failure_cleanup = await requireKnownSettlement('cancelled'); } finally { providerFailure = heldFailure; }
        }
        if (active.observer) report.failure_process_identity = windows.windowsProcessObservation(active.pid); // Session process retention is not a false reaped receipt.
      } catch (failure) { report.failure_cleanup_unproven = { name: failure.name, message: String(failure.message).replaceAll(token, '[REDACTED]').slice(0, 4096) }; }
    }
    report.retained_fixture = { path: fixture, reason: 'Failed/unknown results are preserved; no gate replay, reconciliation or host deletion was attempted' };
  } finally {
    process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', interrupt);
    report.completed_at = new Date().toISOString(); report.native_api_mutation_dispatched = dispatchedMutation;
    interrupted = true; if (server) { server.closeAllConnections(); await new Promise(done => server.close(done)); }
    await checkpoint();
    console.log(JSON.stringify({ status: report.status, report: reportPath, report_sha256: hash(await readFile(reportPath)),
      session_id: sessionId, case_count: report.cases.length, passed_cases: report.cases.filter(item => item.status === 'passed').length }));
  }
  return report.status === 'passed' ? 0 : 1;
}
function selfTest() {
  assert.deepEqual(parseOptions(['--self-test']), { selfTest: true });
  assert.throws(() => parseOptions(['--execute'])); assert.throws(() => parseOptions(['--execute', '--execute']));
  assert.throws(() => parseOptions(['--yes']));
  const session = 'session', operation = 'operation', requestId = 'question';
  const event = (seq, content, owner = operation) => ({ agent_session_id: session, correlation_id: owner, seq, kind: 'runtime/progress-recorded', payload: { value: { event: content } } });
  const accepted = { message_id: 'root' }, retained = { agent_session_id: session, owner_generation: '00000000-0000-7000-8000-000000000001',
    processes: [{ process_id: 'owned', pid: 42, creator_turn_id: operation }] };
  const events = [{ agent_session_id: session, correlation_id: operation, seq: 1, kind: 'turn/started', payload: { value: { source_message_id: 'root' } } },
    event(2, { event: 'model_step_started' }), event(3, { event: 'user_input_requested', request: { request_id: requestId } }),
    event(4, { event: 'turn_input_required', request: { request_id: requestId } }), event(5, { event: 'host_cleanup_proven', session_process_retention: retained }),
    { agent_session_id: session, correlation_id: operation, seq: 6, kind: 'turn/completed' }];
  const plan = { conversation_id: session, turn_id: 'root', turn_status: 'completed', turn_outcome: { kind: 'input_required', request_id: requestId } };
  const projection = { runtime: { state: 'idle', active_turn_id: null, can_send_message: true, is_processing: false } };
  const handoff = extras => handoffProof({ events, operation, session, accepted, requestId, plan, projection, processId: 'owned', pid: 42, ...extras });
  assert.equal(handoff({}).model_step_count, 1);
  assert.throws(() => handoff({ plan: { ...plan, turn_status: 'running' } }));
  assert.throws(() => handoff({ projection: { runtime: { ...projection.runtime, active_turn_id: 'root' } } }));
  assert.throws(() => handoff({ projection: { runtime: { ...projection.runtime, can_send_message: false } } }));
  assert.throws(() => handoff({ projection: { runtime: { ...projection.runtime, can_steer: true } } }));
  assert.throws(() => handoff({ events: events.filter(item => item.seq !== 5) }));
  assert.throws(() => handoff({ events: [...events, event(7, { event: 'turn_wait_resolved' })] }));
  assert.throws(() => handoff({ requestId: 'foreign' })); assert.throws(() => handoff({ processId: 'foreign-process' }));
  const successor = 'next-operation', reply = { message_id: 'reply' }, continuedCall = { id: 'poll', name: 'poll_process', args: { process_id: 'owned', cursor: 0 } };
  const terminalValue = { state: 'exited', process_id: 'owned', exit_code: 0, cleanup: { reaped: true, errors: [] } }, terminalText = JSON.stringify(terminalValue);
  const successorEvents = [{ agent_session_id: session, correlation_id: successor, seq: 7, kind: 'turn/started', payload: { value: { source_message_id: 'reply' } } },
    event(8, { event: 'tool_call_completed', call: { call_id: 'poll', name: 'poll_process', arguments: continuedCall.args } }, successor),
    event(9, { event: 'tool_completed', result: { call_id: 'poll', is_error: false, output: [{ type: 'text', text: terminalText }] } }, successor)];
  const continued = extras => continuedProcessProof({ events: successorEvents, operation: successor, initialOperation: operation, session,
    accepted: reply, call: continuedCall, output: terminalText, processId: 'owned', cancelled: false, ...extras });
  assert.equal(continued({}).actual_result.exit_code, 0);
  assert.throws(() => continued({ operation })); assert.throws(() => continued({ processId: 'another-process' }));
  assert.throws(() => continued({ events: [...successorEvents, event(10, { event: 'tool_started', action_id: 'workspace.process/start' }, successor)] }));
  assert.throws(() => continued({ events: [...successorEvents, event(10, { event: 'steering_inputs' }, successor)] }));
  const cancelledValue = { state: 'cancelled', process_id: 'owned', cleanup: { reaped: true, errors: ['interrupt unavailable; terminated instead'] } };
  const cancelledText = JSON.stringify(cancelledValue), cancelCall = { id: 'cancel', name: 'cancel_process', args: { process_id: 'owned' } };
  const processCancelEvents = [successorEvents[0], event(8, { event: 'tool_call_completed', call: { call_id: 'cancel', name: 'cancel_process', arguments: cancelCall.args } }, successor),
    event(9, { event: 'tool_completed', result: { call_id: 'cancel', is_error: false, output: [{ type: 'text', text: cancelledText }] } }, successor)];
  assert.equal(continued({ events: processCancelEvents, call: cancelCall, output: cancelledText, cancelled: true }).actual_result.cleanup.errors.length, 1);
  assert.throws(() => continued({ events: processCancelEvents, call: cancelCall, output: cancelledText, cancelled: false }));
  const bytes = [{ type: 'text', text: ACK }], digest = hash(stable(bytes));
  slot({ agent_session_id: session, payload: { value: { event: { result: { output: bytes } } } } }, '/event/result/output', 'tool_output', bytes, session);
  const shared = { agent_session_id: session, payload: { value: { _canonical_shared_content: { version: 1, references: [{ pointer: '/event/result/output', kind: 'tool_output',
    digest, byte_len: Buffer.byteLength(stable(bytes)), payload_id: `shared-tool:${session}:output:${digest}` }] } } } };
  slot(shared, '/event/result/output', 'tool_output', bytes, session);
  assert.throws(() => slot(shared, '/event/result/output', 'tool_output', [{ type: 'text', text: 'forged' }], session));
  assert.throws(() => slot(shared, '/event/result/output', 'tool_output', bytes, 'foreign-session'));
  const start = { agent_session_id: session, correlation_id: operation, seq: 1, kind: 'turn/started', payload: { value: { source_message_id: accepted.message_id } } };
  const formal = [start, event(10, { event: 'completion_reported', report: { criteria: [{ disposition: 'supported' }] } }),
    event(11, { event: 'completion_delivered' }), event(12, { event: 'host_cleanup_proven' }),
    { agent_session_id: session, correlation_id: operation, seq: 13, kind: 'turn/completed' }];
  const finalPlan = { conversation_id: session, turn_id: accepted.message_id, turn_status: 'completed', turn_outcome: { kind: 'answered', message_ids: ['sealed-message'] } };
  const finalProof = extras => terminalDispositionProof({ events: formal, operation, session, accepted, plan: finalPlan, cancelled: false,
    completionMode: 'report_completion', ...extras });
  assert.equal(finalProof({}).completion_delivered_seq, 11);
  assert.throws(() => finalProof({ plan: { ...finalPlan, turn_outcome: undefined } }));
  assert.throws(() => finalProof({ plan: { ...finalPlan, turn_id: 'foreign-turn' } }));
  assert.throws(() => finalProof({ plan: { ...finalPlan, turn_outcome: { kind: 'failed', message_ids: [] } } }));
  assert.throws(() => finalProof({ plan: { ...finalPlan, turn_outcome: { kind: 'input_required', request_id: 'pending' } } }));
  assert.throws(() => finalProof({ events: formal.filter(row => row.seq !== 11) }));
  assert.throws(() => finalProof({ events: formal.filter(row => row.seq !== 10) }));
  assert.throws(() => finalProof({ events: formal.map(row => ({ ...row, correlation_id: 'foreign-operation' })) }));
  const originalBuild = { build_id: 'fixture-build', build_digest: 'b'.repeat(64) };
  const owned = (seq, value) => ({ ...event(seq, value), producer_id: 'runtime_supervisor' });
  const cancelledEvents = [start, owned(2, { event: 'turn_started', turn_operation_id: operation, binding: originalBuild }),
    { agent_session_id: session, correlation_id: operation, seq: 10, event_id: 'cancel-receipt', producer_id: 'session_api',
      kind: 'turn/cancelled', payload: { value: { target_operation_id: operation } } },
    owned(11, { event: 'host_cleanup_proven', operation, epoch: 1, binding: { ...originalBuild } }), owned(12, { event: 'turn_cancelled', model_steps: 5 })];
  const cancelledPlan = { ...finalPlan, turn_status: 'cancelled', turn_outcome: { kind: 'cancelled', message_ids: [] } };
  const cancelledExecution = { operation_id: operation, state: 'cancelled', execution_generation: 1, pending_effects: 0, unknown_effects: 0,
    producer_lease_live: false, checkpoint_retained: false, automatic_replay_authorized: false, model_progress: { used_model_steps: 5 } };
  const cancelledReceipt = { agent_session_id: session, target_operation_id: operation, message_id: 'cancel-receipt', status: 'cancelled', duplicate: false,
    cursor: { agent_session_id: session, seq: 10 } };
  const cancelProof = extras => cancelledOwnerCleanupProof({ events: cancelledEvents, operation, session, accepted, execution: cancelledExecution,
    cancellation: cancelledReceipt, ...extras });
  assert.equal(finalProof({ events: cancelledEvents, plan: cancelledPlan, cancelled: true, execution: cancelledExecution, cancellation: cancelledReceipt }).turn_outcome.kind, 'cancelled');
  assert.throws(() => finalProof({ events: cancelledEvents, plan: finalPlan, cancelled: true, execution: cancelledExecution, cancellation: cancelledReceipt }));
  assert.equal(cancelProof({ cancellation: undefined }), undefined);
  assert.equal(cancelProof({ events: cancelledEvents.filter(row => row.seq < 11) }), undefined, 'An admission stop is not a cleanup receipt');
  assert.equal(cancelProof({ events: cancelledEvents.filter(row => row.seq !== 12) }), undefined, 'The cleanup witness still needs the original Runtime closure');
  assert.throws(() => cancelProof({ events: cancelledEvents.filter(row => row.seq !== 11) }));
  const changedCancellation = change => { const copy = structuredClone(cancelledEvents); change(copy); return copy; };
  assert.throws(() => cancelProof({ events: changedCancellation(copy => { copy[3].payload.value.event.epoch = 2; }) }));
  assert.throws(() => cancelProof({ events: changedCancellation(copy => { copy[3].payload.value.event.binding.build_digest = 'c'.repeat(64); }) }));
  assert.throws(() => cancelProof({ cancellation: { ...cancelledReceipt, target_operation_id: 'another-operation' } }));
  assert.throws(() => cancelProof({ cancellation: { ...cancelledReceipt, message_id: 'another-cancel-receipt' } }));
  assert.throws(() => cancelProof({ execution: { ...cancelledExecution, execution_generation: 99 } }));
  assert.throws(() => cancelProof({ events: changedCancellation(copy => { copy[4].seq = 9; }) }));
  assert.throws(() => cancelProof({ execution: { ...cancelledExecution, unknown_effects: 1 } }));
  assert.throws(() => cancelProof({ events: changedCancellation(copy => { copy[4].payload.value.event.model_steps = 6; }) }));
  const call = { id: 'start-call', args: { command: 'fixture.exe' } }, startedValue = { state: 'running', process_id: 'owned', pid: 42 };
  const effectId = 'effect-independent-correlation';
  const effectInput = { strategy: 'managed_effect', effect_id: effectId, turn_id: operation, operation_id: `${operation}:tool:${call.id}`,
    owner_domain: 'workspace', capability_module: 'workspace.process', action_id: 'workspace.process/start', input_digest: 'a'.repeat(64),
    resource_binding_id: 'frozen-process-resource', resource_key: 'workspace:fixture' };
  const launchEvents = [event(1, { event: 'tool_call_completed', call: { call_id: call.id, name: 'start_process', arguments: call.args } }),
    event(2, { event: 'tool_started', call_id: call.id, action_id: 'workspace.process/start' }),
    { agent_session_id: session, seq: 3, event_id: `${effectId}:started`, kind: 'effect/started', correlation_id: effectId, payload: { value: effectInput } },
    { agent_session_id: session, seq: 4, kind: 'effect/succeeded', correlation_id: effectId, causation_event_id: `${effectId}:started`, payload: { value: { ...effectInput, result: startedValue } } },
    event(5, { event: 'tool_completed', result: { call_id: call.id, is_error: false, output: [{ type: 'text', text: JSON.stringify(startedValue) }] } })];
  const launchProof = values => uniqueLaunchProof({ events: values, operation, session, call, output: JSON.stringify(startedValue) });
  assert.equal(launchProof(launchEvents).effect_terminal_seq, 4);
  assert.throws(() => launchProof(launchEvents.filter(record => record.correlation_id === operation)));
  const altered = change => { const copy = structuredClone(launchEvents); change(copy); return copy; };
  assert.throws(() => launchProof(altered(copy => { copy[2].payload.value.operation_id = 'another-operation:tool:start-call'; })));
  assert.throws(() => launchProof(altered(copy => { copy[3].payload.value.effect_id = 'another-effect'; })));
  assert.throws(() => launchProof(altered(copy => { copy[3].payload.value.input_digest = 'b'.repeat(64); })));
  assert.throws(() => launchProof(altered(copy => { copy[2].agent_session_id = 'foreign-session'; })));
  assert.throws(() => launchProof(altered(copy => { copy[3].correlation_id = operation; })));
  console.log(JSON.stringify({ status: 'passed', scope: 'pure validation only; native App/UAC/API were not invoked', checks: 55 }));
}
try { const options = parseOptions(process.argv.slice(2)); process.exitCode = options.selfTest ? (selfTest(), 0) : await run(options); }
catch (error) { console.error(JSON.stringify({ status: 'failed_before_native_receipt', name: error.name, message: String(error.message).slice(0, 2048) })); process.exitCode = 1; }
