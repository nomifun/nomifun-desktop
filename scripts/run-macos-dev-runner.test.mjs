import { describe, expect, test } from 'bun:test';
import { mkdtemp, writeFile, mkdir, readFile, rm, chmod } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { EventEmitter, once } from 'node:events';
import { connect } from 'node:net';
import { createMacosDevLifetime } from './run-dev.mjs';
import { createMacosDevSupervisor, acquireMacosDevGeneration, validateDevelopmentLaunch } from './lib/macos-dev-supervisor.mjs';
import { parseMacosCargoRunnerArguments, developmentInfoPlist, ensureMacosDevelopmentBundle, runCargoArtifact, macosBrowserCompilerEnvironment, nativeBrowserLaunchEnvironment } from './run-macos-dev-runner.mjs';
import { fastBuildInvocation, findBuiltMacosApp, completeFastMacosApp } from './run-fast-build.mjs';
import { MACOS_BROWSER_RUNTIME } from './lib/macos-browser-bundle.mjs';
import { macosDevelopmentSigningIdentity, macosDevelopmentSigningNotice } from './lib/macos-dev-signing.mjs';

describe('macOS cargo runner', () => {
  test('only an explicit installed identity replaces ad-hoc signing', () => {
    expect(macosDevelopmentSigningIdentity({})).toBe('-');
    expect(macosDevelopmentSigningNotice({})).toContain('重建后可能需 macOS 钥匙串授权');
    const environment = { NOMIFUN_MACOS_DEV_SIGN_IDENTITY: ' Apple Development: Fixture (TEAMFIXTURE) ' };
    expect(macosDevelopmentSigningIdentity(environment)).toBe('Apple Development: Fixture (TEAMFIXTURE)');
    expect(macosDevelopmentSigningNotice(environment)).toBeNull();
    for (const identity of ['', ' ', '--options', 'fixture\nother', 'fixture\0other']) {
      expect(() => macosDevelopmentSigningIdentity({ NOMIFUN_MACOS_DEV_SIGN_IDENTITY: identity })).toThrow('installed codesign identity');
    }
  });

  test('retains features, target, profile and application arguments across bundle launch', () => {
    const plan = parseMacosCargoRunnerArguments(['run', '--no-default-features', '--features', 'fixture', '--target', 'aarch64-apple-darwin', '--profile', 'release', '--color', 'always', '--', '--fixture', '中文'], 'arm64');
    expect(plan.build).toEqual(['build', '--no-default-features', '--features', 'fixture', '--target', 'aarch64-apple-darwin', '--profile', 'release', '--color', 'always', '--bin', 'nomifun-desktop', '--message-format=json-render-diagnostics']);
    expect(plan.helper).toContain('aarch64-apple-darwin');
    expect(plan.helper).toContain('release');
    expect(plan.application).toEqual(['--fixture', '中文']);
    expect(() => parseMacosCargoRunnerArguments(['run'], 'x64')).toThrow('arm64');
    expect(() => parseMacosCargoRunnerArguments(['run', '--target', 'x86_64-apple-darwin'], 'arm64')).toThrow('aarch64');
    expect(() => parseMacosCargoRunnerArguments(['run', '--bin', 'unrelated'], 'arm64')).toThrow('desktop');
  });

  test('fast Mac builds produce an app while Windows and Linux keep no-bundle', () => {
    expect(fastBuildInvocation([], 'darwin', 'arm64').args).toContain('app');
    expect(fastBuildInvocation([], 'darwin', 'arm64').args).not.toContain('--no-bundle');
    for (const platform of ['win32', 'linux']) expect(fastBuildInvocation([], platform, 'x64').args).toContain('--no-bundle');
    expect(() => fastBuildInvocation(['--no-bundle'], 'darwin', 'arm64')).toThrow('complete');
    expect(() => fastBuildInvocation(['--target=x86_64-apple-darwin'], 'darwin', 'arm64')).toThrow('arm64');
    expect(() => fastBuildInvocation(['--', '--profile=release'], 'darwin', 'arm64')).toThrow('debug profile');
  });

  test('uses Cargo executable reports and propagates compilation failure without launching', async () => {
    const root = await mkdtemp(join(tmpdir(), 'nomifun-cargo-report-'));
    try {
      const cargo = join(root, 'cargo');
      await writeFile(cargo, '#!/usr/bin/env bun\n' +
        'if (process.argv.includes("--fail")) { process.stderr.write("could not compile fixture\\n"); process.exit(101); }\n' +
        'process.stdout.write(JSON.stringify({reason:"compiler-artifact",target:{name:"nomifun-desktop"},executable:"target/debug/nomifun-desktop"}) + "\\n");\n');
      await chmod(cargo, 0o755);
      const options = { cwd: root, environment: { PATH: `${root}:${dirname(process.execPath)}:/usr/bin:/bin` } };
      expect(await runCargoArtifact(['build'], 'nomifun-desktop', options)).toBe(join(root, 'target/debug/nomifun-desktop'));
      try {
        await runCargoArtifact(['build', '--fail'], 'nomifun-desktop', options);
        throw new Error('expected a compilation failure');
      } catch (error) { expect(error.exitCode).toBe(101); }
    } finally { await rm(root, { recursive: true, force: true }); }
  });

  test('reuses only a validated pinned compile cache and never passes CEF_PATH to the app', async () => {
    const environment = { CEF_PATH: '/ambient-unverified', FLATPAK: '1', NOMIFUN_DATA_DIR: '/isolated', TAURI_CONFIG: '{"identifier":"fixture"}' };
    const root = await mkdtemp(join(tmpdir(), 'nomifun-pinned-cef-cache-'));
    try {
      const runtime = join(root, 'build.noindex/debug/build/cef-dll-sys-fixture/out/cef_macos_aarch64');
      await mkdir(join(runtime, 'Chromium Embedded Framework.framework'), { recursive: true });
      await writeFile(join(runtime, 'Chromium Embedded Framework.framework/Chromium Embedded Framework'), 'fixture');
      await writeFile(join(runtime, 'archive.json'), JSON.stringify({ name: MACOS_BROWSER_RUNTIME.archive, sha1: MACOS_BROWSER_RUNTIME.archive_sha1 }));
      const compiler = await macosBrowserCompilerEnvironment({ root, target: 'aarch64-apple-darwin', profile: 'release', environment });
      expect(compiler.environment.CEF_PATH).toBe(runtime);
      expect(compiler.environment.FLATPAK).toBeUndefined();
      expect(nativeBrowserLaunchEnvironment(compiler.environment)).toEqual({ NOMIFUN_DATA_DIR: '/isolated', TAURI_CONFIG: '{"identifier":"fixture"}' });
      await rm(join(root, 'build.noindex'), { recursive: true, force: true });
      const fresh = await macosBrowserCompilerEnvironment({ root, environment });
      expect(fresh.runtimePath).toBeNull();
      expect(fresh.environment.CEF_PATH).toBeUndefined();
    } finally { await rm(root, { recursive: true, force: true }); }
  });
});

