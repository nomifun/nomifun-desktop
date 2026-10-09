import { describe, expect, test } from 'bun:test';
import { mkdtemp, writeFile, mkdir, readFile, rm, chmod } from 'node:fs/promises';
import { join, dirname, sep } from 'node:path';
import { tmpdir } from 'node:os';
import { EventEmitter, once } from 'node:events';
import { connect } from 'node:net';
import { createMacosDevLifetime } from './run-dev.mjs';
import { createMacosDevSupervisor, acquireMacosDevGeneration, validateDevelopmentLaunch } from './lib/macos-dev-supervisor.mjs';
import { parseMacosCargoRunnerArguments, developmentInfoPlist, ensureMacosDevelopmentBundle, runCargoArtifact } from './run-macos-dev-runner.mjs';
import { fastBuildInvocation, findBuiltMacosApp, completeFastMacosApp } from './run-fast-build.mjs';
import { prepareMacosBuildEnvironment } from './lib/macos-build-environment.mjs';
import { INTEL_ONNX_RUNTIME } from './lib/macos-onnx-runtime.mjs';
import { macosDevelopmentSigningIdentity, macosDevelopmentSigningNotice } from './lib/macos-dev-signing.mjs';

describe('macOS cargo runner', () => {
  test('only an explicit installed identity replaces ad-hoc signing', () => {
    expect(macosDevelopmentSigningIdentity({})).toBe('-');
    expect(macosDevelopmentSigningNotice({})).toContain('使用 ad-hoc 签名');
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
    expect(plan.profile).toBe('release');
    expect(plan.application).toEqual(['--fixture', '中文']);
    expect(parseMacosCargoRunnerArguments(['run'], 'x64').target).toBeNull();
    expect(parseMacosCargoRunnerArguments(['run', '--target', 'x86_64-apple-darwin'], 'arm64').target).toBe('x86_64-apple-darwin');
    expect(() => parseMacosCargoRunnerArguments(['run', '--target', 'x86_64-unknown-linux-gnu'], 'arm64')).toThrow('macOS builds');
    expect(() => parseMacosCargoRunnerArguments(['run', '--bin', 'unrelated'], 'arm64')).toThrow('desktop');
  });

  test('fast Mac builds produce an app while Windows and Linux keep no-bundle', () => {
    expect(fastBuildInvocation([], 'darwin', 'arm64').args).toContain('app');
    expect(fastBuildInvocation([], 'darwin', 'arm64').args).not.toContain('--no-bundle');
    for (const platform of ['win32', 'linux']) expect(fastBuildInvocation([], platform, 'x64').args).toContain('--no-bundle');
    expect(() => fastBuildInvocation(['--no-bundle'], 'darwin', 'arm64')).toThrow('complete');
    expect(fastBuildInvocation([], 'darwin', 'x64').args).toContain('app');
    expect(fastBuildInvocation(['--target=x86_64-apple-darwin'], 'darwin', 'arm64').target).toBe('x86_64-apple-darwin');
    expect(() => fastBuildInvocation(['--target=x86_64-pc-windows-msvc'], 'darwin', 'arm64')).toThrow('macOS builds');
    expect(() => fastBuildInvocation(['--', '--profile=release'], 'darwin', 'arm64')).toThrow('debug profile');
  });

  test.skipIf(process.platform === 'win32')('uses POSIX Cargo executable reports and propagates compilation failure without launching', async () => {
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

  test('a missing Cargo executable rejects without waiting for stdout EOF', async () => {
    const root = await mkdtemp(join(tmpdir(), 'nomifun-missing-cargo-'));
    try {
      await expect(runCargoArtifact(['build'], 'nomifun-desktop', {
        cwd: root, environment: { PATH: root },
      })).rejects.toHaveProperty('code', 'ENOENT');
    } finally { await rm(root, { recursive: true, force: true }); }
  }, 1500);

  test('Apple Silicon builds use system frameworks without preparing a native runtime', async () => {
    const environment = { NOMIFUN_DATA_DIR: '/isolated', ORT_LIB_PATH: '/previous-intel-build', ORT_PREFER_DYNAMIC_LINK: '1' };
    const compiler = await prepareMacosBuildEnvironment({ architecture: 'arm64', environment }, {
      prepareOnnx: async () => { throw new Error('must not download'); },
    });
    expect(compiler.target).toBe('aarch64-apple-darwin');
    expect(compiler.environment).toEqual({ NOMIFUN_DATA_DIR: '/isolated' });
    expect(compiler.bundleConfig).toBeNull();
    expect(compiler.bundleFiles).toEqual({});
    expect(environment.ORT_LIB_PATH).toBe('/previous-intel-build');
  });

  test('Intel builds retain their ONNX library, licenses and runtime rpath', async () => {
    const compiler = await prepareMacosBuildEnvironment({ architecture: 'x64', environment: { RUSTFLAGS: '-C debuginfo=1' } }, {
      prepareOnnx: async () => '/owned/onnx/lib',
    });
    expect(compiler.target).toBe('x86_64-apple-darwin');
    expect(compiler.environment.ORT_LIB_PATH).toBe('/owned/onnx/lib');
    expect(compiler.environment.ORT_PREFER_DYNAMIC_LINK).toBe('1');
    expect(compiler.environment.RUSTFLAGS).toBe('-C debuginfo=1 -C link-arg=-Wl,-rpath,@executable_path/../Frameworks');
    expect(compiler.bundleConfig).toBe('/owned/onnx/tauri.conf.json');
    expect(compiler.bundleFiles).toEqual({
      [`Frameworks/${INTEL_ONNX_RUNTIME.library}`]: `/owned/onnx/lib/${INTEL_ONNX_RUNTIME.library}`,
      'Resources/onnxruntime-LICENSE': '/owned/onnx/LICENSE',
      'Resources/onnxruntime-ThirdPartyNotices.txt': '/owned/onnx/ThirdPartyNotices.txt',
    });
    const encoded = await prepareMacosBuildEnvironment({ architecture: 'arm64', environment: {
      CARGO_BUILD_TARGET: 'x86_64-apple-darwin', CARGO_ENCODED_RUSTFLAGS: '-C\x1fdebuginfo=1',
    } }, { prepareOnnx: async () => '/owned/onnx/lib' });
    expect(encoded.environment.CARGO_ENCODED_RUSTFLAGS).toBe('-C\x1fdebuginfo=1\x1f-C\x1flink-arg=-Wl,-rpath,@executable_path/../Frameworks');
    const again = await prepareMacosBuildEnvironment({ environment: encoded.environment }, { prepareOnnx: async () => '/owned/onnx/lib' });
    expect(again.environment.CARGO_ENCODED_RUSTFLAGS).toBe(encoded.environment.CARGO_ENCODED_RUSTFLAGS);
  });
});

async function fixtureRoot() {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-dev-bundle-'));
  await mkdir(join(root, 'apps/desktop'), { recursive: true });
  await mkdir(join(root, 'scripts/lib'), { recursive: true });
  await writeFile(join(root, 'apps/desktop/tauri.conf.json'), JSON.stringify({ productName: 'NomiFun', identifier: 'com.nomifun.desktop' }));
  await writeFile(join(root, 'apps/desktop/tauri.dev.conf.json'), JSON.stringify({ productName: 'NomiFun Dev', identifier: 'com.nomifun.desktop.dev' }));
  await writeFile(join(root, 'apps/desktop/Info.plist'), '<plist><dict><key>NSAppTransportSecurity</key><dict><key>NSAllowsArbitraryLoadsInWebContent</key><true/></dict><key>NSCameraUsageDescription</key><string>existing camera reason</string></dict></plist>');
  await writeFile(join(root, 'scripts/lib/macos-app-bundle.mjs'), '// fixture signing version');
  await writeFile(join(root, 'package.json'), JSON.stringify({ version: '0.7.6' }));
  await writeFile(join(root, 'host'), 'host-one');
  await writeFile(join(root, 'native-library'), 'library-one');
  return root;
}

describe('complete development app cache', () => {
  test('an unavailable explicit signing identity fails without publishing or falling back', async () => {
    const root = await fixtureRoot();
    const attempted = [];
    try {
      const request = { root, hostPath: join(root, 'host'), environment: { NOMIFUN_MACOS_DEV_SIGN_IDENTITY: 'Unavailable identity' } };
      await expect(ensureMacosDevelopmentBundle(request, {
        sign: async options => { attempted.push(options); throw new Error('installed identity missing'); },
        inspect: async () => ({ status: 'pass' }), run: async () => {},
      })).rejects.toThrow('installed identity missing');
      expect(attempted.map(options => options.identity)).toEqual(['Unavailable identity']);
      await expect(readFile(join(dirname(dirname(attempted[0].appPath)), 'complete.json'))).rejects.toThrow('ENOENT');
    } finally { await rm(root, { recursive: true, force: true }); }
  });

  test('stable signing identity reaches full signing and incremental sealing and isolates caches', async () => {
    const root = await fixtureRoot();
    const staged = [];
    const commands = [];
    const hooks = {
      sign: async request => { staged.push(request); },
      inspect: async () => ({ status: 'pass' }),
      run: async (program, args) => { commands.push([program, ...args]); },
    };
    const identity = 'Apple Development: Fixture (TEAMFIXTURE)';
    const request = { root, hostPath: join(root, 'host'), environment: {} };
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
      expect(plist).toContain('<key>NSAppTransportSecurity</key><dict><key>NSAllowsArbitraryLoadsInWebContent</key><true/></dict>');
      expect(plist).toContain('existing camera reason');
      const hooks = { sign: async () => {}, inspect: async () => ({ status: 'pass' }), run: async () => {} };
      const first = await ensureMacosDevelopmentBundle({ root, hostPath: join(root, 'host'), environment }, hooks);
      const second = await ensureMacosDevelopmentBundle({ root, hostPath: join(root, 'host'), environment: { ...environment, NOMIFUN_DATA_DIR: '/second-data' } }, hooks);
      expect(first).not.toBe(second);
      expect(first).toEndWith(join('.app', 'Contents', 'MacOS', 'nomifun-desktop'));
      expect(await readFile(join(dirname(dirname(first)), 'Info.plist'), 'utf8')).toContain('com.nomifun.fixture');
    } finally { await rm(root, { recursive: true, force: true }); }
  });

  test('incremental host rebuild preserves immutable resources; changed library or broken seal rebuilds the bundle', async () => {
    const root = await fixtureRoot();
    let stages = 0;
    let broken = false;
    const commands = [];
    const hooks = {
      sign: async () => { stages++; broken = false; },
      inspect: async () => ({ status: 'pass' }),
      run: async (program, args) => {
        commands.push([program, ...args]);
        if (broken && program === 'codesign' && args[0] === '--verify') throw new Error('broken app seal');
      },
    };
    const request = { root, hostPath: join(root, 'host'), bundleFiles: { 'Frameworks/native.dylib': join(root, 'native-library') }, environment: {} };
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
      expect(await readFile(join(dirname(dirname(program)), 'Frameworks/native.dylib'), 'utf8')).toBe('library-one');
      expect(commands.some(command => command.includes('--force'))).toBe(true);
      await writeFile(request.bundleFiles['Frameworks/native.dylib'], 'library-two');
      const changed = await ensureMacosDevelopmentBundle(request, hooks);
      expect(stages).toBe(2);
      expect(await readFile(join(dirname(dirname(changed)), 'Frameworks/native.dylib'), 'utf8')).toBe('library-two');
      expect(await readFile(join(dirname(dirname(program)), 'Frameworks/native.dylib'), 'utf8')).toBe('library-one');
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

  test('fast build keeps its verified app when a watcher replaces raw outputs during signing', async () => {
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
        target: 'x86_64-apple-darwin',
        sign: async ({ appPath, identity, target }) => {
          // Simulate Cargo watch and a later bundler replacing raw outputs
          // while signing this immutable generation.
          await writeFile(join(root, 'nomifun-desktop'), 'watcher-build');
          await writeFile(join(original, 'Contents/MacOS/nomifun-desktop'), 'later-bundler-build');
          await writeFile(join(original, 'Contents/Info.plist'), 'later-bundler-identity');
          expect(target).toBe('x86_64-apple-darwin');
          expect(identity).toBe('Apple Development: Fast Fixture (TEAMFIXTURE)');
          expect(appPath).not.toBe(original);
          expect(await readFile(join(appPath, 'Contents/MacOS/nomifun-desktop'), 'utf8')).toBe('this-fast-build');
          expect(await readFile(join(appPath, 'Contents/Info.plist'), 'utf8')).toBe('this-fast-build-identity');
        },
      });
      expect(appPath).toContain(`${sep}${join('bundle', 'macos-fast', 'run-')}`);
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
    const root = join(tmpdir(), 'nomifun-owned-workspace');
    const cache = join(root, 'target', 'macos-dev-app');
    const request = { program: join(cache, 'key', 'NomiFun Dev.app', 'Contents', 'MacOS', 'nomifun-desktop'), args: ['中文'], environment: { NOMIFUN_DATA_DIR: '/isolated' }, cwd: join(root, 'apps', 'desktop') };
    expect(validateDevelopmentLaunch(request, root).environment.NOMIFUN_DATA_DIR).toBe('/isolated');
    expect(() => validateDevelopmentLaunch({ ...request, program: join(tmpdir(), 'Applications', 'Other.app', 'Contents', 'MacOS', 'nomifun-desktop') }, root)).toThrow('owned');
    expect(() => validateDevelopmentLaunch({ ...request, program: join(cache, 'key', 'NomiFun Dev.app', 'Contents', 'MacOS', 'other-binary') }, root)).toThrow('owned');
    expect(() => validateDevelopmentLaunch({ ...request, program: join(root, 'target', 'macos-dev-app-other', 'Other.app', 'Contents', 'MacOS', 'nomifun-desktop') }, root)).toThrow('owned');
    expect(() => validateDevelopmentLaunch({ ...request, cwd: join(tmpdir(), 'outside') }, root)).toThrow('workspace');
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

  test('an abnormal app exit cannot start another development generation before native cleanup', async () => {
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
