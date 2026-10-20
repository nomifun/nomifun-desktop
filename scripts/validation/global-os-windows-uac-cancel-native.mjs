#!/usr/bin/env bun
// One public default-Agent Turn against an already-running native App. The
// operator is expected to cancel the first Windows authentication prompt; the
// native code alone does not identify who cancelled it. This driver never
// operates a security desktop, retries authentication, or launches an App.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { execFile } from 'node:child_process';
import { createReadStream } from 'node:fs';
import { lstat, mkdir, readFile, readdir, realpath, rmdir, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { promisify } from 'node:util';
import { nativeRunCallId, readNativeAnnouncement, validateNativeDataset } from './global-os-native.mjs';
import { initializeWindowsPrivateEmptyFixture, powershell, psArguments, psLiteral, readWindowsOwnedFixtureBytes,
  windowsLoginIdentity, windowsObservation, windowsProcessObservation } from './global-os-windows-cases.mjs';
import { windowsIntegrityObservationScript } from './global-os-windows-cases.mjs';

const execute = promisify(execFile);
const delay = ms => new Promise(done => setTimeout(done, ms));
const hash = value => createHash('sha256').update(value).digest('hex');
const stable = value => JSON.stringify(value, (_, item) => item && typeof item === 'object' && !Array.isArray(item)
  ? Object.fromEntries(Object.keys(item).sort().map(key => [key, item[key]])) : item);
const absent = path => lstat(path).then(() => false, error => { if (error.code === 'ENOENT') return true; throw error; });
const one = (items, label) => { assert.equal(items.length, 1, `Expected one exact ${label}`); return items[0]; };
async function executableHash(path) { const digest = createHash('sha256'); for await (const bytes of createReadStream(path)) digest.update(bytes); return digest.digest('hex'); }

function canonicalSlot(record, pointer, kind, expected, session) {
  assert.equal(record.agent_session_id, session);
  const envelope = record.payload?.value;
  const actual = pointer.slice(1).split('/').reduce((item, key) => item?.[key], envelope);
  if (actual !== null && actual !== undefined) { assert.deepEqual(actual, expected); return; }
  const shared = envelope?._canonical_shared_content; assert.equal(shared?.version, 1);
  const reference = one(shared.references.filter(ref => ref.pointer === pointer), 'shared canonical slot');
  const bytes = Buffer.from(stable(expected));
  assert.equal(reference.kind, kind); assert.equal(reference.digest, hash(bytes)); assert.equal(reference.byte_len, bytes.length);
  assert.equal(reference.payload_id, `shared-tool:${session}:${kind === 'tool_arguments' ? 'arguments' : 'output'}:${reference.digest}`);
}

/** Failed canonical summaries are exposed as compact error Tips, not status rows. */
export function windowsUacFailureMessage(history, {session, accepted}) {
  assert.notEqual(history?.success, false);
  const page = history?.data ?? history;
  assert(Array.isArray(page?.items), 'Expected the public canonical message-history items');
  // StoredMessageMapper also requires object content; JSON-looking strings are
  // not another persisted-message format or authority to invent typed fields.
  return one(page.items.filter(message => message.conversation_id === session
    && message.type === 'tips' && message.status === 'error'
    && message.content && typeof message.content === 'object' && !Array.isArray(message.content)
    && message.content.type === 'error' && message.content.turn_id === accepted.message_id), 'public API failed Turn error');
}

/** Derive the exact Rust carrier for serde_json's two supported map profiles. */
export function windowsUacCarrierPresentations(diagnostic) {
  // SystemExecutionFailure declaration order and typed_owner_error_result's
  // json! field order. preserve_order is enabled in the actual Tauri build;
  // the BTreeMap profile is used by isolated Runtime/library builds.
  const fields = ['code','message','kind','raw_os_error','user_code_not_started','cleanup_proven'];
  assert.deepEqual(Object.keys(diagnostic).sort(), [...fields].sort());
  const native = Object.fromEntries(fields.map(field => [field, diagnostic[field]]));
  const value = {code:native.code,message:native.message,details:{system_execution_error:native}};
  return [['serde_json_preserve_order', JSON.stringify(value)], ['serde_json_btree', stable(value)]]
    .map(([serializer, carrier]) => ({serializer, carrier, notice:`Repository instruction body omitted from durable history (${Buffer.byteLength(carrier)} UTF-8 bytes, sha256:${hash(carrier)}). Re-read current rules through the authorized workspace tool; this digest grants no authority.`}));
}

function instructionFailureSlot(record, diagnostic, session) {
  const candidates = windowsUacCarrierPresentations(diagnostic).flatMap(value => [
    {...value,presentation:'typed_carrier',output:[{type:'text',text:value.carrier}]},
    {...value,presentation:'instruction_digest_notice',output:[{type:'text',text:value.notice}]},
  ]);
  const envelope = record.payload?.value, actual = envelope?.event?.result?.output;
  let selected;
  if (actual !== null && actual !== undefined) selected = one(candidates.filter(value => stable(value.output) === stable(actual)), 'exact original instruction failure or its privacy digest');
  else {
    const ref = one(envelope?._canonical_shared_content?.references?.filter(value => value.pointer === '/event/result/output') ?? [], 'shared instruction failure slot');
    selected = one(candidates.filter(value => hash(stable(value.output)) === ref.digest), 'exact shared instruction failure presentation');
  }
  canonicalSlot(record, '/event/result/output', 'tool_output', selected.output, session);
  return {serializer:selected.serializer,presentation:selected.presentation,carrier_bytes:Buffer.byteLength(selected.carrier),carrier_sha256:hash(selected.carrier)};
}

/** Only typed canonical facts establish cancellation and zero mutation. */
export function windowsUacCancellationProof({ events, session, accepted, execution, summary, call, markerAbsent }) {
  assert.equal(markerAbsent, true, 'The unique proposed mutation marker must remain absent');
  assert(events.every(event => event.agent_session_id === session));
  assert.equal(execution.operation_id, accepted.operation_id); assert.equal(execution.state, 'failed');
  assert.equal(execution.pending_effects, 0); assert.equal(execution.unknown_effects, 0);
  assert.equal(execution.producer_lease_live, false); assert.equal(typeof execution.checkpoint_retained, 'boolean');
  assert.equal(execution.automatic_replay_authorized, false);
  const current = events.filter(event => event.correlation_id === accepted.operation_id);
  const records = current.filter(event => event.kind === 'runtime/progress-recorded')
    .map(record => ({ record, seq: record.seq, event: record.payload?.value?.event }));
  if (execution.checkpoint_retained) {
    const checkpoint = one(records.filter(row => row.seq === execution.checkpoint_seq && row.event?.event === 'execution_checkpoint_saved'), 'retained diagnostic checkpoint');
    assert(Number.isSafeInteger(execution.checkpoint_revision) && execution.checkpoint_revision > 0);
    assert.match(execution.checkpoint_digest, /^[a-f0-9]{64}$/);
    assert.equal(checkpoint.event.revision, execution.checkpoint_revision); assert.equal(checkpoint.event.digest, execution.checkpoint_digest);
  } else assert(execution.checkpoint_seq == null && execution.checkpoint_digest == null);
  const started = one(current.filter(event => event.kind === 'turn/started'), 'canonical Turn start');
  assert.equal(started.payload?.value?.source_message_id, accepted.message_id);
  const terminal = one(current.filter(event => ['turn/failed', 'turn/completed', 'turn/cancelled'].includes(event.kind)), 'canonical terminal');
  assert.equal(terminal.kind, 'turn/failed', 'OS authentication cancellation is not the user Stop contract');
  const error = terminal.payload?.value?.error, diagnostic = error?.systemExecutionError;
  assert.equal(error?.code, 'SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED');
  assert.equal(error.retryable, false); assert.equal(diagnostic?.code, error.code);
  assert.equal(diagnostic.raw_os_error, 1223); assert.equal(diagnostic.user_code_not_started, true); assert.equal(diagnostic.cleanup_proven, true);
  assert(typeof diagnostic.kind === 'string' && diagnostic.kind.length > 0); assert(typeof diagnostic.message === 'string' && diagnostic.message.length > 0);
  const cleanup = one(records.filter(row => row.event?.event === 'host_cleanup_proven'), 'original owner cleanup');
  assert(started.seq < cleanup.seq && cleanup.seq < terminal.seq);
  const closure = one(records.filter(row => row.event?.event === 'turn_failed'), 'Runtime failed closure');
  assert(closure.seq > cleanup.seq); assert.equal(closure.event.failure?.kind, 'system_execution');
  assert.deepEqual(closure.event.failure.diagnostic, diagnostic);
  assert(!records.some(row => ['turn_cancelled', 'turn_completed', 'completion_delivered'].includes(row.event?.event)));
  const proposal = one(records.filter(row => row.event?.event === 'tool_call_completed' && row.event.call?.call_id === call.id), 'proposed mutation');
  assert.equal(proposal.event.call.name, 'start_process');
  canonicalSlot(proposal.record, '/event/call/arguments', 'tool_arguments', call.args, session);
  assert(!records.some(row => row.event?.event === 'tool_started' && row.event.call_id === call.id), 'No user modification ToolStarted is allowed before successful preflight');
  assert(!events.some(event => event.kind.startsWith('effect/')), 'The proposed mutation cannot have an Effect admission');
  const dispatched = records.filter(row => ['host_tool_dispatch', 'host_resource_dispatch'].includes(row.event?.event));
  for (const row of dispatched) {
    const dispatch = row.event.dispatch;
    assert.equal(row.event.event, 'host_tool_dispatch'); assert.equal(dispatch?.capability_id, 'workspace.files');
    assert.equal(dispatch.action_id, 'workspace.files/read'); assert.equal(dispatch.model_name, 'read_file');
    assert(dispatch.call_id.startsWith('agent-instructions:'));
    assert.equal(dispatch.operation_id, `${accepted.operation_id}:tool:${dispatch.call_id}`);
  }
  const adminProposals = records.filter(row => row.event?.event === 'tool_call_completed' && row.event.step === 0
    && row.event.call?.arguments?.execution_identity === 'administrator');
  const acquisition = one(adminProposals, 'administrator instruction preflight');
  assert.equal(acquisition.event.call.name, 'read_file'); assert.equal(acquisition.event.call.arguments.format, 'instruction_scope');
  const preflight = one(records.filter(row => row.event?.event === 'tool_completed' && row.event.result?.call_id === acquisition.event.call.call_id), 'preflight failure');
  assert.equal(preflight.event.result.is_error, true);
  const preflightPresentation = instructionFailureSlot(preflight.record, diagnostic, session);
  assert(proposal.seq < acquisition.seq && preflight.seq < cleanup.seq);
  assert.equal(records.filter(row => row.seq > preflight.seq && row.event?.event === 'model_step_started').length, 0, 'No model retry follows authentication cancellation');
  assert.equal(dispatched.filter(row => row.seq > preflight.seq).length, 0, 'No ordinary fallback or sibling read follows the native failure');
  assert.equal(summary?.conversation_id, session); assert.equal(summary.type, 'tips'); assert.equal(summary.status, 'error');
  assert.equal(summary.content?.type, 'error'); assert.equal(summary.content.turn_id, accepted.message_id);
  assert.equal(summary.content.turn_outcome?.kind, 'failed');
  assert.deepEqual(summary.content.error?.systemExecutionError, diagnostic); assert.equal(summary.content.error.code, error.code);
  return { code: error.code, native: diagnostic, proposal_seq: proposal.seq, preflight_seq: acquisition.seq,
    preflight_result_seq: preflight.seq, cleanup_seq: cleanup.seq, terminal_seq: terminal.seq,
    mutation_effect_count: 0, user_mutation_dispatch_count: 0, administrator_preflight_count: 1,
    internal_read_dispatch_count: dispatched.length, marker_absent: true, automatic_replay_authorized: false,
    diagnostic_checkpoint_retained:execution.checkpoint_retained,preflight_presentation:preflightPresentation };
}

export function windowsUacCancelOptions(argv) {
  const known = new Set(['--execute', '--fixture-report', '--report-root', '--repo-root', '--expected-source-head', '--expected-exe-sha256']);
  const options = {};
  for (let index = 0; index < argv.length; index++) {
    const flag = argv[index]; assert(known.has(flag) && !Object.hasOwn(options, flag), 'Unknown or duplicate option');
    if (flag === '--execute') options[flag] = true;
    else { assert(argv[index + 1] && !argv[index + 1].startsWith('--')); options[flag] = argv[++index]; }
  }
  assert.equal(options['--execute'], true, 'Native execution requires the explicit --execute flag');
  for (const flag of known) if (flag !== '--execute') assert.equal(typeof options[flag], 'string', `${flag} is required`);
  assert.match(options['--expected-source-head'], /^[a-f0-9]{40}$/); assert.match(options['--expected-exe-sha256'], /^[a-f0-9]{64}$/);
  return options;
}

export async function runWindowsUacCancellation(options) {
  assert.equal(process.platform, 'win32');
  const repo = await realpath(resolve(options['--repo-root']));
  const source = (await execute('git', ['rev-parse', 'HEAD'], { cwd: repo, windowsHide: true })).stdout.trim();
  assert.equal(source, options['--expected-source-head']);
  for (const args of [['diff', '--name-only', 'HEAD'], ['ls-files', '--others', '--exclude-standard']]) {
    const paths = (await execute('git', args, { cwd: repo, windowsHide: true })).stdout.trim().split(/\r?\n/).filter(Boolean);
    assert(paths.every(path => path.startsWith('docs/') || path.startsWith('scripts/validation/')), 'Uncommitted production inputs cannot inherit a source anchor');
  }
  const readyPath = await realpath(resolve(options['--fixture-report'])), readyBytes = await readFile(readyPath), ready = JSON.parse(readyBytes);
  assert.equal(ready.schema, 'nomifun.windows-global-os-fixture.v1'); assert.equal(ready.status, 'ready');
  assert.equal(ready.exe_sha256, options['--expected-exe-sha256']);
  const dataset = await validateNativeDataset(ready.native_data_root);
  assert.equal(ready.parent, dataset.parent); assert.equal(ready.token_file, join(dataset.parent, 'owner-jwt'));
  const token = (await readWindowsOwnedFixtureBytes(ready.token_file, 4096)).toString('utf8').trim(); assert.equal(token.split('.').length, 3);
  const claims = JSON.parse(Buffer.from(token.split('.')[1], 'base64url').toString('utf8')); assert(typeof claims.user_id === 'string');
  const announcement = await readNativeAnnouncement(dataset); assert.equal(announcement.pid, ready.app_pid);
  const origin = new URL(ready.base_url); assert.equal(origin.protocol, 'http:');
  assert(['127.0.0.1', 'localhost', '[::1]'].includes(origin.hostname) && !origin.username && !origin.password); assert.equal(Number(origin.port), announcement.port);
  const app = windowsProcessObservation(ready.app_pid, { includeImagePath: true }); assert.equal(app.state, 'running');
  assert.equal(app.creation_filetime, ready.actual_app_identity?.native_process?.creation_filetime);
  assert.equal((await realpath(app.image_path)).toLowerCase(), ready.exe.toLowerCase());
  assert.equal(await executableHash(ready.exe), options['--expected-exe-sha256']);
  assert.equal(await windowsObservation(windowsIntegrityObservationScript(ready.app_pid)), 8192);
  const login = await windowsLoginIdentity(); assert.equal(ready.actual_app_identity.sid, login.sid); assert.equal(ready.actual_app_identity.session_id, login.session_id);
  const api = async (method, path, body) => {
    assert(path.startsWith('/api/') && !path.includes('..'));
    const response = await fetch(new URL(path, origin), { method, headers: { Authorization:`Bearer ${token}`, 'content-type':'application/json' },
      body:body === undefined ? undefined : JSON.stringify(body), signal:AbortSignal.timeout(30000) });
    assert(response.ok, `Native ${method} ${path} returned HTTP ${response.status}; private body omitted`);
    const bytes = await response.arrayBuffer(); assert(bytes.byteLength <= 16 * 1024 * 1024);
    const value = JSON.parse(Buffer.from(bytes).toString('utf8')); assert.notEqual(value.success, false); return value.data ?? value;
  };
  const caller = await api('GET', '/api/system/permissions'); assert.equal(caller.platform, 'windows'); assert.equal(caller.process_id, ready.app_pid);
  const reportRoot = await realpath(resolve(options['--report-root'])); assert((await lstat(reportRoot)).isDirectory());
  const id = randomUUID(), fixture = join(await realpath(tmpdir()), `nomifun-global-os-product-${id}`), workspace = join(fixture, 'workspace'), marker = join(workspace, `unexecuted-${id}.txt`);
  assert(await absent(fixture)); await mkdir(fixture); const acl = await initializeWindowsPrivateEmptyFixture(fixture, login); await mkdir(workspace); assert(await absent(marker));
  const reportPath = join(reportRoot, `uac-cancel-preflight-native-${id}.json`);
  const report = { schema:'nomifun.windows-uac-cancel-native.v1', status:'running', run_id:id, source_head:source, actual_exe_sha256:ready.exe_sha256,
    fixture_report:readyPath, fixture_report_sha256:hash(readyBytes), app_pid:ready.app_pid, app_creation_filetime:app.creation_filetime,
    native_data_root:dataset.root, fixture, workspace, marker, fixture_acl:acl, started_at:new Date().toISOString(),
    requires_first_authentication_in_app_lifetime:true, authentication_interaction_boundary:'Windows system UI only; OS1223 is not attributed to a particular human gesture',
    evidence_scope:'Real native App/public default chat.minimal/readonly instruction preflight/canonical failed Turn/API history; scripted provider does not prove live-model reasoning.',
    replay_submitted:false, reconciliation_submitted:false, app_launched:false, security_desktop_operated:false };
  const checkpoint = () => writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  let session, accepted, providerFailure, server, proposal, requests = 0;
  const eventsFor = async () => {
    const events = []; let after = 0;
    for (let page = 0; page < 16; page++) {
      const value = await api('GET', `/api/agent-sessions/${session}/events?after_seq=${after}&limit=500`);
      events.push(...value.events); const next = value.next_cursor.seq;
      if (!value.events.length || next <= after) return events; after = next;
    }
    throw new Error('Canonical evidence exceeded the bounded pagination window');
  };
  try {
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, { flag:'wx' });
    server = createServer(async (request, response) => {
      try {
        assert.equal(request.method, 'POST'); assert.equal(request.url, '/v1/chat/completions');
        const chunks = []; let length = 0; for await (const chunk of request) { length += chunk.length; assert(length <= 4 * 1024 * 1024); chunks.push(chunk); }
        const body = JSON.parse(Buffer.concat(chunks).toString('utf8')); assert(Array.isArray(body.messages)); assert(++requests <= 3);
        const deadline = Date.now() + 5000; while (!accepted && Date.now() < deadline) await delay(10); assert(accepted);
        let call;
        if (!body.tools?.some(tool => tool.function?.name === 'start_process')) {
          assert(!proposal); assert.equal(requests, 1); assert(body.tools?.some(tool => tool.function?.name === 'ToolSearch'));
          call = { id:nativeRunCallId(id, 'discover-start'), name:'ToolSearch', args:{query:'start_process'} };
        } else {
          assert(!proposal, 'Authentication cancellation must terminate the Turn without another model execution proposal');
          const script = `$p=${psLiteral(marker)};$f=[IO.File]::Open($p,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None);try{$b=[Text.Encoding]::UTF8.GetBytes(${psLiteral(id)});$f.Write($b,0,$b.Length)}finally{$f.Dispose()};exit 0`;
          call = { id:nativeRunCallId(id, 'unexecuted-admin-marker'), name:'start_process', args:{command:powershell,args:psArguments(script),cwd:workspace,execution_identity:'administrator',timeout_ms:20000} };
          proposal = call; report.proposal = call; await checkpoint();
        }
        const delta = {tool_calls:[{index:0,id:call.id,type:'function',function:{name:call.name,arguments:JSON.stringify(call.args)}}]};
        response.writeHead(200, {'content-type':'text/event-stream'});
        response.end(`data: ${JSON.stringify({id:`uac-${id}`,choices:[{index:0,delta,finish_reason:null}]})}\n\ndata: ${JSON.stringify({id:`uac-${id}`,choices:[{index:0,delta:{},finish_reason:'tool_calls'}]})}\n\ndata: [DONE]\n\n`);
      } catch (error) { providerFailure ??= error; response.writeHead(500, {'content-type':'application/json'}); response.end('{"error":{"message":"Strict native UAC cancellation fixture rejected a request; inspect its report."}}'); }
    });
    await new Promise(done => server.listen(0, '127.0.0.1', done));
    const provider = await api('POST', '/api/providers', {platform:'custom',name:`Native UAC cancellation ${id}`,base_url:`http://127.0.0.1:${server.address().port}/v1`,
      auth_scheme:'bearer',credentials:{api_keys:['local-fixture-only']},enabled:true,
      initial_model:{model:'native-uac-cancel',enabled:true,capabilities:[{task:'chat',traits:[],protocol:'openai.chat_text',connection_role:'default',output_limit:4096}]}});
    const model = {provider_id:provider.provider_id,model:'native-uac-cancel'};
    const preset = await api('POST', '/api/agent-presets/from-template/chat.minimal', {reuse_existing:false,display_name:'Native UAC cancellation acceptance',model});
    const opened = await api('POST', '/api/agent-sessions', {preset_id:preset.preset.preset_id,model,workspace}); session = opened.agent_session_id;
    report.session_id = session; report.provider_id = provider.provider_id; report.default_template = 'chat.minimal';
    const before = await api('GET', `/api/agent-sessions/${session}`); assert.deepEqual(before.session.owner_ref, {principal_kind:'user',principal_id:claims.user_id});
    report.frozen_binding_sha256 = hash(stable(before.session.agent_binding));
    accepted = await api('POST', `/api/agent-sessions/${session}/turns`, {idempotency_key:randomUUID(),input:{content:`隔离 Windows UAC 取消验收 ${id}：拟以 administrator 在 ${workspace} 执行一次唯一 marker 写入。用户会在 Windows 原生认证界面取消；保留认证失败及原生错误码，不重试、不降级普通身份、不继续写入、不报告成功。指令预检必须沿同一 administrator 只读 owner；取消后结束本轮并保留真实清理与 canonical 失败。凭据只进入系统界面。`}});
    report.accepted = accepted; await checkpoint();
    console.log(JSON.stringify({status:'awaiting_native_authentication',report:reportPath,session_id:session,instruction:'Cancel the native Windows UAC prompt for this isolated fixture. This driver does not operate the system prompt.'}));
    const until = Date.now() + 240000;
    let final;
    while (Date.now() < until) {
      if (providerFailure) throw providerFailure;
      const execution = await api('GET', `/api/agent-sessions/${session}/execution`), current = await api('GET', `/api/agent-sessions/${session}`);
      if (execution?.operation_id === accepted.operation_id && execution.state !== 'running' && !current.head.active_turn_id && !execution.producer_lease_live) { final = {execution,session:current}; break; }
      await delay(250);
    }
    assert(final && proposal, 'Native authentication/terminal cleanup was not proven before the deadline');
    const events = await eventsFor(), history = await api('GET', `/api/agent-sessions/${session}/message-history?page=1&page_size=100`);
    report.execution = final.execution; report.events = events; report.history = history;
    const summary = windowsUacFailureMessage(history, {session, accepted});
    report.execution = final.execution; report.events = events; report.summary = summary;
    report.proof = windowsUacCancellationProof({events,session,accepted,execution:final.execution,summary,call:proposal,markerAbsent:await absent(marker)});
    assert.deepEqual(final.session.session.agent_binding, before.session.agent_binding); assert.deepEqual(final.session.session.owner_ref, before.session.owner_ref);
    const providerCount = requests; await delay(1500); assert.equal(requests, providerCount); assert(await absent(marker));
    report.no_retry_observation_ms = 1500; report.provider_request_count = requests; report.post_terminal_marker_absent = true;
    report.actual_exe_sha256_after = await executableHash(ready.exe); assert.equal(report.actual_exe_sha256_after, ready.exe_sha256);
    report.status = 'passed'; report.host_fixture_cleanup = [];
    for (const path of [workspace, fixture]) {
      try { assert.deepEqual(await readdir(path), []); await rmdir(path); report.host_fixture_cleanup.push({path,removed_known_empty_directory:true}); }
      catch (error) { report.host_fixture_cleanup.push({path,retained:true,os_code:error.code ?? null,reason:'Busy or nonempty exact fixture retained for nonrecursive cleanup after App exit'}); break; }
    }
  } catch (error) {
    report.status = 'failed'; report.failure = {name:error.name,message:String(error.message).replaceAll(token,'[REDACTED]').slice(0,4096)};
    if (session) {
      try { report.failure_execution = await api('GET', `/api/agent-sessions/${session}/execution`); report.failure_events = await eventsFor(); report.failure_marker_absent = await absent(marker); }
      catch (failure) { report.failure_observation_unavailable = {name:failure.name,message:String(failure.message).replaceAll(token,'[REDACTED]').slice(0,2048)}; }
    }
    report.retained_fixture = {path:fixture,reason:'Failure is retained without replay, reconciliation or cleanup of an unproven native effect'};
  } finally {
    if (server) await new Promise(done => server.close(done));
    report.completed_at = new Date().toISOString(); report.provider_server_closed = true; await checkpoint();
    console.log(JSON.stringify({status:report.status,report:reportPath,report_sha256:hash(await readFile(reportPath)),session_id:session}));
  }
  return report.status === 'passed' ? 0 : 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { process.exitCode = await runWindowsUacCancellation(windowsUacCancelOptions(process.argv.slice(2))); }
  catch (error) { console.error(JSON.stringify({status:'failed_before_native_receipt',name:error.name,message:String(error.message).slice(0,2048)})); process.exitCode = 1; }
}
