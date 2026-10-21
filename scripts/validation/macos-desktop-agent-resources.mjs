#!/usr/bin/env node
// Actual Desktop T32/T33 driver. The preparer supplies only a disposable
// dataset, normal password credentials and deterministic loopback model.
// LaunchServices/controller, product JWT APIs, native kqueue generations and
// the original resource owners supply all execution/cleanup evidence.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createReadStream, writeSync } from 'node:fs';
import { randomBytes, createHash } from 'node:crypto';
import { open, mkdir, readFile, writeFile, realpath, stat, unlink, rename } from 'node:fs/promises';
import { dirname, join, resolve, isAbsolute } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { launchMacosApplication } from '../lib/macos-application-launch.mjs';
import { macosComputerProcessProof } from './desktop-computer-resource-proof.mjs';
import { verifyGuardedMacosBuild } from './macos-guarded-build-proof.mjs';

const options = process.argv.slice(2);
const option = (name) => { const index = options.indexOf(name); return index < 0 ? undefined : options[index + 1]; };
if (options.includes('--help')) {
  console.log('Use --app /ABS/NomiFun\ Dev.app --guarded-build-proof /ABS/EXTERNAL/build-proof.json --controller /ABS/macos_application_controller --fixture /ABS/browser_gui_fixture --observer /ABS/macos-owned-process-observer --appkit-probe /ABS/macos_appkit_termination_probe --bun /ABS/bun --mode appkit|sigint|sigterm|parent-death|cmd-q [--with-computer] [--stop-first] [--retirement-observe-ms 70000] [--wk-inflight] [--matting-request]. Before any preparer or App starts, the external proof must bind this exact signed debug Host to the reviewed guard/main source and existing build records; strict signature and compiled guard presence are checked. The matching runtime nonce receipt remains mandatory. Every run creates a private direct temporary root and retains failures. with-computer only accepts appkit/cmd-q and adds one permission-free canonical guide call plus the real bundled worker native lifetime/normal exit proof; it does not send desktop input or capture images. retirement-observe-ms requires stop-first and adds a 70000–180000 ms read-only fixture window after original native retirement proof; it is not a product TTL. matting-request verifies normal host cancellation of an original pending authenticated model-supply HTTP request; it is not available for parent-death. cmd-q writes gui-quit-ready.json and gives the authorized GUI operator 300 seconds from that readiness, then requires gui-quit-evidence.json with host_pid, app_path, shortcut="Cmd+Q", observed_at and nonempty absolute evidence_paths. It never generates keyboard evidence itself. Compile the observer with clang -Wall -Wextra -Werror scripts/validation/macos-owned-process-observer.c -o /ABS/observer.');
  process.exit(0);
}
assert.equal(process.platform, 'darwin', 'Actual macOS required');
const { DatabaseSync } = await import('node:sqlite');
const repository = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const mode = option('--mode') ?? 'appkit';
assert(['appkit', 'sigint', 'sigterm', 'parent-death', 'cmd-q'].includes(mode));
const withComputer = options.includes('--with-computer');
assert(!withComputer || ['appkit', 'cmd-q'].includes(mode), 'Computer native cleanup observation requires normal AppKit/Cmd-Q');
assert(!withComputer || !options.includes('--stop-first'), 'Computer Quit evidence requires its original worker live before Quit; use the separate Stop fixture without --with-computer');
const retirementObserveMs = Number(option('--retirement-observe-ms') ?? 0);
assert(Number.isSafeInteger(retirementObserveMs) && (retirementObserveMs === 0
  || (options.includes('--stop-first') && retirementObserveMs >= 70_000 && retirementObserveMs <= 180_000)),
'The optional original Runtime observation requires --stop-first and 70000–180000 ms');
assert(!(mode === 'parent-death' && options.includes('--matting-request')), 'Matting host cancellation requires a normal verified Quit mode');
const paths = {};
for (const name of ['app', 'controller', 'fixture', 'observer', 'appkit-probe', 'bun']) {
  assert(isAbsolute(option(`--${name}`) ?? ''), `An existing absolute --${name} is required`);
  paths[name] = await realpath(option(`--${name}`));
}
const program = await realpath(join(paths.app, 'Contents/MacOS/nomifun-desktop'));
const computerWorker = withComputer ? await realpath(join(paths.app, 'Contents/MacOS/nomifun-computer-worker')) : undefined;
// Refuse stale descriptor-compatible Dev Apps before they can enter Keyboard
// startup. A post-launch missing nonce receipt alone is too late for this case.
const guardedBuild = await verifyGuardedMacosBuild({ proofPath: option('--guarded-build-proof'), appPath: paths.app, repoRoot: repository });
// No daily development supervisor, provider credential or app-specific data
// environment crosses into this isolated launch. Normal native path/locale
// discovery stays available; tool programs are frozen explicitly below.
const isolatedEnvironment = Object.fromEntries(Object.entries(process.env).filter(([name]) =>
  ['PATH', 'HOME', 'TMPDIR', 'LANG', 'LC_ALL', 'LC_CTYPE', 'USER', 'LOGNAME', 'SHELL', 'DEVELOPER_DIR'].includes(name)));
