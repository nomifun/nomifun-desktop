import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { isAbsolute, join, relative } from 'node:path';
import { certificate, freePort, Processes, request, until } from './connectivity-stack-helpers.mjs';
import { nativeAcceptance } from './connectivity-stack-native.mjs';

/** Diagnostic subset only; final delivery still runs the complete stack. */
export async function runNativeOnly(config, root) {
  await freePort('tcp', 25811);
  const parent = realpathSync(tmpdir()), fixture = mkdtempSync(join(parent, 'nomifun-connectivity-native-'));
  const marker = randomBytes(32).toString('hex'); writeFileSync(join(fixture, '.runner-owned'), marker);
  mkdirSync(config.output, { recursive: true });
  const processes = new Processes(fixture), allChildren = [], resources = [];
  const report = { schema_version: 1, diagnostic_subset: true, environment: 'same-computer-loopback',
    evidence_source: 'real-production-desktop-server-relay-manager-and-relay-binaries', started_at: new Date().toISOString(), result: 'running', checks: [],
    binaries: Object.fromEntries(['relayBin', 'nativeHarnessBin'].map(name => [name, { path: config[name], sha256: createHash('sha256').update(readFileSync(config[name])).digest('hex') }])),
    boundaries: ['Core Mobile/canonical Turn cases intentionally omitted in this diagnostic subset; use full runner for delivery', 'Tauri window UI and phone hardware not measured'],
    processes_cleaned: false, fixture_removed: false };
  const check = async (name, action) => {
    const start = Date.now(); process.stdout.write(`CHECK ${name}\n`);
    try { const evidence = await action(); report.checks.push({ name, result: 'pass', duration_ms: Date.now() - start, evidence }); return evidence; }
    catch (error) { report.checks.push({ name, result: 'fail', duration_ms: Date.now() - start, error: processes.redact(error.message) }); throw error; }
  };
  try {
    const udp = await freePort('udp'), consolePort = await freePort(), sharedPort = await freePort();
    const consoleUrl = `http://127.0.0.1:${consolePort}`;
    const directory = join(fixture, 'relay'); mkdirSync(directory);
    const cert = certificate(fixture, config.opensslBin); writeFileSync(join(directory, 'relay-cert.pem'), cert.cert); writeFileSync(join(directory, 'relay-key.pem'), cert.key, { mode: 0o600 });
    const relay = processes.start('native-only-relay', config.relayBin, ['-data-dir', directory, '-bind', `127.0.0.1:${udp}`, '-advertise', `127.0.0.1:${udp}`, '-console', `127.0.0.1:${consolePort}`, '-shared', `127.0.0.1:${sharedPort}`, '-desktop-bootstrap-url', `${consoleUrl}/api/bootstrap/desktop/v2`, '-desktop-dev-insecure', '-drain', '1s']);
    allChildren.push(relay);
    await until('real native-only Relay starts', async () => (await request(`${consoleUrl}/healthz`)).status === 200);
    const password = `Local-${randomBytes(32).toString('hex')}!`; processes.secrets.add(password);
    const setup = await request(`${consoleUrl}/api/setup`, { method: 'POST', headers: { Origin: consoleUrl }, body: { username: 'native-owner', password } });
    assert.equal(setup.status, 200); assert(setup.json.csrf_token);
    const cookies = (setup.headers['set-cookie'] ?? []).map(cookie => cookie.split(';')[0]).join('; ');
    processes.secrets.add(cookies); processes.secrets.add(setup.json.csrf_token);
    const relayAdmin = (path, options = {}) => request(`${consoleUrl}${path}`, { ...options, headers: { Cookie: cookies, Origin: consoleUrl, 'X-NFR-CSRF': setup.json.csrf_token, ...options.headers } });
    report.native_managed_runtime = await nativeAcceptance({ config, root, fixture, processes, allChildren, resources, check, relayAdmin });
    report.result = 'pass';
  } catch (error) { report.result = 'fail'; report.error = processes.redact(error.message); }
  finally {
    const failures = [];
    for (const resource of resources.reverse()) try { await resource.close(); } catch (error) { failures.push(error); }
    try { await processes.close(); } catch (error) { failures.push(error); }
    report.processes_cleaned = processes.children.size === 0;
    for (const child of allChildren) writeFileSync(join(config.output, `${child.label}.log`), child.output());
    if (!config.keepFixture && !failures.length) {
      const target = realpathSync(fixture), within = relative(parent, target);
      assert(within && !within.startsWith('..') && !isAbsolute(within) && readFileSync(join(target, '.runner-owned'), 'utf8') === marker, 'Native fixture ownership check failed');
      rmSync(target, { recursive: true, force: true }); report.fixture_removed = true;
    } else report.fixture = fixture;
    if (failures.length) { report.result = 'fail'; report.cleanup_error = 'Native subset cleanup failed; fixture retained'; }
    report.finished_at = new Date().toISOString(); writeFileSync(join(config.output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  }
  process.stdout.write(`${report.result.toUpperCase()} ${join(config.output, 'report.json')}\n`);
  return report;
}
