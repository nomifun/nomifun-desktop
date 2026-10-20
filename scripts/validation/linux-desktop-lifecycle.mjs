#!/usr/bin/env node
import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { createHash, randomBytes, randomUUID } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { access, lstat, mkdir, readdir, readFile, readlink, realpath, rename, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { delimiter, dirname, isAbsolute, join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { readNativeAnnouncement, validateNativeDataset } from './global-os-native.mjs';
import { sourceIdentity } from '../lib/source-identity.mjs';
import { verifyLinuxNativeApp } from './global-os-linux-cases.mjs';

const MODES = { sigterm: 'SIGTERM', sigint: 'SIGINT', 'parent-death': 'SIGKILL', stop: 'SIGTERM' };
const here = dirname(fileURLToPath(import.meta.url));
const repo = dirname(dirname(here));
const execute = promisify(execFile);
const delay = (ms) => new Promise((done) => setTimeout(done, ms));
const option = (key) => { const index = process.argv.indexOf(key); return index < 0 ? undefined : process.argv[index + 1]; };
const sha256 = (path) => new Promise((done, fail) => {
  const hash = createHash('sha256'); createReadStream(path).on('error', fail).on('data', (chunk) => hash.update(chunk)).on('end', () => done(hash.digest('hex')));
});
const until = async (check, label, budget) => {
  const end = Date.now() + budget;
  while (Date.now() < end) { const value = await check(); if (value) return value; await delay(100); }
  throw new Error(`Timed out waiting for ${label}`);
};

const procStat = async (pid) => {
  const raw = await readFile(`/proc/${pid}/stat`, 'utf8');
  const close = raw.lastIndexOf(')');
  const values = raw.slice(close + 2).trim().split(/\s+/);
  return { pid, comm: raw.slice(raw.indexOf('(') + 1, close), state: values[0], ppid: Number(values[1]), pgrp: Number(values[2]),
    session: Number(values[3]), tty_nr: Number(values[4]), threads: Number(values[17]), start_time: Number(values[19]) };
};
const processTable = async () => {
  const rows = new Map();
  for (const name of await readdir('/proc')) {
    if (!/^\d+$/.test(name)) continue;
    try { rows.set(Number(name), await procStat(Number(name))); }
    catch (error) { if (!['ENOENT', 'ESRCH'].includes(error.code)) throw error; }
  }
  return rows;
};
const withIdentity = async (row) => ({ ...row, uid: (await lstat(`/proc/${row.pid}`)).uid,
  exe: await readlink(`/proc/${row.pid}/exe`).catch((error) => { if (['ENOENT', 'EACCES'].includes(error.code)) return null; throw error; }) });
const descendants = (table, root) => {
  const children = new Map();
  for (const row of table.values()) { if (!children.has(row.ppid)) children.set(row.ppid, []); children.get(row.ppid).push(row); }
  const found = []; const queue = [...(children.get(root) ?? [])];
  while (queue.length) { const row = queue.shift(); found.push(row); queue.push(...(children.get(row.pid) ?? [])); }
  return found;
};
const ancestry = (table, pid, host) => {
  const chain = []; let current = table.get(pid);
  while (current && current.pid !== host) { chain.push(current); current = table.get(current.ppid); }
  assert(current, `PID ${pid} is not a descendant of the exact App host`);
  return chain;
};
const descriptorCount = async (pid) => (await readdir(`/proc/${pid}/fd`)).length;
const stillOriginal = async ({ pid, start_time: started }) => {
  try { const row = await procStat(pid); return row.start_time === started && !['Z', 'X'].includes(row.state); }
  catch (error) { if (['ENOENT', 'ESRCH'].includes(error.code)) return false; throw error; }
};
const existingExecutable = async (name) => {
  for (const entry of (process.env.PATH ?? '').split(delimiter)) {
    const candidate = join(entry, name);
    try { await access(candidate); if ((await stat(candidate)).isFile()) return await realpath(candidate); }
    catch (error) { if (!['ENOENT', 'ENOTDIR', 'EACCES'].includes(error.code)) throw error; }
  }
  throw new Error(`The validation requires an existing ${name}; it never installs a runtime`);
};


const SERVICE_CLOSE = `import os, signal, sys
pid, started, exe = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
fd = os.pidfd_open(pid, 0)
fields = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
assert int(fields[19]) == started and os.readlink(f"/proc/{pid}/exe") == exe and os.stat(f"/proc/{pid}").st_uid == os.getuid()
assert b"--keyboard-service" in open(f"/proc/{pid}/cmdline", "rb").read()
signal.pidfd_send_signal(fd, signal.SIGTERM)
`;

async function main() {
  assert.equal(process.platform, 'linux', 'Run on the actual Linux native App host');
  assert(process.getuid() > 0, 'Run as the ordinary native App user');
  const mode = option('--mode'); assert(Object.hasOwn(MODES, mode ?? ''), `--mode must be one of ${Object.keys(MODES).join(', ')}`);
  const app = option('--app'); assert(app && isAbsolute(app) && await realpath(app) === app, '--app must be the canonical frozen nomifun-desktop');
  const ownerTool = option('--owner-tool'); assert(ownerTool && isAbsolute(ownerTool), '--owner-tool must name the built global_os_fixture_owner example');
  const evidenceRoot = option('--evidence'); assert(evidenceRoot && isAbsolute(evidenceRoot), '--evidence must be an absolute new directory');
  await mkdir(evidenceRoot, { recursive: false, mode: 0o700 });
  const uuidBytes = randomBytes(16);
  uuidBytes.writeUIntBE(Date.now(), 0, 6); uuidBytes[6] = uuidBytes[6] & 0x0f | 0x70; uuidBytes[8] = uuidBytes[8] & 0x3f | 0x80;
  const hex = uuidBytes.toString('hex');
  const token = `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
  const root = `/tmp/nomifun-global-os-owner-${token}`;
  const data = join(root, 'native-data');
  const sessionRoot = `/tmp/nomifun-global-os-x11-${randomUUID()}`;
  const evidence = { schema: 'nomifun.linux-desktop-lifecycle.v1', mode, token, root, session_root: sessionRoot, started_at: new Date().toISOString(),
    app: { path: app, sha256: await sha256(app), size: (await stat(app)).size, mtime: (await stat(app)).mtime.toISOString() },
    source: await sourceIdentity(repo).catch((error) => ({ error: String(error?.message ?? error) })), passed: false, stage: 'fixture' };
  const journal = () => writeFile(join(evidenceRoot, 'lifecycle-report.json'), `${JSON.stringify(evidence, null, 2)}\n`, { mode: 0o600 });
  const optionalJson = async (path) => { try { return JSON.parse(await readFile(path, 'utf8')); } catch (error) { if (error.code === 'ENOENT') return undefined; throw error; } };
  const jsonRequired = async (path) => { const value = await optionalJson(path); assert(value, `${path} must exist`); return value; };
  let launcher, launcherExit, sentinel, jwt, sidecar, sidecarExit;
  let services = [];
  const launcherLines = []; const sidecarErrors = [];
  try {
    evidence.owner_prepare = JSON.parse((await execute(ownerTool, ['--prepare-empty-fixture', data])).stdout.trim());
    await symlink(data, join(root, 'data'));
    const dataset = await validateNativeDataset(data);
    await mkdir(join(root, 'work'), { mode: 0o700 });
    const artifacts = join(root, 'agent-resource-fixtures');
    evidence.stage = 'launch'; await journal();
    launcher = spawn('python3', [join(here, 'global-os-linux-x11-session.py'), '--app', app, '--native-data-root', data, '--session-root', sessionRoot],
      { stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env } });
    launcherExit = new Promise((done) => launcher.once('exit', (code, signal) => done({ code, signal })));
    createInterface({ input: launcher.stdout }).on('line', (line) => { try { launcherLines.push(JSON.parse(line)); } catch { launcherLines.push({ raw: line }); } });
    const launcherErrors = []; createInterface({ input: launcher.stderr }).on('line', (line) => launcherErrors.push(line));
    evidence.launcher_stderr = launcherErrors;
    const announcement = await until(async () => {
      if (launcher.exitCode !== null) throw new Error(`Private X11 launcher exited early: ${launcherErrors.join(' | ')}`);
      try { return await readNativeAnnouncement(dataset); } catch (error) { if (['ENOENT', 'NATIVE_FIXTURE_METADATA_CHANGED'].includes(error.code) || error instanceof SyntaxError) return undefined; throw error; }
    }, 'the native App port announcement', 180_000);
    const hostPid = announcement.pid; const base = `http://127.0.0.1:${announcement.port}`;
    evidence.host = { ...(await verifyLinuxNativeApp({ pid: hostPid, port: announcement.port, data, uid: process.getuid() })), port: announcement.port };
    const hostStat = await withIdentity(await procStat(hostPid));
    assert.equal(hostStat.exe, app, 'The announcement must identify the exact frozen App executable');
    assert.equal(hostStat.ppid, launcher.pid, 'The App must be the private launcher child');
    evidence.host.start_time = hostStat.start_time; evidence.host.baseline = { fds: await descriptorCount(hostPid), threads: hostStat.threads };
    evidence.stage = 'owner'; await journal();
    evidence.owner_initialize = JSON.parse((await execute(ownerTool, ['--initialize-running-fixture', data])).stdout.trim());
    evidence.owner_login = JSON.parse((await execute(process.execPath, [join(here, 'global-os-linux-fixture-login.mjs'), '--native-data-root', data])).stdout.trim());
    jwt = (await readFile(join(root, 'owner-jwt'), 'utf8')).trim(); assert.equal(jwt.split('.').length, 3, 'Expected an ordinary owner login JWT');
    const api = async (method, path, body) => {
      const response = await fetch(`${base}${path}`, { method, headers: { 'content-type': 'application/json', Authorization: `Bearer ${jwt}` },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }), signal: AbortSignal.timeout(20_000) });
      const text = await response.text(); let value;
      try { value = JSON.parse(text); } catch { throw new Error(`${method} ${path}: ${response.status}, non-JSON response`); }
      (evidence.api_calls ??= []).push({ method, path, status: response.status });
      if (!response.ok) throw new Error(`${method} ${path}: ${response.status} ${JSON.stringify(value)}`);
      return value.data ?? value;
    };
    evidence.owner_identity = (await api('GET', '/api/auth/user')).user?.user_id ?? null;

    evidence.stage = 'agent-resources'; await journal();
    const bun = await existingExecutable('bun');
    await writeFile(join(root, 'fixture-sentinel'), token, { flag: 'wx', mode: 0o600 });
    await writeFile(join(root, 'agent-resources-owner.jwt'), `${jwt}\n`, { flag: 'wx', mode: 0o600 });
    await writeFile(join(root, 'agent-resources-auth.json'), JSON.stringify({ token, owner_id: evidence.owner_identity, authenticated: true,
      host_local_setup: false, isolated_preparer_setup: true, password_login: true, local_trust_exported: false,
      credential_source: 'isolated-preparer-and-normal-desktop-password-login' }), { flag: 'wx', mode: 0o600 });
    await writeFile(join(root, 'ready.json'), JSON.stringify({ token, host_pid: hostPid, backend_port: announcement.port,
      host_program: app, data_root: data, agent_resources: true, mode, fixture_kind: 'nomifun-global-os-owner' }), { flag: 'wx', mode: 0o600 });
    sidecar = spawn(process.execPath, [join(here, 'desktop-agent-resources.mjs'), '--root', root, '--bun-exe', bun],
      { stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env } });
    sidecarExit = new Promise((done) => sidecar.once('exit', (code, signal) => done({ code, signal })));
    createInterface({ input: sidecar.stdout }).on('line', (line) => sidecarErrors.push(line));
    createInterface({ input: sidecar.stderr }).on('line', (line) => sidecarErrors.push(line));
    evidence.sidecar_log = sidecarErrors;
    const resources = await until(async () => {
      const progress = await optionalJson(join(root, 'agent-resources-progress.json'));
      if (progress?.failure) throw new Error(progress.failure.message);
      if (sidecar.exitCode !== null && sidecar.exitCode !== 0) throw new Error(`Agent-resource sidecar exited early: ${sidecarErrors.join(' | ')}`);
      return await optionalJson(join(root, 'agent-resources-ready.json'));
    }, 'the original Bun pipe, PTY and inflight stdio MCP', 120_000);
    const session = resources.session_id; const operation = resources.operation_id;
    Object.assign(evidence, { session_id: session, operation_id: operation, mcp_server_id: resources.mcp_server_id,
      model_requests: resources.model_requests });

    const table = await processTable(); const native = {};
    for (const entry of resources.processes) {
      assert.equal(entry.expected_image ? await realpath(entry.expected_image) : bun, bun, 'Each fixture child must be the exact Bun image');
      const chain = await Promise.all(ancestry(table, entry.pid, hostPid).map(withIdentity));
      native[entry.kind] = { pid: entry.pid, process_id: entry.process_id, mcp_request_id: entry.mcp_request_id, chain };
    }
    for (const kind of ['pipe', 'pty', 'mcp']) assert(native[kind], `The sidecar must publish the original ${kind} process`);
    const all = await Promise.all(descendants(table, hostPid).map(withIdentity));
    const servicePids = new Set((await Promise.all(all.map(async (row) =>
      row.exe === app && (await readFile(`/proc/${row.pid}/cmdline`, 'utf8').catch(() => '')).includes('--keyboard-service') ? row.pid : null))).filter(Boolean));
    const agentPids = new Set(Object.values(native).flatMap((item) => item.chain.map((row) => row.pid)));
    const groups = new Set(Object.values(native).map((item) => item.pid));
    const tree = all.map((row) => ({ ...row, class: agentPids.has(row.pid) ? 'agent' : row.comm.startsWith('WebKit') ? 'webkit'
      : servicePids.has(row.pid) ? 'service'
      : row.exe === app && row.ppid === hostPid && (groups.has(row.pgrp) || row.pgrp === hostPid) ? 'watchdog' : 'other' }));
    services = tree.filter((row) => row.class === 'service');
    const watchdogs = tree.filter((row) => row.class === 'watchdog');
    assert.equal(watchdogs.length, 3, 'Each original Session process must have exactly one forked process-runtime watchdog');
    for (const kind of ['pipe', 'mcp']) {
      const paired = watchdogs.filter((row) => row.pgrp === native[kind].pid);
      assert.equal(paired.length, 1, `The ${kind} watchdog must share its exact child process group`);
      native[kind].watchdog = paired[0];
    }
    const ptyLeader = table.get(native.pty.pid);
    assert(ptyLeader.session === native.pty.pid && ptyLeader.pgrp === native.pty.pid, 'The PTY child must lead its own session');
    const remainingWatchdogs = watchdogs.filter((row) => row !== native.pipe.watchdog && row !== native.mcp.watchdog);
    assert(remainingWatchdogs.length === 1 && remainingWatchdogs[0].pgrp === hostPid, 'The PTY watchdog must stay outside the PTY session');
    native.pty.watchdog = remainingWatchdogs[0];
    const owned = (kind) => [...native[kind].chain, native[kind].watchdog];
    const processScript = join(artifacts, 'held-agent-process.mjs');
    sentinel = spawn(bun, [processScript, join(root, 'sentinel-pid.json'), token, String(hostPid), 'sentinel'], { detached: true, stdio: ['pipe', 'ignore', 'ignore'] });
    const sentinelProof = await until(() => optionalJson(join(root, 'sentinel-pid.json')), 'the tree-external sentinel', 15_000);
    const sentinelRow = await withIdentity(await procStat(sentinelProof.pid)); assert.equal(sentinelRow.exe, bun);
    Object.assign(evidence, { stage: 'ready', native, tree, sentinel: sentinelRow,
      host_ready: { fds: await descriptorCount(hostPid), threads: (await procStat(hostPid)).threads }, model_requests_ready: evidence.model_requests.length });
    await journal();

    const launchCounts = async () => Object.fromEntries(await Promise.all(['pipe', 'pty', 'mcp'].map(async (kind) =>
      [kind, (await readFile(join(artifacts, `${kind}-pid.json.launches.jsonl`), 'utf8')).trim().split('\n').length])));
    const watchRow = (row, label, expect) => ({ label, pid: row.pid, start_time: row.start_time, exe: row.exe ?? undefined, expect });
    const hostEntry = (signal) => ({ label: 'app', pid: hostPid, start_time: hostStat.start_time, exe: app, signal });
    const observe = async (label, spec, afterGo) => {
      const satisfied = [];
      spec.watch = (await Promise.all(spec.watch.map(async (row) =>
        row.expect === 'exit' && !(await stillOriginal(row)) ? (satisfied.push(row), null) : row))).filter(Boolean);
      const specPath = join(evidenceRoot, `${label}-spec.json`), reportPath = join(evidenceRoot, `${label}-observation.json`);
      await writeFile(specPath, `${JSON.stringify({ ...spec, satisfied }, null, 2)}\n`, { flag: 'wx', mode: 0o600 });
      const observer = spawn('python3', [join(here, 'linux-process-exit-observer.py'), '--spec', specPath, '--report', reportPath], { stdio: ['pipe', 'pipe', 'inherit'] });
      const done = new Promise((resolve) => observer.once('exit', resolve));
      const lines = createInterface({ input: observer.stdout })[Symbol.asyncIterator]();
      const armed = await lines.next(); assert(!armed.done && JSON.parse(armed.value).status === 'armed', `Observer ${label} did not arm`);
      observer.stdin.end('go\n');
      const extra = afterGo ? await afterGo() : undefined;
      await done; const report = JSON.parse(await readFile(reportPath, 'utf8'));
      evidence.observations ??= {}; evidence.observations[label] = { report, extra };
      return report;
    };
    const quitAll = async (label, signal, remaining) => {
      const report = await observe(label, { host: hostEntry(signal), deadline_ms: 90_000,
        watch: [...remaining.map((row) => watchRow(row, `${row.class}:${row.comm}:${row.pid}`, row.class === 'service' ? 'alive' : 'exit')), watchRow(sentinelRow, 'sentinel', 'alive')] });
      assert(report.pass, `${label}: an app-owned process did not exit or the tree-external sentinel did not survive`);
    };

    if (mode === 'stop') {
      const stopReport = await observe('stop-agent', { deadline_ms: 90_000, watch: [watchRow(hostStat, 'app', 'alive'),
        ...owned('mcp').map((row) => watchRow(row, `mcp:${row.comm}:${row.pid}`, 'exit')),
        ...owned('pipe').map((row) => watchRow(row, `pipe:${row.comm}:${row.pid}`, 'exit')),
        ...owned('pty').map((row) => watchRow(row, `pty:${row.comm}:${row.pid}`, 'alive')),
        ...tree.filter((row) => row.class === 'service').map((row) => watchRow(row, `service:${row.comm}:${row.pid}`, 'alive')),
        watchRow(sentinelRow, 'sentinel', 'alive')] }, async () => {
        await writeFile(join(root, 'stop-agent.request'), JSON.stringify({ token, host_pid: hostPid, session_id: session, operation_id: operation }), { flag: 'wx' });
        return await until(async () => {
          const progress = await optionalJson(join(root, 'agent-resources-progress.json'));
          if (progress?.failure) throw new Error(progress.failure.message);
          if (sidecar.exitCode !== null && sidecar.exitCode !== 0) throw new Error(`Sidecar failed while verifying original Stop: ${sidecarErrors.join(' | ')}`);
          const stopped = await optionalJson(join(root, 'stop-agent-ready.json'));
          if (!stopped) return undefined;
          const requestPath = join(root, 'stop-process.request');
          if (!(await optionalJson(requestPath))) {
            await writeFile(requestPath, JSON.stringify({ token, host_pid: hostPid, session_id: session, operation_id: operation }), { flag: 'wx' });
            return undefined;
          }
          return await optionalJson(join(root, 'stop-process-ready.json'));
        }, 'original Turn Stop, exact pipe process Stop and automatic Runtime retirement', 60_000);
      });
      assert(stopReport.pass, 'Stop must retire only its exact owners while the original sibling processes remain');
      const stopped = new Set([...owned('mcp'), ...owned('pipe')].map((row) => row.pid));
      await quitAll('quit-after-stop', 'SIGTERM', tree.filter((row) => !stopped.has(row.pid)));
    } else {
      await quitAll(mode, MODES[mode], tree);
    }
    evidence.stage = 'host-exited'; await journal();
    const infraPids = new Set();
    for (const file of [join(sessionRoot, 'environment.json'), join(sessionRoot, 'cleanup.json')]) {
      const doc = await optionalJson(file);
      const collect = (value) => { if (Number.isSafeInteger(value) && value > 0) infraPids.add(value); else if (value && typeof value === 'object') for (const item of Object.values(value)) collect(item); };
      collect(doc);
    }
    const orphanTable = await processTable();
    const childrenOf = new Map();
    for (const row of orphanTable.values()) {
      if (!childrenOf.has(row.ppid)) childrenOf.set(row.ppid, []);
      childrenOf.get(row.ppid).push(row);
    }
    const infraEnv = await optionalJson(join(sessionRoot, 'environment.json')) ?? {};
    const infraSessions = new Set([infraEnv.xvfb_pid, infraEnv.window_manager_pid, infraEnv.session_bus_pid].filter((pid) => Number.isSafeInteger(pid)));
    const orphans = [];
    const seen = new Set();
    const queue = [launcher.pid, hostPid].filter((pid) => Number.isSafeInteger(pid));
    while (queue.length) {
      const pid = queue.shift();
      for (const child of childrenOf.get(pid) ?? []) {
        if (seen.has(child.pid) || infraPids.has(child.pid) || infraSessions.has(child.session) || infraSessions.has(child.pgrp)) continue;
        seen.add(child.pid); orphans.push(child); queue.push(child.pid);
      }
    }
    const unexpected = orphans.filter((row) => row.pid !== sentinel.pid && row.state !== 'Z' && !servicePids.has(row.pid));
    evidence.orphan_scan = { launcher_pid: launcher.pid, infra_pids: [...infraPids].sort((left, right) => left - right),
      orphans: await Promise.all(unexpected.map(async (row) => ({ ...row, original: await stillOriginal(row) }))) };
    assert.equal(unexpected.length, 0, `Orphans must not survive the original App host: ${JSON.stringify(unexpected.map((row) => [row.pid, row.comm]))}`);
    const exited = await Promise.race([launcherExit, delay(120_000).then(() => { throw new Error('The private launcher did not finish after the App exit'); })]);
    const final = launcherLines.findLast((line) => line.status === 'exited');
    evidence.launcher = { exit: exited, final, cleanup: JSON.parse(await readFile(join(sessionRoot, 'cleanup.json'), 'utf8')) };
    const expectedCode = mode === 'parent-death' ? -9 : 0;
    assert.equal(final?.app_exit_code, expectedCode, `The App exit code must be ${expectedCode} for ${mode}`);
    assert.equal(evidence.launcher.cleanup.status, 'complete', 'The private launcher must reap its own X11/bus infrastructure');
    evidence.after_exit = { remaining_original: (await Promise.all(tree.filter((row) => row.class !== 'service').map(async (row) => (await stillOriginal(row)) ? row.pid : null))).filter(Boolean),
      service_survivors: (await Promise.all(tree.filter((row) => row.class === 'service').map(async (row) => (await stillOriginal(row)) ? row.pid : null))).filter(Boolean),
      sentinel_alive: await stillOriginal(sentinelRow), model_requests: evidence.model_requests.length, launches: await launchCounts(),
      database_bytes: (await stat(join(data, 'nomifun-backend.db'))).size };
    assert.deepEqual(evidence.after_exit.remaining_original, [], 'No original app-owned process may survive the host');
    assert(evidence.after_exit.sentinel_alive, 'The tree-external same-image sentinel must survive');
    evidence.passed = true; evidence.stage = 'passed';
  } catch (error) {
    evidence.failure = { message: String(error?.message ?? error).replaceAll(jwt ?? '\0', '[REDACTED]') };
    evidence.stage = `failed:${evidence.stage}`;
    process.exitCode = 1;
  } finally {
    if (sidecar && sidecar.exitCode === null && sidecar.signalCode === null) {
      sidecar.kill('SIGTERM');
      await Promise.race([sidecarExit, delay(30_000)]);
      evidence.sidecar_interrupted = { exit: sidecar.exitCode, signal: sidecar.signalCode };
    }
    if (launcher && launcher.exitCode === null && launcher.signalCode === null) {
      launcher.kill('SIGTERM');
      await Promise.race([launcherExit, delay(90_000)]);
      evidence.launcher_interrupted = { exit: launcher.exitCode, signal: launcher.signalCode, lines: launcherLines };
    }
    if (sentinel && sentinel.exitCode === null && sentinel.signalCode === null) {
      const gone = new Promise((done) => sentinel.once('exit', done));
      sentinel.kill('SIGTERM');
      const ended = await Promise.race([gone.then(() => true), delay(30_000).then(() => false)]);
      if (ended) evidence.sentinel_closed_by_its_own_owner = true;
      else evidence.sentinel_close_timeout = { exit: sentinel.exitCode, signal: sentinel.signalCode };
    }
    for (const row of services) {
      if (!(await stillOriginal(row))) continue;
      const closed = await execute('python3', ['-c', SERVICE_CLOSE, String(row.pid), String(row.start_time), app]).then(() => true, () => false);
      const gone = closed && await until(async () => !(await stillOriginal(row)), 'the run-owned keyboard service', 10_000).then(() => true, () => false);
      (evidence.services_closed ??= []).push({ pid: row.pid, signalled: closed, exited: gone });
    }
    for (const secret of ['owner-jwt', 'credential.json', 'agent-resources-owner.jwt']) {
      await rm(join(root, secret), { force: true });
      evidence.secrets_removed = [...(evidence.secrets_removed ?? []), secret];
    }
    evidence.finished_at = new Date().toISOString();
    await journal();
    const temporary = join(evidenceRoot, '.summary.tmp');
    await writeFile(temporary, `${JSON.stringify({ mode, passed: evidence.passed, stage: evidence.stage, failure: evidence.failure, root, session_root: sessionRoot }, null, 2)}\n`, { flag: 'wx' });
    await rename(temporary, join(evidenceRoot, 'summary.json'));
    console.log(JSON.stringify({ mode, passed: evidence.passed, stage: evidence.stage, evidence: evidenceRoot }));
  }
}

await main();