async function fixtureRoot() {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-dev-bundle-'));
  await mkdir(join(root, 'apps/desktop'), { recursive: true });
  await mkdir(join(root, 'scripts/lib'), { recursive: true });
  await writeFile(join(root, 'apps/desktop/tauri.conf.json'), JSON.stringify({ productName: 'NomiFun', identifier: 'com.nomifun.desktop' }));
  await writeFile(join(root, 'apps/desktop/tauri.dev.conf.json'), JSON.stringify({ productName: 'NomiFun Dev', identifier: 'com.nomifun.desktop.dev' }));
  await writeFile(join(root, 'apps/desktop/Info.plist'), '<plist><dict><key>NSCameraUsageDescription</key><string>existing camera reason</string></dict></plist>');
  await writeFile(join(root, 'scripts/lib/macos-browser-bundle.mjs'), '// fixture staging version');
  await writeFile(join(root, 'package.json'), JSON.stringify({ version: '0.7.6' }));
  await writeFile(join(root, 'host'), 'host-one');
  await writeFile(join(root, 'helper'), 'helper-one');
  return root;
}

describe('complete development app cache', () => {
  test('an unavailable explicit signing identity fails without publishing or falling back', async () => {
    const root = await fixtureRoot();
    const attempted = [];
    try {
      const request = { root, hostPath: join(root, 'host'), helperPath: join(root, 'helper'), runtimePath: '/pinned', environment: { NOMIFUN_MACOS_DEV_SIGN_IDENTITY: 'Unavailable identity' } };
      await expect(ensureMacosDevelopmentBundle(request, {
        stage: async options => { attempted.push(options); throw new Error('installed identity missing'); },
        inspect: async () => ({ status: 'pass' }), run: async () => {},
      })).rejects.toThrow('installed identity missing');
      expect(attempted.map(options => options.identity)).toEqual(['Unavailable identity']);
      await expect(readFile(join(dirname(dirname(attempted[0].appPath)), 'complete.json'))).rejects.toThrow('ENOENT');
    } finally { await rm(root, { recursive: true, force: true }); }
  });

  test('stable signing identity reaches the full stage and incremental seal and isolates caches', async () => {
    const root = await fixtureRoot();
    const staged = [];
    const commands = [];
    const hooks = {
      stage: async request => { staged.push(request); },
      inspect: async () => ({ status: 'pass' }),
      run: async (program, args) => { commands.push([program, ...args]); },
    };
    const identity = 'Apple Development: Fixture (TEAMFIXTURE)';
    const request = { root, hostPath: join(root, 'host'), helperPath: join(root, 'helper'), runtimePath: '/pinned', environment: {} };
    try {
      const adhoc = await ensureMacosDevelopmentBundle(request, hooks);
      const signedRequest = { ...request, environment: { NOMIFUN_MACOS_DEV_SIGN_IDENTITY: identity } };
      const signed = await ensureMacosDevelopmentBundle(signedRequest, hooks);
      expect(signed).not.toBe(adhoc);
      expect(staged.map(request => request.identity)).toEqual(['-', identity]);
      expect(await ensureMacosDevelopmentBundle(signedRequest, hooks)).toBe(signed);
      await writeFile(request.hostPath, 'host-two');
      const rebuilt = await ensureMacosDevelopmentBundle(signedRequest, hooks);
      expect(rebuilt).not.toBe(signed);
      expect(staged.length).toBe(2);
      const seal = commands.find(command => command[0] === 'codesign' && command[1] === '--force');
      expect(seal).toEqual(['codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', identity, dirname(dirname(dirname(rebuilt)))]);
      const other = await ensureMacosDevelopmentBundle({ ...request, environment: { NOMIFUN_MACOS_DEV_SIGN_IDENTITY: 'Apple Development: Other (TEAMOTHER)' } }, hooks);
      expect(other).not.toBe(rebuilt);
      expect(staged.at(-1).identity).toBe('Apple Development: Other (TEAMOTHER)');
      expect(commands.some(command => command[0] === 'security')).toBe(false);
    } finally { await rm(root, { recursive: true, force: true }); }
  });

  test('matches embedded test identity and keeps datasets apart', async () => {
    const root = await fixtureRoot();
    try {
      const environment = { TAURI_CONFIG: JSON.stringify({ identifier: 'com.nomifun.fixture', productName: 'Fixture & Test' }), NOMIFUN_DATA_DIR: '/fixture-data' };
      const plist = await developmentInfoPlist(root, environment);
      expect(plist).toContain('com.nomifun.fixture');
      expect(plist).toContain('Fixture &amp; Test');
      expect(plist).toContain('existing camera reason');
      const hooks = { stage: async () => {}, inspect: async () => ({ status: 'pass' }), run: async () => {} };
      const first = await ensureMacosDevelopmentBundle({ root, hostPath: join(root, 'host'), helperPath: join(root, 'helper'), runtimePath: '/pinned', environment }, hooks);
      const second = await ensureMacosDevelopmentBundle({ root, hostPath: join(root, 'host'), helperPath: join(root, 'helper'), runtimePath: '/pinned', environment: { ...environment, NOMIFUN_DATA_DIR: '/second-data' } }, hooks);
      expect(first).not.toBe(second);
      expect(first).toEndWith('.app/Contents/MacOS/nomifun-desktop');
      expect(await readFile(join(dirname(dirname(first)), 'Info.plist'), 'utf8')).toContain('com.nomifun.fixture');
    } finally { await rm(root, { recursive: true, force: true }); }
  });

  test('incremental host rebuild reuses CEF; changed helper or broken seal restages it', async () => {
    const root = await fixtureRoot();
    let stages = 0;
    let broken = false;
    const commands = [];
    const hooks = {
      stage: async () => { stages++; broken = false; },
      inspect: async () => ({ status: 'pass' }),
      run: async (program, args) => {
        commands.push([program, ...args]);
        if (broken && program === 'codesign' && args[0] === '--verify') throw new Error('broken app seal');
      },
    };
    const request = { root, hostPath: join(root, 'host'), helperPath: join(root, 'helper'), runtimePath: '/pinned', environment: {} };
    try {
      const program = await ensureMacosDevelopmentBundle(request, hooks);
      await ensureMacosDevelopmentBundle(request, hooks);
      expect(stages).toBe(1);
      await writeFile(request.hostPath, 'host-two');
      const rebuilt = await ensureMacosDevelopmentBundle(request, hooks);
      expect(stages).toBe(1);
      expect(rebuilt).not.toBe(program);
      expect(await readFile(program, 'utf8')).toBe('host-one');
      expect(await readFile(rebuilt, 'utf8')).toBe('host-two');
      expect(commands.some(command => command.includes('--force'))).toBe(true);
      await writeFile(request.helperPath, 'helper-two');
      await ensureMacosDevelopmentBundle(request, hooks);
      expect(stages).toBe(2);
      broken = true;
      await ensureMacosDevelopmentBundle(request, hooks);
      expect(stages).toBe(3);
    } finally { await rm(root, { recursive: true, force: true }); }
  });

  test('fast build chooses the exact current host, not an old product bundle', async () => {
    const root = await mkdtemp(join(tmpdir(), 'nomifun-fast-build-'));
    try {
      await writeFile(join(root, 'nomifun-desktop'), 'current');
      for (const [name, binary] of [['Old.app', 'old'], ['Current.app', 'current']]) {
        const contents = join(root, 'bundle/macos', name, 'Contents');
        await mkdir(join(contents, 'MacOS'), { recursive: true });
        await writeFile(join(contents, 'Info.plist'), 'plist');
        await writeFile(join(contents, 'MacOS/nomifun-desktop'), binary);
      }
      expect(await findBuiltMacosApp(root)).toBe(join(root, 'bundle/macos/Current.app'));
    } finally { await rm(root, { recursive: true, force: true }); }
  });

  test('fast build keeps its verified app when a watcher replaces raw outputs during the helper build', async () => {
    const root = await mkdtemp(join(tmpdir(), 'nomifun-fast-build-race-'));
    const original = join(root, 'bundle/macos/NomiFun Dev.app');
    try {
      await mkdir(join(original, 'Contents/MacOS'), { recursive: true });
      await writeFile(join(root, 'nomifun-desktop'), 'this-fast-build');
      await writeFile(join(original, 'Contents/MacOS/nomifun-desktop'), 'this-fast-build');
      await writeFile(join(original, 'Contents/Info.plist'), 'this-fast-build-identity');
      const appPath = await completeFastMacosApp({
        outDir: root,
        environment: { NOMIFUN_MACOS_DEV_SIGN_IDENTITY: 'Apple Development: Fast Fixture (TEAMFIXTURE)' },
        buildHelper: async () => {
          // Simulate Cargo watch winning the lock and a later bundler touching
          // the same output while the helper waits. Neither may alter our app.
          await writeFile(join(root, 'nomifun-desktop'), 'watcher-build');
          await writeFile(join(original, 'Contents/MacOS/nomifun-desktop'), 'later-bundler-build');
          await writeFile(join(original, 'Contents/Info.plist'), 'later-bundler-identity');
          return { helperPath: '/owned/helper', runtimePath: '/pinned/runtime' };
        },
        stage: async ({ appPath, identity }) => {
          expect(identity).toBe('Apple Development: Fast Fixture (TEAMFIXTURE)');
          expect(appPath).not.toBe(original);
          expect(await readFile(join(appPath, 'Contents/MacOS/nomifun-desktop'), 'utf8')).toBe('this-fast-build');
          expect(await readFile(join(appPath, 'Contents/Info.plist'), 'utf8')).toBe('this-fast-build-identity');
        },
      });
      expect(appPath).toContain('/bundle/macos-fast/run-');
      expect(await readFile(join(dirname(appPath), 'build-identity.json'), 'utf8')).toContain('hostSha256');
    } finally { await rm(root, { recursive: true, force: true }); }
  });

  test('fast build rejects ambiguous matching app identities instead of choosing the newest', async () => {
    const root = await mkdtemp(join(tmpdir(), 'nomifun-fast-build-ambiguous-'));
    try {
      await writeFile(join(root, 'nomifun-desktop'), 'same-build');
      for (const name of ['First.app', 'Second.app']) {
        const contents = join(root, 'bundle/macos', name, 'Contents');
        await mkdir(join(contents, 'MacOS'), { recursive: true });
        await writeFile(join(contents, 'Info.plist'), name);
        await writeFile(join(contents, 'MacOS/nomifun-desktop'), 'same-build');
      }
      await expect(findBuiltMacosApp(root)).rejects.toThrow('ambiguous');
    } finally { await rm(root, { recursive: true, force: true }); }
  });
});