const uuidBytes = randomBytes(16);
uuidBytes.writeUIntBE(Date.now(), 0, 6); uuidBytes[6] = uuidBytes[6] & 0x0f | 0x70; uuidBytes[8] = uuidBytes[8] & 0x3f | 0x80;
const hex = uuidBytes.toString('hex');
const token = `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
const root = join(await realpath(tmpdir()), `nomifun-lifecycle-${token}`);
await mkdir(root, { mode: 0o700 });
const data = join(root, 'data'), work = join(root, 'work');
await mkdir(work, { mode: 0o700 });
await writeFile(join(root, 'fixture-sentinel'), token, { flag: 'wx', mode: 0o600 });
const report = { schema: 'nomifun.macos-desktop-agent-resources.v1', root, mode, token, passed: false,
  source_head: '', started_at: new Date().toISOString(), paths, model: 'scripted-loopback-only',
  data_isolation: 'fresh-private-direct-temporary-root', retirement_observe_ms: retirementObserveMs, native_events: [] };
report.computer = { enabled: withComputer, input_calls: 0, screenshot_calls: 0, model: 'local scripted provider', ordinary_quit_only: withComputer };
report.guarded_build_admission = guardedBuild;
const sleep = ms => new Promise(done => setTimeout(done, ms));
const journal = async () => {
  await writeFile(join(root, 'result.json.tmp'), `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 });
  await rename(join(root, 'result.json.tmp'), join(root, 'result.json'));
};
const publish = async (name, value) => {
  await writeFile(join(root, `${name}.tmp`), `${JSON.stringify(value, null, 2)}\n`, { flag: 'wx', mode: 0o600 });
  await rename(join(root, `${name}.tmp`), join(root, name));
};
const until = async (check, label, budget = 60_000) => {
  const deadline = Date.now() + budget;
  while (Date.now() < deadline) { const value = await check(); if (value) return value; await sleep(100); }
  throw Error(`Timed out waiting for ${label}`);
};
const json = async path => { try { return JSON.parse(await readFile(path, 'utf8')); } catch (error) { if (error.code === 'ENOENT') return undefined; throw error; } };
const completion = child => {
  const done = new Promise((accept, reject) => { child.once('error', reject); child.once('close', (code, signal) => accept({ code, signal })); });
  done.catch(() => {}); return done;
};
const running = child => child.exitCode === null && child.signalCode === null;
const sha256 = async path => { const hash = createHash('sha256'); for await (const bytes of createReadStream(path)) hash.update(bytes); return hash.digest('hex'); };
async function logged(program, args, name, env = process.env) {
  const output = await open(join(root, `${name}.stdout.log`), 'wx', 0o600);
  const errors = await open(join(root, `${name}.stderr.log`), 'wx', 0o600);
  const child = spawn(program, args, { cwd: repository, env, stdio: ['ignore', output.fd, errors.fd] });
  const done = completion(child);
  await output.close(); await errors.close();
  return { child, done };
}
async function run(program, args, name) {
  const command = await logged(program, args, name);
  const result = await command.done;
  assert.equal(result.code, 0, `${name} failed; inspect its retained private logs`);
  return result;
}
let preparer, sidecar, observer, sentinel, sentinelDone, application, jwt;
let applicationResult, observerDone, observerReady = false, observerComplete = false, observerFailure, observerLog;
let exitReached = false, failure, preparerStopped = false;
let exitActionStarted;
let mattingRequest;
try {
  const git = spawn('git', ['rev-parse', 'HEAD'], { cwd: repository, stdio: ['ignore', 'pipe', 'inherit'] });
  let head = ''; git.stdout.on('data', bytes => { head += bytes; }); const gitResult = await completion(git); assert.equal(gitResult.code, 0); report.source_head = head.trim();
  report.build_identity = {};
  for (const [name, path] of Object.entries({ host: program, ...paths, driver: fileURLToPath(import.meta.url),
    sidecar: join(repository, 'scripts/validation/desktop-agent-resources.mjs'),
    computer_proof: join(repository, 'scripts/validation/desktop-computer-resource-proof.mjs'),
    guarded_build_verifier: join(repository, 'scripts/validation/macos-guarded-build-proof.mjs'),
    ...(withComputer ? { computer_worker: computerWorker } : {}) })) {
    if (name !== 'app') report.build_identity[name] = { path, sha256: await sha256(path) };
  }
  preparer = await logged(paths.fixture, [data, '--wk-actions'], 'preparer', { ...isolatedEnvironment, NOMIFUN_WORK_DIR: work });
  report.auxiliary_owners = { driver_pid: process.pid, preparer_pid: preparer.child.pid };
  await journal();
  const fixture = await until(async () => {
    if (!running(preparer.child)) throw Error('Dataset preparer exited before readiness');
    const readyLine = (await readFile(join(root, 'preparer.stdout.log'), 'utf8')).split('\n')
      .find(line => line.startsWith('BROWSER_GUI_FIXTURE_READY '));
    if (!readyLine) return undefined;
    const prepared = JSON.parse(readyLine.slice('BROWSER_GUI_FIXTURE_READY '.length));
    assert.equal(prepared.data_dir, data); assert.equal(prepared.real_provider, false);
    // The file is authored inside preparation. Only the final stdout receipt
    // follows shutdown_all and surrender of the preparer's live DB lease.
    return await json(join(data, 'wk-acceptance.json'));
  }, 'the isolated dataset preparer', 120_000);
  assert.equal(await realpath(fixture.data_dir), await realpath(data)); assert.equal(fixture.mode, 'wk-actions');
  assert.equal((await json(join(data, 'dataset-v3.json'))).work_root, work, 'Preparer and actual Desktop must retain the same existing work-root receipt');
  const preservation = await json(join(data, 'wk-upgrade-preservation.json'));
  const legacyProfile = resolve(data, preservation.legacy_sentinel);
  assert(legacyProfile.startsWith(`${data}/`));
  const generationBefore = await sha256(join(data, 'storage-generation'));
  assert.equal(await readFile(legacyProfile, 'utf8'), preservation.legacy_sentinel_text);
  sentinel = spawn('/bin/sleep', ['600'], { stdio: 'ignore' }); sentinelDone = completion(sentinel);
  report.outside_tree_sentinel = { pid: sentinel.pid, parent: process.pid, preservation_verified: false };
  application = await launchMacosApplication({ program, controllerProgram: paths.controller, cwd: repository,
    args: ['-ApplePersistenceIgnoreState', 'YES'], applicationScope: 'desktop_development',
    environment: { ...isolatedEnvironment, NOMIFUN_DATA_DIR: data, NOMIFUN_WORK_DIR: work, NOMIFUN_BUN_PATH: paths.bun,
      NOMIFUN_LIFECYCLE_FIXTURE_DIR: root, NOMIFUN_LIFECYCLE_FIXTURE_TOKEN: token, NOMIFUN_LIFECYCLE_FIXTURE_MODE: mode } });
  report.application_evidence = application.evidenceDirectory;
  await journal();
  // A crash must be recorded as abnormal by the existing controller. The
  // native observer separately proves exact original children really exited.
  application.exit.then(value => { applicationResult = { normal: true, ...value }; }, error => { applicationResult = { normal: false, error: error.message }; });
  const bound = await until(async () => {
    const value = await json(join(data, 'port.json'));
    if (value && Number.isSafeInteger(value.pid) && value.pid > 1 && Number.isSafeInteger(value.port)) return value;
    if (applicationResult) throw Error('Actual App exited before backend readiness');
  }, 'the actual Desktop backend port', 120_000);
  report.host_pid = bound.pid; report.backend_port = bound.port;
  // Retain the exact ordinary teardown capability even if a supplied App
  // lacks the required fixture guard receipt and resource setup is refused.
  await run(paths['appkit-probe'], ['--prepare', String(bound.pid), paths.app, join(root, 'owned-appkit-request.json')], 'appkit-prepare');
  const isolation = await json(join(root, 'macos-lifecycle-fixture.json'));
  assert(isolation, 'The actual App must confirm the strict debug fixture guard before resource setup');
  for (const [key, expected] of Object.entries({ host_pid: bound.pid, token, data, work, shared_keyboard_started: false })) {
    assert.equal(isolation[key], expected, `The exact App fixture isolation receipt must confirm ${key}`);
  }
  report.fixture_isolation = isolation;
  const base = `http://127.0.0.1:${bound.port}`;
  const request = async (path, method = 'GET', body) => {
    const response = await fetch(`${base}${path}`, { method,
      headers: { 'content-type': 'application/json', ...(jwt ? { authorization: `Bearer ${jwt}` } : {}) },
      body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(20_000) });
    const value = await response.json(); assert(response.ok, `Product ${method} ${path}: ${response.status}`); return value.data ?? value;
  };
  const credentials = await json(join(data, 'wk-acceptance-auth.json'));
  const login = await request('/login', 'POST', credentials); jwt = login.token; assert.equal(jwt.split('.').length, 3);
  const owner = await request('/api/auth/user');
  const ownerId = owner.user?.user_id ?? owner.user_id ?? owner.id; assert.equal(typeof ownerId, 'string');
  try { report.actual_app_permissions = { observed: true, inventory: await request('/api/system/permissions') }; }
  catch (error) { report.actual_app_permissions = { observed: false, reason: error.message }; }
  await writeFile(join(root, 'agent-resources-owner.jwt'), jwt, { flag: 'wx', mode: 0o600 });
  await writeFile(join(root, 'agent-resources-auth.json'), JSON.stringify({ token, owner_id: ownerId, authenticated: true,
    host_local_setup: false, isolated_preparer_setup: true, password_login: true, local_trust_exported: false }), { flag: 'wx', mode: 0o600 });
  await writeFile(join(root, 'ready.json'), JSON.stringify({ token, host_pid: bound.pid, backend_port: bound.port,
    host_program: program, data_root: data, agent_resources: true, mode }), { flag: 'wx', mode: 0o600 });
  sidecar = await logged(process.execPath, [join(repository, 'scripts/validation/desktop-agent-resources.mjs'), '--root', root, '--bun-exe', paths.bun,
    ...(withComputer ? ['--with-computer'] : []),
    ...(retirementObserveMs ? ['--retirement-observe-ms', String(retirementObserveMs)] : [])], 'sidecar', isolatedEnvironment);
  const resources = await until(async () => {
    const progress = await json(join(root, 'agent-resources-progress.json'));
    if (progress?.failure) throw Error(progress.failure.message);
    if (!running(sidecar.child)) throw Error('Sidecar exited before real native resources');
    return await json(join(root, 'agent-resources-ready.json'));
  }, 'the original Bun pipe, PTY and inflight stdio MCP', 120_000);
  report.resources = resources.processes; report.session_id = resources.session_id; report.operation_id = resources.operation_id;
  const pairs = [{ pid: bound.pid, kind: 'host', expected_image: program }, ...resources.processes];
  if (withComputer) {
    const computer = resources.computer, status = computer?.status;
    assert.equal(computer?.enabled, true);
    assert.equal(computer.canonical_call?.is_error, false); assert.equal(computer.canonical_call.image_parts, 0);
    assert(computer.canonical_call.topic_count > 0);
    assert.equal(status?.running, true); assert.equal(status.phase, 'ready'); assert.equal(status.bound_sessions, 1);
    assert(Number.isSafeInteger(status.process_id) && status.process_id > 1);
    assert.match(status.generation, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    assert(!pairs.some(value => value.pid === status.process_id), 'The internal Computer worker must have a distinct native owner');
    Object.assign(report.computer, { internal_desktop_worker: { pid: status.process_id, generation: status.generation,
      image: report.build_identity.computer_worker, status_at_readiness: status,
      canonical_call: computer.canonical_call, frozen_definition: computer.definition } });
    pairs.push({ pid: status.process_id, kind: 'computer', expected_image: computerWorker });
  }
  observerLog = await open(join(root, 'native-observer.jsonl'), 'wx', 0o600);
  observer = spawn(paths.observer, pairs.flatMap(value => [String(value.pid), value.expected_image]), { stdio: ['pipe', 'pipe', 'inherit'] });
  observerDone = completion(observer); let buffer = '';
  observer.stdin.on('error', error => { observerFailure ??= error; });
  observer.stdout.setEncoding('utf8');
  observer.stdout.on('data', bytes => {
    try {
      writeSync(observerLog.fd, bytes); buffer += bytes;
      let end; while ((end = buffer.indexOf('\n')) >= 0) {
        const frame = { ...JSON.parse(buffer.slice(0, end)), observed_at: new Date().toISOString(),
          since_exit_action_ms: exitActionStarted === undefined ? null : Number(process.hrtime.bigint() - exitActionStarted) / 1_000_000 };
        buffer = buffer.slice(end + 1);
        report.native_events.push(frame); if (frame.event === 'ready') observerReady = true; if (frame.event === 'complete') observerComplete = true;
      }
    } catch (error) { observerFailure ??= error; }
  });
  await until(() => { if (observerFailure) throw observerFailure; if (!running(observer)) throw Error('Native observer failed before registration'); return observerReady; }, 'kernel registrations of the exact original process generations');
  const originalLauncher = resources.processes.find(resource => resource.kind === 'pty_launcher');
  const launcherIdentity = report.native_events.find(event => event.event === 'registered' && event.pid === originalLauncher.pid);
  const payloadIdentity = report.native_events.find(event => event.event === 'registered' && event.pid === originalLauncher.payload_pid);
  assert.equal(payloadIdentity.parent_pid, launcherIdentity.pid);
  assert.equal(payloadIdentity.parent_unique_id, launcherIdentity.unique_id, 'The original PTY payload must name this exact launcher kernel generation');
  assert.equal(payloadIdentity.original_parent_pid_version, launcherIdentity.pid_version);
  report.pty_kernel_parent_proof = { launcher: launcherIdentity, payload: payloadIdentity };
  if (withComputer) report.computer.kernel_parent_proof = macosComputerProcessProof(report.native_events, bound.pid, resources.computer.status.process_id);
  if (options.includes('--wk-inflight')) {
    const accepted = await request(`/api/agent-sessions/${fixture.session_id}/turns`, 'POST', {
      idempotency_key: `desktop-lifecycle-wk-${token}`, input: { content: fixture.prompt } });
    report.wk_operation_id = accepted.operation_id;
    await until(async () => {
      const response = await fetch(`${fixture.control}/status`, { signal: AbortSignal.timeout(5_000) }); const state = await response.json();
      if (state.failure) throw Error(state.failure);
      if (state.wk_evidence?.awaiting_terminal_release) { report.wk_inflight = state.wk_evidence; return true; }
    }, 'actual managed WK task held before host termination', 120_000);
  }
  if (options.includes('--stop-first')) {
    await publish('stop-agent.request', { token, host_pid: bound.pid,
      session_id: resources.session_id, operation_id: resources.operation_id });
    report.stop = await until(async () => {
      const progress = await json(join(root, 'agent-resources-progress.json'));
      if (progress?.failure) throw Error(progress.failure.message);
      if (!running(sidecar.child)) throw Error('Sidecar failed while verifying original Stop');
      return await json(join(root, 'stop-agent-ready.json'));
    }, 'original Turn Stop and automatic Runtime retirement', 60_000 + (retirementObserveMs ? 60_000 + retirementObserveMs : 0));
    if (retirementObserveMs) {
      assert.equal(report.stop.retirement_observation?.passed, true);
      assert(report.stop.retirement_observation.elapsed_ms >= retirementObserveMs);
    }
    const retained = pairs.flatMap((value, index) => index > 0 && !['mcp', 'computer'].includes(value.kind) ? [{ index, pid: value.pid }] : []);
    for (const process of retained) assert(!report.native_events.some(event => event.event === 'exit' && event.index === process.index), 'Original pipe/PTY must still be alive after this Turn Stop');
    report.stop_native = { exact_original_pipe_and_pty_retained: retained, observed_at: new Date().toISOString() };
  }
  if (options.includes('--matting-request')) {
    report.matting_request = { path: '/api/companion/matting-model', started_at: new Date().toISOString(),
      client_abort_requested: false, model_data_root: join(data, 'companion/models'),
      evidence_scope: 'Original authenticated HTTP request started before Quit and remained pending; model worker admission requires separate owner evidence' };
    mattingRequest = (async () => {
      try {
        const response = await fetch(`${base}/api/companion/matting-model`, { headers: { authorization: `Bearer ${jwt}` } });
        Object.assign(report.matting_request, { status: response.status, headers_received_at: new Date().toISOString() });
        let bytes = 0, errorText = '';
        for await (const part of response.body) {
          bytes += part.length;
          if (!response.ok && bytes <= 16_384) errorText += new TextDecoder().decode(part);
        }
        Object.assign(report.matting_request, { response_bytes: bytes, completed_at: new Date().toISOString(),
          shutdown_reason_observed: errorText.includes('Companion matting model supply is shutting down') });
      } catch (error) {
        report.matting_request.transport_error = { name: error.name, message: String(error.message).replaceAll(jwt ?? '\0', '[REDACTED]') };
      }
    })();
    await sleep(100);
    await request('/api/auth/status');
    report.matting_request.pending_before_exit_action = report.matting_request.headers_received_at === undefined;
  }
  if (withComputer) {
    const confirmed = await request('/api/computer/status');
    const original = resources.computer.status;
    for (const [key, expected] of Object.entries({ running: true, phase: 'ready', process_id: original.process_id,
      generation: original.generation, bound_sessions: options.includes('--stop-first') ? 0 : 1, last_error: null })) {
      assert.equal(confirmed[key], expected, `The original Computer generation must remain ${key} before Quit`);
    }
    report.computer.internal_desktop_worker.status_before_quit = confirmed;
    report.computer.kernel_parent_proof = macosComputerProcessProof(report.native_events, bound.pid, original.process_id);
  }
  const expectedLive = pairs.slice(1).filter(resource => !options.includes('--stop-first') || resource.kind !== 'mcp');
  for (const resource of expectedLive) assert(!report.native_events.some(event => event.event === 'exit' && event.pid === resource.pid),
    'An already exited original resource cannot substitute for a live resource at the quit action');
  report.original_resources_live_before_exit_action = expectedLive.map(resource => ({ pid: resource.pid, kind: resource.kind }));
  report.ready_at = new Date().toISOString(); await journal();
  await publish('gui-quit-ready.json', { root, mode, host_pid: bound.pid,
    app_path: paths.app, session_id: resources.session_id, operation_id: resources.operation_id, ready_at: report.ready_at });
  const guiQuitDeadline = Date.now() + 300_000;
  if (mode === 'cmd-q') report.gui_quit_deadline_at = new Date(guiQuitDeadline).toISOString();
  console.log(`NOMIFUN_MACOS_DESKTOP_RESOURCES READY mode=${mode} host=${bound.pid} root=${root}`);
  exitActionStarted = process.hrtime.bigint();
  report.exit_action = { mode, requested_at: new Date().toISOString(),
    source: mode === 'appkit' ? 'exact_owned_NSRunningApplication_terminate' : mode === 'cmd-q' ? 'awaiting_external_native_shortcut' : 'exact_kernel_generation_signal' };
  if (mode === 'appkit') await run(paths['appkit-probe'], [join(root, 'owned-appkit-request.json')], 'appkit-terminate');
  else if (mode === 'sigint' || mode === 'sigterm' || mode === 'parent-death') {
    await new Promise((accept, reject) => observer.stdin.write(
      mode === 'sigint' ? 'SIGINT\n' : mode === 'sigterm' ? 'SIGTERM\n' : 'SIGKILL\n',
      error => error ? reject(error) : accept()));
  }
  await until(() => { if (observerFailure) throw observerFailure; if (!running(observer) && !observerComplete) throw Error('Native observer exited before all original processes'); return observerComplete; }, 'all original native process NOTE_EXIT proofs', mode === 'cmd-q' ? Math.max(0, guiQuitDeadline - Date.now()) : 60_000);
  const observed = await observerDone; await observerLog.close(); observerLog = undefined; assert.equal(observed.code, 0); exitReached = true;
  if (withComputer) {
    const proof = macosComputerProcessProof(report.native_events, bound.pid, resources.computer.status.process_id, { exited: true });
    report.computer.native_exit_proof = proof;
    report.computer.worker_exit_code = proof.worker_exit_code;
    report.computer.exact_native_generation_exited = true;
  }
  if (['sigint', 'sigterm', 'parent-death'].includes(mode)) {
    const expectedSignal = mode === 'sigint' ? 2 : mode === 'sigterm' ? 15 : 9;
    const signals = report.native_events.filter(event => event.event === 'signal');
    assert.equal(signals.length, 1, 'An unexpected earlier App exit cannot substitute for the requested native signal');
    assert.equal(signals[0].pid, bound.pid); assert.equal(signals[0].signal, expectedSignal);
    assert.equal(signals[0].kernel_generation_checked, true);
  }
  report.exit_timing = { original_host_note_exit_after_ms: report.native_events.find(event => event.event === 'exit' && event.index === 0)?.since_exit_action_ms,
    all_original_note_exit_after_ms: report.native_events.find(event => event.event === 'complete')?.since_exit_action_ms,
    basis: 'driver monotonic clock at original native kqueue event receipt; Cmd-Q external delivery time is in its separate operator receipt' };
  await until(() => applicationResult, 'the launch controller final result', 30_000); report.application_exit = applicationResult;
  if (mode === 'parent-death') assert.equal(applicationResult.normal, false, 'Crash must not forge authenticated normal cleanup');
  else assert(applicationResult.normal && applicationResult.cleanupVerified && applicationResult.code === 0, 'Ordinary quit requires authenticated cleanup and actual exit');
  if (mattingRequest) {
    await mattingRequest;
    assert.equal(report.matting_request.pending_before_exit_action, true, 'A cached/completed model request does not prove in-flight cancellation');
    assert.equal(report.matting_request.status, 409, 'The original pending model-supply request must receive its real host-shutdown refusal');
    assert.equal(report.matting_request.shutdown_reason_observed, true);
    assert.equal(report.matting_request.transport_error, undefined, 'Client abort/socket loss cannot substitute for the original model owner cancellation response');
    report.matting_request.host_shutdown_response_confirmed = true;
  }
  if (mode === 'cmd-q') {
    const receipt = await until(() => json(join(root, 'gui-quit-evidence.json')), 'the native Cmd-Q operator evidence receipt', 30_000);
    assert.equal(receipt.host_pid, bound.pid); assert.equal(await realpath(receipt.app_path), paths.app);
    assert.equal(receipt.shortcut, 'Cmd+Q'); assert(Date.parse(receipt.observed_at) >= Date.parse(report.ready_at));
    assert(Array.isArray(receipt.evidence_paths) && receipt.evidence_paths.length > 0);
    for (const path of receipt.evidence_paths) { assert(isAbsolute(path)); assert((await stat(path)).isFile()); }
    report.native_cmd_q_evidence = receipt;
  }
  let backendClosed = false;
  try { await fetch(`${base}/api/auth/status`, { signal: AbortSignal.timeout(2_000) }); } catch { backendClosed = true; }
  assert(backendClosed, 'The original actual App loopback must close'); report.original_loopback_closed = true;
  assert(running(sentinel), 'The outside-tree sentinel must survive all App-owned cleanup');
  report.outside_tree_sentinel.preservation_verified = true;
  assert.equal(await readFile(join(root, 'fixture-sentinel'), 'utf8'), token);
  report.persistent_data = { database_bytes: (await stat(join(data, 'nomifun-backend.db'))).size,
    preparer_auth_exists: (await stat(join(data, 'wk-acceptance-auth.json'))).isFile(), fixture_sentinel_preserved: true };
  assert(report.persistent_data.database_bytes > 0);
  const preservedIds = [fixture.session_id, fixture.secondary_session_id, resources.session_id];
  for (const id of [...preservedIds, ownerId]) assert.match(id, /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  // Apple's system sqlite CLI can reject read-only WAL databases after close.
  // The existing Node runtime supplies a WAL-aware readOnly connection; never
  // use immutable=1, which would ignore an uncheckpointed crash-mode WAL.
  const database = new DatabaseSync(join(data, 'nomifun-backend.db'), { readOnly: true });
  let counts;
  try {
    counts = database.prepare('SELECT (SELECT count(*) FROM agent_sessions WHERE agent_session_id IN (?,?,?)) AS sessions, (SELECT count(*) FROM users WHERE user_id=?) AS owners, (SELECT count(*) FROM agent_events WHERE session_id=?) AS events')
      .get(...preservedIds, ownerId, resources.session_id);
  } finally { database.close(); }
  assert.equal(counts.sessions, 3); assert.equal(counts.owners, 1); assert(counts.events > 0);
  assert.equal(await readFile(legacyProfile, 'utf8'), preservation.legacy_sentinel_text);
  assert.equal(await sha256(join(data, 'storage-generation')), generationBefore);
  Object.assign(report.persistent_data, { exact_canonical_sessions_preserved: counts.sessions, normal_owner_preserved: true,
    original_resource_session_events: counts.events, legacy_profile_sentinel_preserved: true, storage_generation_preserved: true,
    readonly_database_reader: { runtime: process.version, api: 'node:sqlite DatabaseSync readOnly=true' } });
  report.passed = true; report.finished_at = new Date().toISOString();
} catch (error) {
  failure = error;
  report.failure = { name: error.name, message: String(error.message).replaceAll(jwt ?? '\0', '[REDACTED]') };
} finally {
  // Never scan/kill names. An incomplete actual App stays alive for review;
  // the caller must use its original admitted controller/native owner.
  const teardownErrors = [];
  const settle = async (owner, action) => {
    try { await action(); }
    catch (error) { teardownErrors.push({ owner, message: String(error.message).replaceAll(jwt ?? '\0', '[REDACTED]') }); }
  };
  if (failure && application && !applicationResult) {
    // An already failed fixture may request normal cleanup of its exact App.
    // This is a separately recorded teardown, never a successful test action.
    await settle('failed-app-normal-cleanup', async () => {
      if (!await json(join(root, 'owned-appkit-request.json'))) return;
      await run(paths['appkit-probe'], [join(root, 'owned-appkit-request.json')], 'failed-app-normal-cleanup');
      await until(() => applicationResult, 'failed fixture App ordinary cleanup receipt', 30_000);
      report.failure_app_cleanup = applicationResult;
    });
  }
  const hostExitConfirmed = applicationResult?.normal === true
    || report.native_events.some(event => event.event === 'exit' && event.index === 0);
  if (exitReached || !application || hostExitConfirmed) {
    if (observer && !observerComplete) {
      report.unconfirmed_original_resources = report.native_events.filter(event => event.event === 'registered'
        && !report.native_events.some(exit => exit.event === 'exit' && exit.index === event.index));
      // This child only observes. Ending our own failed observer neither
      // signals the actual resources nor turns missing NOTE_EXIT into proof.
      await settle('native-observer', async () => { if (running(observer)) observer.kill('SIGTERM'); await observerDone; });
    }
    if (observerLog) await settle('native-log', async () => { await observerLog.close(); observerLog = undefined; });
    if (sidecar) await settle('model-sidecar', async () => {
      if (running(sidecar.child)) sidecar.child.kill('SIGTERM');
      const result = await sidecar.done;
      assert.equal(result.code, 0, 'The strict real-tool sidecar must finish without a failure');
      assert.equal(result.signal, null, 'The sidecar must handle its own normal cleanup signal');
      const progress = await json(join(root, 'agent-resources-progress.json'));
      assert(progress && !progress.failure, 'The strict sidecar must retain no unreported failure');
      report.final_sidecar = { stage: progress.stage, model_requests: progress.model_requests.length, code: result.code, signal: result.signal };
    });
    if (preparer) await settle('dataset-preparer', async () => { if (running(preparer.child)) preparer.child.kill('SIGTERM'); await preparer.done; preparerStopped = true; });
    if (sentinel) await settle('outside-tree-sentinel', async () => { if (running(sentinel)) sentinel.kill('SIGTERM'); await sentinelDone; });
  } else {
    report.retained_for_review = { reason: 'Actual App exit is not yet confirmed; its original controller and isolated fixture remain owned by this running driver',
      host_pid: report.host_pid, application_evidence: report.application_evidence,
      exact_appkit_request: join(root, 'owned-appkit-request.json'), fixture_root: root };
  }
  await settle('owner-jwt', async () => {
    try { await unlink(join(root, 'agent-resources-owner.jwt')); report.owner_jwt_removed = true; }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
  });
  jwt = undefined;
  if (report.passed && preparerStopped) {
    await settle('preparer-password', async () => { await unlink(join(data, 'wk-acceptance-auth.json')); report.preparer_password_removed_after_exit = true; });
  }
  if (teardownErrors.length) { report.teardown_errors = teardownErrors; report.passed = false; failure ??= Error('Fixture teardown has unconfirmed owners; inspect result.json'); }
  await journal();
  console.log(`NOMIFUN_MACOS_DESKTOP_RESOURCES ${report.passed ? 'PASS' : 'FAIL'} mode=${mode} root=${root}`);
}
if (failure) { console.error(report.failure?.message ?? failure.message); process.exitCode = 1; }
