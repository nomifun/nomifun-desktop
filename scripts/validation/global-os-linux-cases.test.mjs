import assert from 'node:assert/strict';
import test from 'node:test';
import { randomUUID } from 'node:crypto';
import { createServer } from 'node:net';
import { execFile } from 'node:child_process';
import { mkdir, rmdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { createLinuxSystemScenario, observeLinuxListener, verifyLinuxNativeApp } from './global-os-linux-cases.mjs';

test('Linux listener evidence binds the exact PID socket inode, IPv4 loopback address and port', { skip: process.platform !== 'linux' }, async () => {
  for (const host of ['127.0.0.1', '0.0.0.0']) {
    const server = createServer();
    await new Promise(resolve => server.listen(0, host, resolve));
    try {
      assert.equal(await observeLinuxListener({ pid: process.pid, port: server.address().port }), host === '127.0.0.1');
      assert.equal(await observeLinuxListener({ pid: process.pid, port: server.address().port === 65535 ? 65534 : server.address().port + 1 }), false);
      await assert.rejects(verifyLinuxNativeApp({ pid: process.pid, port: server.address().port, data: '/tmp/not-a-native-dataset', uid: process.getuid() }), /real native App executable/);
    } finally { await new Promise(resolve => server.close(resolve)); }
  }
});

test('Linux CLI rejects historical continuation and incomplete lifecycle options before reading credentials', { skip: process.platform !== 'linux' }, async () => {
  const execute = promisify(execFile);
  const script = fileURLToPath(new URL('./global-os-linux-native.mjs', import.meta.url));
  for (const [args, diagnostic] of [
    [['--continue-session', 'historical-session'], /Unsupported Linux option --continue-session/],
    [['--app-pid', '12345'], /--app-pid requires the retained-service scenario/],
    [['--administrator', '--administrator'], /Duplicate option --administrator/],
    [['--scenario', 'desktop'], /Desktop acceptance requires its private --x11-session-root/],
    [['--x11-session-root', '/tmp/not-a-private-session'], /--x11-session-root requires --scenario desktop/],
  ]) await assert.rejects(execute(process.execPath, [script, ...args]), error => error.code === 1 && diagnostic.test(error.stderr) && !error.stderr.includes('ENOENT'));
});

test('Linux fixture allocation stays separate from canonical effects and detects real cleanup needs', { skip: process.platform !== 'linux' }, async () => {
  const fixture = `/tmp/nomifun-global-os-product-${randomUUID()}`;
  const scenario = createLinuxSystemScenario({ fixture, workspace: `${fixture}/workspace`, uid: process.getuid(), loopbackPort: 42819 });
  assert.equal(await scenario.cleanupNeeded(), false);
  assert.deepEqual(scenario.phases.map(phase => phase.role), ['system-install-config', 'system-service', 'system-network', 'system-finalize']);
  assert(scenario.steps.some(step => step.name === 'start_process' && step.args.command === '/usr/bin/systemd-run'));
  assert(scenario.steps.some(step => step.name === 'start_process'));
  assert(scenario.steps.some(step => step.name === 'cancel_process'));
  assert(scenario.steps.some(step => step.name === 'poll_process' && step.allowCancelledObservation));
  assert(scenario.pendingAcceptance.every(item => item.status === 'not_run'));
  await mkdir(fixture, { mode: 0o700 }); await mkdir(scenario.resources.root, { mode: 0o700 });
  try { assert.equal(await scenario.cleanupNeeded(), true); }
  finally { await rmdir(scenario.resources.root); await rmdir(fixture); }
  assert.throws(() => createLinuxSystemScenario({ fixture: '/home/muri', workspace: '/home/muri/workspace', uid: process.getuid(), loopbackPort: 42819 }), /temporary fixture/);
});