describe('macOS watch generation ownership', () => {
  test('runner death requests native cleanup and blocks the next launch until child exit', async () => {
    const children = [];
    const desktops = [];
    const supervisor = await createMacosDevSupervisor({
      root: '/workspace', createLifetime: createMacosDevLifetime,
      validateLaunch: request => request,
      spawnProgram(_program, _args, options) {
        expect(options.detached).toBe(true);
        const child = new EventEmitter();
        children.push(child);
        const desktop = connect({ path: options.env.NOMIFUN_DEV_LIFETIME_SOCKET, allowHalfOpen: true });
        desktops.push(desktop);
        desktop.on('error', error => child.emit('error', error));
        desktop.on('data', bytes => child.emit('stop-request', bytes.toString()));
        desktop.once('connect', () => child.emit('connected'));
        return child;
      },
    });
    const first = await acquireMacosDevGeneration(supervisor.socketPath);
    let second;
    try {
      const exited = first.launch({ program: '/owned.app/Contents/MacOS/nomifun-desktop', args: [], environment: {}, cwd: '/workspace' });
      exited.catch(() => {});
      while (!children[0]) await new Promise(accept => setImmediate(accept));
      await once(children[0], 'connected');
      const requested = once(children[0], 'stop-request');
      first.close(); // Tauri's SIGKILL closes this control lease.
      let admitted = false;
      const next = acquireMacosDevGeneration(supervisor.socketPath).then(value => { admitted = true; return value; });
      expect((await requested)[0]).toBe('q');
      expect(admitted).toBe(false);
      desktops[0].destroy();
      await new Promise(accept => setImmediate(accept));
      expect(admitted).toBe(false); // socket EOF alone is not child exit proof.
      children[0].emit('exit', 0, null);
      second = await next;
      expect(admitted).toBe(true);
    } finally {
      first.close(); second?.close();
      for (const desktop of desktops) desktop.destroy();
      await supervisor.stop();
    }
  });

  test('rejects arbitrary launch paths and keeps each control socket private', async () => {
    const request = { program: '/workspace/target/macos-dev-browser/key/NomiFun Dev.app/Contents/MacOS/nomifun-desktop', args: ['中文'], environment: { NOMIFUN_DATA_DIR: '/isolated' }, cwd: '/workspace/apps/desktop' };
    expect(validateDevelopmentLaunch(request, '/workspace').environment.NOMIFUN_DATA_DIR).toBe('/isolated');
    expect(() => validateDevelopmentLaunch({ ...request, program: '/Applications/Other.app/Contents/MacOS/nomifun-desktop' }, '/workspace')).toThrow('owned');
    expect(() => validateDevelopmentLaunch({ ...request, cwd: '/outside' }, '/workspace')).toThrow('workspace');
  });

  test('unproven native cleanup closes admission and never force kills the app', async () => {
    let child;
    let forced = false;
    const supervisor = await createMacosDevSupervisor({
      root: '/workspace', validateLaunch: request => request,
      createLifetime: async () => ({ socketPath: '/fake-native-lifetime', stop: async () => { throw new Error('native cleanup unconfirmed'); } }),
      spawnProgram() {
        child = new EventEmitter();
        child.kill = () => { forced = true; };
        return child;
      },
    });
    const first = await acquireMacosDevGeneration(supervisor.socketPath);
    try {
      first.launch({ program: '/owned.app/Contents/MacOS/nomifun-desktop', args: [], environment: {}, cwd: '/workspace' }).catch(() => {});
      while (!child) await new Promise(accept => setImmediate(accept));
      first.close();
      await expect(acquireMacosDevGeneration(supervisor.socketPath)).rejects.toThrow('cleanup unconfirmed');
      await expect(supervisor.stop()).rejects.toThrow('cleanup unconfirmed');
      expect(forced).toBe(false);
      child.emit('exit', 1, null);
    } finally {
      first.close();
      await rm(dirname(supervisor.socketPath), { recursive: true, force: true });
    }
  });

  test('an abnormal app exit cannot start another CEF generation', async () => {
    let child;
    const supervisor = await createMacosDevSupervisor({
      root: '/workspace', validateLaunch: request => request,
      createLifetime: async () => ({ socketPath: '/fake-native-lifetime', stop: async () => {} }),
      spawnProgram() { child = new EventEmitter(); return child; },
    });
    const first = await acquireMacosDevGeneration(supervisor.socketPath);
    try {
      const exited = first.launch({ program: '/owned.app/Contents/MacOS/nomifun-desktop', args: [], environment: {}, cwd: '/workspace' });
      while (!child) await new Promise(accept => setImmediate(accept));
      child.emit('exit', null, 'SIGKILL');
      expect((await exited).signal).toBe('SIGKILL');
      await expect(acquireMacosDevGeneration(supervisor.socketPath)).rejects.toThrow('cleanup is unconfirmed');
      await expect(supervisor.stop()).rejects.toThrow('cleanup is unconfirmed');
    } finally {
      first.close();
      await rm(dirname(supervisor.socketPath), { recursive: true, force: true });
    }
  });
});
