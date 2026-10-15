import { describe, expect, test } from 'bun:test';
import { mkdtemp, mkdir, readFile, realpath, rm, stat, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { launchMacosApplication, MacosApplicationLaunchCancelled, validateApplicationLaunch } from './macos-application-launch.mjs';

async function fixtureLaunch(kind = 'verified', symbolic = false, {
  terminalOverrides = {}, applicationScope = 'desktop_development', controllerCode = 0, controllerSignal = null,
  beforeClose = async () => {}, requestOverride = {}, isLaunchAllowed, onSubmit = () => {},
} = {}) {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-launch-protocol-'));
  const physicalCwd = join(root, 'physical');
  await mkdir(physicalCwd);
  const cwd = symbolic ? join(root, 'alias') : physicalCwd;
  if (symbolic) await symlink(physicalCwd, cwd);
  const request = { program: join(root, 'Fixture.app/Contents/MacOS/nomifun-desktop'), controllerProgram: join(root, 'macos_application_controller'),
    args: applicationScope === 'browser_media_audit' ? ['--media-permissions']
      : applicationScope === 'appkit_main_loop_audit' ? ['--report', join(root, 'report.json')] : [],
    environment: { NOMIFUN_DATA_DIR: join(root, 'data'), FIXTURE_PRIVATE_ENV: 'private-value' }, cwd,
    applicationScope, ...requestOverride };
  let descriptor;
  let argv;
  let descriptorMode;
  let directoryMode;
  let spawnedCwd;
  const launch = await launchMacosApplication(request, { isLaunchAllowed, spawnController(program, args, options) {
    onSubmit();
    argv = args;
    spawnedCwd = options.cwd;
    const child = new EventEmitter();
    child.stdout = new PassThrough();
    setImmediate(async () => {
      descriptor = JSON.parse(await readFile(args[0], 'utf8'));
      descriptorMode = (await stat(args[0])).mode & 0o777;
      directoryMode = (await stat(dirname(args[0]))).mode & 0o777;
      const ready = { event: 'ready', pid: 4242, cwd: descriptor.cwd, data_dir: descriptor.environment.NOMIFUN_DATA_DIR };
      if (kind === 'null') child.stdout.write('null\n');
      if (kind === 'oversized') {
        child.stdout.write('x'.repeat(1024 * 1024 + 1));
        for (let count = 0; count < 4; count++) child.stdout.write('y'.repeat(256 * 1024));
      }
      if (kind !== 'helper-only') child.stdout.write(`${JSON.stringify(ready)}\n`);
      const terminal = { event: 'exit', pid: 4242, code: 0, signal: null,
        cleanup_verified: kind === 'verified' || kind === 'duplicate', restart_requested: false, ...terminalOverrides };
      if (kind !== 'helper-only') child.stdout.write(`${JSON.stringify(terminal)}\n`);
      if (kind === 'duplicate') child.stdout.write(`${JSON.stringify({ ...terminal, restart_requested: true })}\n`);
      await beforeClose();
      child.stdout.end();
      child.emit('close', controllerCode, controllerSignal);
    });
    return child;
  } });
  return { root, launch, request, inspect: () => ({ descriptor, argv, descriptorMode, directoryMode, spawnedCwd }) };
}

describe.skipIf(process.platform === 'win32')('LaunchServices controller protocol routing', () => {
  test('keeps environment/nonce in a private descriptor and requires App evidence', async () => {
    const fixture = await fixtureLaunch();
    try {
      const result = await fixture.launch.exit;
      const observed = fixture.inspect();
      expect(result.cleanupVerified).toBe(true);
      expect(result.restartRequested).toBe(false);
      expect(observed.descriptorMode).toBe(0o600);
      expect(observed.directoryMode).toBe(0o700);
      expect(observed.argv).toHaveLength(1);
      expect(observed.argv.join('')).not.toContain(observed.descriptor.nonce);
      expect(observed.argv.join('')).not.toContain('private-value');
      expect(observed.descriptor.environment.FIXTURE_PRIVATE_ENV).toBe('private-value');
    } finally { await rm(fixture.root, { recursive: true, force: true }); await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true }); }
  });
  test.each(['helper-only', 'unverified'])('%s exit zero cannot become an App cleanup PASS', async kind => {
    const fixture = await fixtureLaunch(kind);
    try { await expect(fixture.launch.exit).rejects.toThrow('authenticated cleanup'); }
    finally { await rm(fixture.root, { recursive: true, force: true }); await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true }); }
  });
  test('preserves a legitimate symlink cwd as its real directory on both sides', async () => {
    const fixture = await fixtureLaunch('verified', true);
    try {
      await fixture.launch.exit;
      const expected = await realpath(fixture.request.cwd);
      expect(fixture.inspect().descriptor.cwd).toBe(expected);
      expect(fixture.inspect().spawnedCwd).toBe(expected);
    } finally { await rm(fixture.root, { recursive: true, force: true }); await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true }); }
  });
  test('JSON null cannot throw outside the owned exit promise or become successful cleanup', async () => {
    const fixture = await fixtureLaunch('null');
    try { await expect(fixture.launch.exit).rejects.toThrow('must be an object'); }
    finally { await rm(fixture.root, { recursive: true, force: true }); await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true }); }
  });
  test('oversized output discards further bytes while retaining the controller until its actual close', async () => {
    const fixture = await fixtureLaunch('oversized');
    try {
      await expect(fixture.launch.exit).rejects.toThrow('exceeded its bound');
      expect(fixture.launch.diagnostics().bufferedBytes).toBe(0);
      expect(fixture.launch.diagnostics().discardedBytes).toBeGreaterThanOrEqual(1024 * 1024);
    } finally { await rm(fixture.root, { recursive: true, force: true }); await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true }); }
  });

  test('authenticated restart is returned only after the controller closes normally', async () => {
    let release;
    let framed;
    const closeGate = new Promise(accept => { release = accept; });
    const frameGate = new Promise(accept => { framed = accept; });
    const fixture = await fixtureLaunch('verified', false, {
      terminalOverrides: { restart_requested: true },
      beforeClose: () => { framed(); return closeGate; },
    });
    let settled = false;
    fixture.launch.exit.finally(() => { settled = true; }).catch(() => {});
    try {
      await frameGate;
      expect(settled).toBe(false);
      release();
      expect(await fixture.launch.exit).toMatchObject({ code: 0, signal: null, cleanupVerified: true, restartRequested: true });
    } finally {
      release();
      await Promise.allSettled([fixture.launch.exit]);
      await rm(fixture.root, { recursive: true, force: true });
      await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true });
    }
  });

  test.each([
    [undefined], [null], [0], [1], ['true'], [{}], [[]],
  ])('restart flag %j cannot be inferred or coerced', async restart_requested => {
    const fixture = await fixtureLaunch('verified', false, { terminalOverrides: { restart_requested } });
    try { await expect(fixture.launch.exit).rejects.toThrow('out of sequence'); }
    finally { await rm(fixture.root, { recursive: true, force: true }); await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true }); }
  });

  test.each([
    { terminalOverrides: { code: 1 } },
    { terminalOverrides: { signal: 15 } },
    { terminalOverrides: { cleanup_verified: false } },
    { controllerCode: 1 },
    { controllerSignal: 'SIGKILL' },
  ])('restart true cannot upgrade missing normal cleanup/exit evidence %j', async options => {
    const fixture = await fixtureLaunch('verified', false, {
      ...options, terminalOverrides: { restart_requested: true, ...options.terminalOverrides },
    });
    try { await expect(fixture.launch.exit).rejects.toThrow('authenticated cleanup'); }
    finally { await rm(fixture.root, { recursive: true, force: true }); await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true }); }
  });

  test.each([false, true])('media audit restart %s is explicitly scoped', async restart_requested => {
    const fixture = await fixtureLaunch('verified', false, {
      applicationScope: 'browser_media_audit', terminalOverrides: { restart_requested },
    });
    try {
      if (restart_requested) await expect(fixture.launch.exit).rejects.toThrow('media audit cannot request');
      else expect((await fixture.launch.exit).restartRequested).toBe(false);
    } finally { await rm(fixture.root, { recursive: true, force: true }); await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true }); }
  });

  test.each(['browser_compatibility_audit', 'appkit_main_loop_audit'])('%s keeps admission and final cleanup in its own audit scope', async applicationScope => {
    for (const restart_requested of [false, true]) {
      const fixture = await fixtureLaunch('verified', false, { applicationScope, terminalOverrides: { restart_requested } });
      try {
        if (restart_requested) await expect(fixture.launch.exit).rejects.toThrow('all audit scopes reject restart');
        else {
          expect((await fixture.launch.exit).cleanupVerified).toBe(true);
          expect(fixture.inspect().descriptor.application_scope).toBe(applicationScope);
        }
      } finally { await rm(fixture.root, { recursive: true, force: true }); await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true }); }
    }
  });

  test('fixed audit entries reject cross-scope permission or missing report arguments before controller submission', () => {
    const base = { program: '/tmp/Audit.app/Contents/MacOS/nomifun-desktop', controllerProgram: '/tmp/macos_application_controller', cwd: '/tmp',
      environment: { NOMIFUN_DATA_DIR: '/tmp/independent-audit-data' } };
    expect(() => validateApplicationLaunch({ ...base, applicationScope: 'browser_media_audit', args: [] })).toThrow('fixed permission');
    expect(() => validateApplicationLaunch({ ...base, applicationScope: 'browser_compatibility_audit', args: ['--media-permissions'] })).toThrow('cannot enter the media');
    expect(() => validateApplicationLaunch({ ...base, applicationScope: 'browser_compatibility_audit', args: ['--media-user-ready'] })).toThrow('cannot enter the media');
    expect(() => validateApplicationLaunch({ ...base, applicationScope: 'appkit_main_loop_audit', args: ['--report', 'relative.json'] })).toThrow('absolute report');
    expect(validateApplicationLaunch({ ...base, applicationScope: 'browser_compatibility_audit', args: ['--identity-ab'] }).applicationScope).toBe('browser_compatibility_audit');
  });

  test('a contradictory second terminal cannot upgrade an earlier receipt', async () => {
    const fixture = await fixtureLaunch('duplicate');
    try { await expect(fixture.launch.exit).rejects.toThrow('out of sequence'); }
    finally { await rm(fixture.root, { recursive: true, force: true }); await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true }); }
  });

  test('relaunch of the same frozen App produces distinct private descriptors and nonces', async () => {
    const first = await fixtureLaunch('verified', false, { terminalOverrides: { restart_requested: true } });
    let second;
    try {
      await first.launch.exit;
      second = await fixtureLaunch('verified', false, { requestOverride: first.request });
      await second.launch.exit;
      const old = first.inspect();
      const fresh = second.inspect();
      expect(fresh.argv[0]).not.toBe(old.argv[0]);
      expect(fresh.descriptor.nonce).not.toBe(old.descriptor.nonce);
      expect(fresh.descriptor.nonce).toMatch(/^[a-f0-9]{64}$/);
      expect(fresh.descriptor).toMatchObject({
        app_path: old.descriptor.app_path, program: old.descriptor.program, cwd: old.descriptor.cwd,
        args: old.descriptor.args, environment: old.descriptor.environment,
      });
      expect([old.directoryMode, fresh.directoryMode]).toEqual([0o700, 0o700]);
      expect([old.descriptorMode, fresh.descriptorMode]).toEqual([0o600, 0o600]);
    } finally {
      for (const fixture of [second, first].filter(Boolean)) {
        await rm(fixture.root, { recursive: true, force: true });
        await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true });
      }
    }
  });

  test.each(['entry', 'pre-spawn'])('observed cancellation at %s never submits a controller', async stage => {
    const root = await mkdtemp(join(tmpdir(), 'nomifun-launch-cancel-'));
    let allowed = stage !== 'entry';
    let submissions = 0;
    let cancellation;
    try {
      const request = { program: join(root, 'Fixture.app/Contents/MacOS/nomifun-desktop'),
        controllerProgram: join(root, 'macos_application_controller'), args: [], cwd: root,
        environment: { NOMIFUN_DATA_DIR: join(root, 'data') } };
      try {
        await launchMacosApplication(request, {
          isLaunchAllowed() {
            const result = allowed;
            // Models a stop observed while asynchronous filesystem preparation
            // is pending; the real final submission guard must read it again.
            if (stage === 'pre-spawn') queueMicrotask(() => { allowed = false; });
            return result;
          },
          spawnController() { submissions++; throw new Error('cancelled App must not be submitted'); },
        });
      } catch (error) { cancellation = error; }
      expect(cancellation).toBeInstanceOf(MacosApplicationLaunchCancelled);
      expect(submissions).toBe(0);
      if (stage === 'pre-spawn') {
        expect(await readFile(join(cancellation.evidenceDirectory, 'launch.json')).catch(error => error.code)).toBe('ENOENT');
        expect((await stat(cancellation.evidenceDirectory)).mode & 0o777).toBe(0o700);
      } else expect(cancellation.evidenceDirectory).toBeUndefined();
    } finally {
      if (cancellation?.evidenceDirectory) await rm(cancellation.evidenceDirectory, { recursive: true, force: true });
      await rm(root, { recursive: true, force: true });
    }
  });

  test('final admission check and controller submission share the same synchronous turn', async () => {
    let allowed = true;
    let checks = 0;
    let allowedAtSubmission;
    const fixture = await fixtureLaunch('verified', false, {
      isLaunchAllowed() {
        checks++;
        // A stop arriving after the final check belongs to the admitted App's
        // ordinary shutdown. An intervening await would create a new gap.
        if (checks === 2) queueMicrotask(() => { allowed = false; });
        return allowed;
      },
      onSubmit() { allowedAtSubmission = allowed; },
    });
    try {
      await fixture.launch.exit;
      expect(allowedAtSubmission).toBe(true);
      expect(allowed).toBe(false);
    } finally { await rm(fixture.root, { recursive: true, force: true }); await rm(fixture.launch.evidenceDirectory, { recursive: true, force: true }); }
  });
});
