import { describe, expect, test } from 'bun:test';
import { mkdtemp, writeFile, mkdir, readFile, rm, chmod } from 'node:fs/promises';
import { join, dirname, sep } from 'node:path';
import { tmpdir } from 'node:os';
import { EventEmitter, once } from 'node:events';
import { connect, createServer } from 'node:net';
import { createMacosDevLifetime } from './run-dev.mjs';
import { createMacosDevSupervisor, acquireMacosDevGeneration, validateDevelopmentLaunch } from './lib/macos-dev-supervisor.mjs';
import { parseMacosCargoRunnerArguments, macosDevelopmentApplicationArguments, developmentInfoPlist, ensureMacosDevelopmentBundle, runCargoArtifact } from './run-macos-dev-runner.mjs';
import { fastBuildInvocation, findBuiltMacosApp, completeFastMacosApp } from './run-fast-build.mjs';
import { prepareMacosBuildEnvironment } from './lib/macos-build-environment.mjs';
import { INTEL_ONNX_RUNTIME } from './lib/macos-onnx-runtime.mjs';
import { macosDevelopmentSigningIdentity, macosDevelopmentSigningNotice } from './lib/macos-dev-signing.mjs';
import { MacosApplicationLaunchCancelled } from './lib/macos-application-launch.mjs';

// These are routing/queue fixtures. A successful result explicitly represents
// authenticated App cleanup and App exit, never the controller's own exit.
function fixtureApplication(child) {
  return { exit: new Promise((accept, reject) => {
    child.once('application-exit', accept);
    child.once('error', reject);
  }) };
}

function deferred() {
  let resolve, reject;
  const promise = new Promise((accept, fail) => { resolve = accept; reject = fail; });
  promise.catch(() => {});
  return { promise, resolve, reject };
}

function observeControlAcquires(notify) {
  let connectionIndex = 0;
  return receiveConnection => createServer(socket => {
    const index = connectionIndex++;
    let buffered = '';
    // Both this observer and production receive run synchronously in one data
    // dispatch. An awaiting test resumes only after production enqueues acquire.
    socket.on('data', bytes => {
      buffered += bytes.toString();
      let end;
      while ((end = buffered.indexOf('\n')) >= 0) {
        const frame = buffered.slice(0, end); buffered = buffered.slice(end + 1);
        try { if (JSON.parse(frame).command === 'acquire') notify(index); } catch {}
      }
    });
    receiveConnection(socket);
  });
}

// These ports model already-authenticated App results and owned lifetime
// resources. They never start an App or claim native acceptance. Gates control
// the meaningful preparation, handoff and cleanup boundaries without timers.
async function restartFixture({
  prepareLifetime = async () => {}, prepareLaunch = async () => {},
  deliverHandle = async () => {}, stopLifetime = async () => {},
} = {}) {
  const signals = new Map();
  const wait = key => {
    if (!signals.has(key)) signals.set(key, deferred());
    return signals.get(key).promise;
  };
  const signal = (key, value) => { wait(key); signals.get(key).resolve(value); };
  const lifetimes = [];
  const attempts = [];
  const submissions = [];
  const request = { program: '/workspace/target/macos-dev-app/frozen/NomiFun Dev.app/Contents/MacOS/nomifun-desktop',
    controllerProgram: '/workspace/target/macos-dev-app/controller-frozen/macos_application_controller',
    args: ['--fixture', '中文', '-ApplePersistenceIgnoreState', 'YES'],
    environment: { NOMIFUN_DATA_DIR: '/isolated-data', FIXTURE_VALUE: 'unchanged' }, cwd: '/workspace' };
  const supervisor = await createMacosDevSupervisor({
    root: '/workspace', validateLaunch: request => request,
    createControlServer: observeControlAcquires(index => signal(`acquire-received:${index}`)),
    async createLifetime() {
      const lifetime = { index: lifetimes.length, socketPath: `/fake-native-lifetime-${lifetimes.length}`, stops: 0 };
      lifetimes.push(lifetime);
      signal(`lifetime-created:${lifetime.index}`, lifetime);
      await prepareLifetime(lifetime);
      return { socketPath: lifetime.socketPath, async stop() {
        lifetime.stops++;
        signal(`stop-entered:${lifetime.index}`, lifetime);
        try { await stopLifetime(lifetime); }
        catch (error) { signal(`stop-failed:${lifetime.index}`, error); throw error; }
        signal(`stop-finished:${lifetime.index}`, lifetime);
      } };
    },
    async launchApplication(request, options) {
      const attempt = { index: attempts.length, request, options, exit: deferred() };
      attempts.push(attempt);
      signal(`preparing:${attempt.index}`, attempt);
      await prepareLaunch(attempt);
      if (!options.isLaunchAllowed()) throw new MacosApplicationLaunchCancelled();
      submissions.push(attempt);
      signal(`submitted:${attempt.index}`, attempt);
      await deliverHandle(attempt);
      signal(`handle-delivered:${attempt.index}`, attempt);
      return { exit: attempt.exit.promise };
    },
  });
  const generation = await acquireMacosDevGeneration(supervisor.socketPath);
  let exited;
  const complete = (index, overrides = {}) => attempts[index].exit.resolve({
    code: 0, signal: null, cleanupVerified: true, restartRequested: false, pid: 43000 + index, ...overrides,
  });
  return {
    supervisor, generation, lifetimes, attempts, submissions, request, wait, complete,
    start() { exited = generation.launch(request); exited.catch(() => {}); return exited; },
    async dispose() {
      generation.close();
      for (const attempt of attempts) complete(attempt.index);
      await Promise.allSettled([exited, supervisor.stop()]);
      await rm(dirname(supervisor.socketPath), { recursive: true, force: true });
    },
  };
}

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

  test('isolates AppKit restoration for the dev process while preserving application arguments', () => {
    const args = Object.freeze(['--fixture', '中文', 'nomifun-dev://fixture']);
    expect(macosDevelopmentApplicationArguments(args)).toEqual([
      '--fixture', '中文', 'nomifun-dev://fixture', '-ApplePersistenceIgnoreState', 'YES',
    ]);
    expect(args).toEqual(['--fixture', '中文', 'nomifun-dev://fixture']);
    expect(macosDevelopmentApplicationArguments([])).toEqual(['-ApplePersistenceIgnoreState', 'YES']);
    const plan = parseMacosCargoRunnerArguments(['run', '--profile', 'release', '--', '--fixture'], 'arm64');
    expect(plan.build).not.toContain('-ApplePersistenceIgnoreState');
    expect(macosDevelopmentApplicationArguments(plan.application)).toEqual(['--fixture', '-ApplePersistenceIgnoreState', 'YES']);
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
      await writeFile(cargo, '#!/bin/sh\n' +
        'for argument do\n' +
        '  if [ "$argument" = "--fail" ]; then\n' +
        '    printf \'%s\\n\' \'could not compile fixture\' >&2\n' +
        '    exit 101\n' +
        '  fi\n' +
        'done\n' +
        'printf \'%s\\n\' \'{"reason":"compiler-artifact","target":{"name":"nomifun-desktop"},"executable":"target/debug/nomifun-desktop"}\'\n');
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
    expect(compiler.bundleConfig).toBe(join('/owned/onnx', 'tauri.conf.json'));
    expect(compiler.bundleFiles).toEqual({
      [`Frameworks/${INTEL_ONNX_RUNTIME.library}`]: join('/owned/onnx/lib', INTEL_ONNX_RUNTIME.library),
      'Resources/onnxruntime-LICENSE': join('/owned/onnx', 'LICENSE'),
      'Resources/onnxruntime-ThirdPartyNotices.txt': join('/owned/onnx', 'ThirdPartyNotices.txt'),
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
  await writeFile(join(root, 'apps/desktop/entitlements.plist'), await readFile(new URL('../apps/desktop/entitlements.plist', import.meta.url)));
  await writeFile(join(root, 'scripts/lib/macos-app-bundle.mjs'), '// fixture signing version');
  await writeFile(join(root, 'scripts/lib/macos-signing.mjs'), '// fixture app signing arguments');
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
      expect(seal).toEqual(['codesign', '--force', '--options', 'runtime', '--timestamp', '--entitlements', join(root, 'apps/desktop/entitlements.plist'), '--sign', identity, dirname(dirname(dirname(rebuilt)))]);
      expect(staged[1].entitlementsPath).toBe(join(root, 'apps/desktop/entitlements.plist'));
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

  test('changed media entitlements invalidate a completed dev app rather than reusing its old signature', async () => {
    const root = await fixtureRoot();
    const staged = [];
    const hooks = { sign: async request => staged.push(request), inspect: async () => ({ status: 'pass' }), run: async () => {} };
    const request = { root, hostPath: join(root, 'host'), environment: {} };
    try {
      const original = await ensureMacosDevelopmentBundle(request, hooks);
      const entitlementsPath = join(root, 'apps/desktop/entitlements.plist');
      await writeFile(entitlementsPath, `${await readFile(entitlementsPath, 'utf8')}\n<!-- changed source -->\n`);
      const changed = await ensureMacosDevelopmentBundle(request, hooks);
      expect(changed).not.toBe(original);
      expect(staged).toHaveLength(2);
      expect(staged[1].entitlementsPath).toBe(entitlementsPath);
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

describe('macOS authenticated application restart ownership', () => {
  test('one live runner lease restarts only after old App proof and old lifetime cleanup', async () => {
    const oldStop = deferred();
    const fixture = await restartFixture({ stopLifetime: lifetime => lifetime.index === 0 ? oldStop.promise : undefined });
    let next;
    let nextReady = false;
    let finalSettled = false;
    const exited = fixture.start();
    exited.finally(() => { finalSettled = true; }).catch(() => {});
    try {
      await fixture.wait('submitted:0');
      const queued = acquireMacosDevGeneration(fixture.supervisor.socketPath);
      queued.then(() => { nextReady = true; }, () => {});
      await fixture.wait('acquire-received:1');
      fixture.complete(0, { restartRequested: true });
      await fixture.wait('stop-entered:0');
      expect(fixture.submissions).toHaveLength(1);
      expect(nextReady).toBe(false);
      expect(finalSettled).toBe(false);
      oldStop.resolve();
      await fixture.wait('submitted:1');
      expect(fixture.lifetimes.map(lifetime => lifetime.socketPath)).toEqual(['/fake-native-lifetime-0', '/fake-native-lifetime-1']);
      const [old, fresh] = fixture.submissions.map(attempt => attempt.request);
      expect(fresh).toMatchObject({ program: old.program, controllerProgram: old.controllerProgram, args: old.args, cwd: old.cwd,
        applicationScope: 'desktop_development' });
      expect(fresh.environment).toEqual({ ...old.environment, NOMIFUN_DEV_LIFETIME_SOCKET: '/fake-native-lifetime-1' });
      expect(nextReady).toBe(false);
      expect(finalSettled).toBe(false);
      fixture.complete(1);
      expect(await exited).toMatchObject({ pid: 43001, restartRequested: false, cleanupVerified: true });
      next = await queued;
      expect(nextReady).toBe(true);
      expect(fixture.submissions).toHaveLength(2);
      expect(fixture.lifetimes.slice(0, 2).map(lifetime => lifetime.stops)).toEqual([1, 1]);
    } finally { oldStop.resolve(); next?.close(); await fixture.dispose(); }
  });

  test('CLI stop wins an already-authenticated restart while old lifetime teardown is pending', async () => {
    const teardown = deferred();
    const fixture = await restartFixture({ stopLifetime: () => teardown.promise });
    let stopped;
    const exited = fixture.start();
    try {
      await fixture.wait('submitted:0');
      fixture.complete(0, { restartRequested: true });
      await fixture.wait('stop-entered:0');
      stopped = fixture.supervisor.stop();
      stopped.catch(() => {});
      teardown.resolve();
      expect((await exited).restartRequested).toBe(true); // The App fact is retained; the request was suppressed.
      await stopped;
      expect(fixture.submissions).toHaveLength(1);
      expect(fixture.lifetimes).toHaveLength(1);
    } finally { teardown.resolve(); await fixture.dispose(); await Promise.allSettled([stopped]); }
  });

  test.each(['lifetime-creation', 'descriptor-preparation'])('CLI stop during successor %s suppresses its submission', async stage => {
    const preparation = deferred();
    const fixture = await restartFixture({
      prepareLifetime: lifetime => stage === 'lifetime-creation' && lifetime.index === 1 ? preparation.promise : undefined,
      prepareLaunch: attempt => stage === 'descriptor-preparation' && attempt.index === 1 ? preparation.promise : undefined,
    });
    const exited = fixture.start();
    let stopped;
    try {
      await fixture.wait('submitted:0');
      fixture.complete(0, { restartRequested: true });
      await fixture.wait(stage === 'lifetime-creation' ? 'lifetime-created:1' : 'preparing:1');
      stopped = fixture.supervisor.stop();
      stopped.catch(() => {});
      preparation.resolve();
      expect((await exited).pid).toBe(43000);
      await stopped;
      expect(fixture.submissions).toHaveLength(1);
      expect(fixture.lifetimes.map(lifetime => lifetime.stops)).toEqual([1, 1]);
    } finally { preparation.resolve(); await fixture.dispose(); await Promise.allSettled([stopped]); }
  });

  test('a cancelled fresh lifetime cleanup failure stays failed after normal old App evidence', async () => {
    const creation = deferred();
    const fixture = await restartFixture({
      prepareLifetime: lifetime => lifetime.index === 1 ? creation.promise : undefined,
      stopLifetime: lifetime => { if (lifetime.index === 1) throw new Error('fresh lifetime cleanup failed'); },
    });
    const exited = fixture.start();
    let stopped;
    try {
      await fixture.wait('submitted:0');
      fixture.complete(0, { restartRequested: true });
      await fixture.wait('lifetime-created:1');
      stopped = fixture.supervisor.stop();
      stopped.catch(() => {});
      creation.resolve();
      await expect(exited).rejects.toThrow('fresh lifetime cleanup failed');
      await expect(stopped).rejects.toThrow('fresh lifetime cleanup failed');
      await expect(fixture.supervisor.stop()).rejects.toThrow('fresh lifetime cleanup failed');
      expect(fixture.submissions).toHaveLength(1);
      expect(fixture.lifetimes[1].stops).toBe(1);
    } finally { creation.resolve(); await fixture.dispose(); await Promise.allSettled([stopped]); }
  });

  test('stop after successor submission waits that App and old stop callbacks cannot reach a new lifetime', async () => {
    const fixture = await restartFixture();
    const exited = fixture.start();
    let stopped;
    let finalSettled = false;
    let stopSettled = false;
    exited.finally(() => { finalSettled = true; }).catch(() => {});
    try {
      await fixture.wait('submitted:0');
      fixture.complete(0, { restartRequested: true });
      await fixture.wait('submitted:1');
      stopped = fixture.supervisor.stop();
      stopped.finally(() => { stopSettled = true; }).catch(() => {});
      await fixture.wait('stop-finished:1');
      expect(finalSettled).toBe(false);
      expect(stopSettled).toBe(false);
      expect(fixture.lifetimes.map(lifetime => lifetime.stops)).toEqual([1, 1]);
      fixture.complete(1, { restartRequested: true });
      expect((await exited).pid).toBe(43001);
      await stopped;
      expect(fixture.submissions).toHaveLength(2);
      expect(fixture.lifetimes).toHaveLength(2);
      expect(fixture.lifetimes.map(lifetime => lifetime.stops)).toEqual([1, 1]);
    } finally { await fixture.dispose(); await Promise.allSettled([stopped]); }
  });

  test.each([false, true])('stop retains a submitted App with late handle; lifetime failure=%s', async failStop => {
    const handoff = deferred();
    const fixture = await restartFixture({
      deliverHandle: () => handoff.promise,
      stopLifetime: () => { if (failStop) throw new Error('owned lifetime stop failed'); },
    });
    const exited = fixture.start();
    let next;
    let nextReady = false;
    try {
      await fixture.wait('submitted:0');
      fixture.generation.close();
      const queued = acquireMacosDevGeneration(fixture.supervisor.socketPath);
      queued.then(value => { next = value; nextReady = true; }, () => {});
      await fixture.wait('acquire-received:1');
      await fixture.wait(failStop ? 'stop-failed:0' : 'stop-finished:0');
      expect(nextReady).toBe(false);
      expect(fixture.submissions).toHaveLength(1);
      fixture.complete(0, { restartRequested: true });
      // Even a completed App result cannot release its late, owned handle.
      expect(nextReady).toBe(false);
      handoff.resolve();
      if (failStop) await expect(queued).rejects.toThrow('owned lifetime stop failed');
      else next = await queued;
      expect(fixture.submissions).toHaveLength(1);
      expect(fixture.lifetimes[0].stops).toBe(1);
    } finally { handoff.resolve(); next?.close(); await fixture.dispose(); await Promise.allSettled([exited]); }
  });

  test('an ordinary launch rejection after observed stop is not converted to no-start success', async () => {
    const preparing = deferred();
    const fixture = await restartFixture({ prepareLaunch: async attempt => {
      if (attempt.index === 1) { await preparing.promise; throw new Error('ordinary launch failure'); }
    } });
    const exited = fixture.start();
    let stopped;
    try {
      await fixture.wait('submitted:0');
      fixture.complete(0, { restartRequested: true });
      await fixture.wait('preparing:1');
      stopped = fixture.supervisor.stop();
      stopped.catch(() => {});
      preparing.resolve();
      await expect(exited).rejects.toThrow('ordinary launch failure');
      await expect(stopped).rejects.toThrow('ordinary launch failure');
      expect(fixture.submissions).toHaveLength(1);
    } finally { preparing.resolve(); await fixture.dispose(); await Promise.allSettled([stopped]); }
  });

  test('the first launch failure survives a later lifetime teardown failure', async () => {
    const fixture = await restartFixture({
      prepareLaunch: () => { throw new Error('first launch failure'); },
      stopLifetime: () => { throw new Error('later teardown failure'); },
    });
    const exited = fixture.start();
    try {
      await expect(exited).rejects.toThrow('first launch failure');
      await expect(acquireMacosDevGeneration(fixture.supervisor.socketPath)).rejects.toThrow('first launch failure');
      await expect(fixture.supervisor.stop()).rejects.toThrow('first launch failure');
      expect(fixture.submissions).toHaveLength(0);
      expect(fixture.lifetimes[0].stops).toBe(1);
    } finally { await fixture.dispose(); }
  });

  test('typed no-start cancellation without an observed stop cannot release admission', async () => {
    const fixture = await restartFixture({ prepareLaunch: () => { throw new MacosApplicationLaunchCancelled(); } });
    const exited = fixture.start();
    try {
      await expect(exited).rejects.toThrow('cancelled before controller start');
      await expect(acquireMacosDevGeneration(fixture.supervisor.socketPath)).rejects.toThrow('cancelled before controller start');
      expect(fixture.submissions).toHaveLength(0);
    } finally { await fixture.dispose(); }
  });

  test.each([
    { restartRequested: undefined }, { restartRequested: null }, { restartRequested: 'true' }, { restartRequested: 1 },
    { restartRequested: true, cleanupVerified: false }, { restartRequested: true, cleanupVerified: 1 },
    { restartRequested: true, code: 1 }, { restartRequested: true, signal: 'SIGTERM' },
  ])('a restart result with incomplete evidence %j closes admission', async result => {
    const fixture = await restartFixture();
    const exited = fixture.start();
    try {
      await fixture.wait('submitted:0');
      fixture.complete(0, result);
      await expect(exited).rejects.toThrow('cleanup is unconfirmed');
      await expect(acquireMacosDevGeneration(fixture.supervisor.socketPath)).rejects.toThrow('cleanup is unconfirmed');
      expect(fixture.submissions).toHaveLength(1);
    } finally { await fixture.dispose(); }
  });

  test('watcher FIN suppresses restart before control close and still blocks next admission until App proof', async () => {
    const appExit = deferred();
    const appSubmitted = deferred();
    const stopEntered = deferred();
    const acquireReceived = deferred();
    let launches = 0;
    const supervisor = await createMacosDevSupervisor({
      root: '/workspace', validateLaunch: request => request,
      createControlServer: observeControlAcquires(index => { if (index === 1) acquireReceived.resolve(); }),
      createLifetime: async () => ({ socketPath: '/fake-fin-lifetime', stop: async () => { stopEntered.resolve(); } }),
      launchApplication() { launches++; appSubmitted.resolve(); return { exit: appExit.promise }; },
    });
    const control = connect({ path: supervisor.socketPath, allowHalfOpen: true });
    control.on('error', () => {});
    let next;
    let nextReady = false;
    try {
      await once(control, 'connect');
      const ready = once(control, 'data');
      control.write(`${JSON.stringify({ command: 'acquire' })}\n`);
      await ready;
      control.write(`${JSON.stringify({ command: 'launch', program: '/owned.app/Contents/MacOS/nomifun-desktop',
        args: [], environment: {}, cwd: '/workspace' })}\n`);
      await appSubmitted.promise;
      control.end(); // FIN is an observed stop, not proof of App exit.
      await stopEntered.promise;
      const queued = acquireMacosDevGeneration(supervisor.socketPath);
      queued.then(value => { next = value; nextReady = true; }, () => {});
      await acquireReceived.promise;
      expect(nextReady).toBe(false);
      expect(launches).toBe(1);
      appExit.resolve({ code: 0, signal: null, cleanupVerified: true, restartRequested: true });
      next = await queued;
      expect(launches).toBe(1);
    } finally {
      control.destroy(); next?.close();
      appExit.resolve({ code: 0, signal: null, cleanupVerified: true, restartRequested: false });
      await supervisor.stop();
    }
  });
});

describe('macOS watch generation ownership', () => {
  test('every watch generation forwards process-scoped restoration arguments without changing identity or data root', async () => {
    const root = join(tmpdir(), 'nomifun-owned-workspace');
    const program = join(root, 'target', 'macos-dev-app', 'same-identity', 'generation-fixture', 'NomiFun Dev.app', 'Contents', 'MacOS', 'nomifun-desktop');
    const controllerProgram = join(root, 'target', 'macos-dev-app', 'controller-fixture', 'macos_application_controller');
    const cwd = join(root, 'apps', 'desktop');
    const children = [];
    const launches = [];
    const supervisor = await createMacosDevSupervisor({
      root,
      createLifetime: async () => ({ socketPath: '/fake-native-lifetime', stop: async () => {} }),
      launchApplication({ program, controllerProgram, args, environment, cwd, applicationScope }) {
        const options = { env: environment, cwd };
        launches.push({ program, controllerProgram, args, options, applicationScope });
        const child = new EventEmitter();
        children.push(child);
        return fixtureApplication(child);
      },
    });
    try {
      for (const label of ['first', 'watch-restart']) {
        const generation = await acquireMacosDevGeneration(supervisor.socketPath);
        const plan = parseMacosCargoRunnerArguments(['run', '--', '--fixture', label], 'arm64');
        const exited = generation.launch({ program, controllerProgram, args: macosDevelopmentApplicationArguments(plan.application), environment: { NOMIFUN_DATA_DIR: '/isolated-owned-data' }, cwd });
        const index = label === 'first' ? 0 : 1;
        while (!children[index]) await new Promise(accept => setImmediate(accept));
        expect(launches[index].program).toBe(program);
        expect(launches[index].controllerProgram).toBe(controllerProgram);
        expect(launches[index].applicationScope).toBe('desktop_development');
        expect(launches[index].args).toEqual(['--fixture', label, '-ApplePersistenceIgnoreState', 'YES']);
        expect(launches[index].options.env.NOMIFUN_DATA_DIR).toBe('/isolated-owned-data');
        expect(launches[index].options.env.NOMIFUN_DEV_LIFETIME_SOCKET).toBe('/fake-native-lifetime');
        expect(launches[index].options.cwd).toBe(cwd);
        children[index].emit('application-exit', { code: 0, signal: null, cleanupVerified: true, restartRequested: false });
        expect((await exited).code).toBe(0);
        generation.close();
      }
    } finally { await supervisor.stop(); }
  });

  test('runner death requests native cleanup and blocks the next launch until child exit', async () => {
    const children = [];
    const desktops = [];
    const supervisor = await createMacosDevSupervisor({
      root: '/workspace', createLifetime: createMacosDevLifetime,
      validateLaunch: request => request,
      launchApplication({ environment }) {
        const child = new EventEmitter();
        children.push(child);
        const desktop = connect({ path: environment.NOMIFUN_DEV_LIFETIME_SOCKET, allowHalfOpen: true });
        desktops.push(desktop);
        desktop.on('error', error => child.emit('error', error));
        desktop.on('data', bytes => child.emit('stop-request', bytes.toString()));
        desktop.once('connect', () => child.emit('connected'));
        return fixtureApplication(child);
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
      children[0].emit('exit', 0, null); // A controller exit is not authenticated App exit evidence.
      await new Promise(accept => setImmediate(accept));
      expect(admitted).toBe(false);
      children[0].emit('application-exit', { code: 0, signal: null, cleanupVerified: true, restartRequested: false });
      second = await next;
      expect(admitted).toBe(true);
    } finally {
      first.close(); second?.close();
      for (const desktop of desktops) desktop.destroy();
      await supervisor.stop();
    }
  });

  test('CLI stop requests native cleanup while retaining the active control lease until authenticated App exit', async () => {
    let child;
    let desktop;
    let forced = false;
    const supervisor = await createMacosDevSupervisor({
      root: '/workspace', createLifetime: createMacosDevLifetime,
      validateLaunch: request => request,
      launchApplication({ environment }) {
        child = new EventEmitter();
        child.kill = () => { forced = true; };
        desktop = connect({ path: environment.NOMIFUN_DEV_LIFETIME_SOCKET, allowHalfOpen: true });
        desktop.on('error', error => child.emit('error', error));
        desktop.on('data', bytes => child.emit('stop-request', bytes.toString()));
        desktop.once('connect', () => child.emit('connected'));
        return fixtureApplication(child);
      },
    });
    const generation = await acquireMacosDevGeneration(supervisor.socketPath);
    let exited;
    let stopped;
    try {
      let runnerSettled = false;
      let supervisorSettled = false;
      exited = generation.launch({ program: '/owned.app/Contents/MacOS/nomifun-desktop', args: [], environment: {}, cwd: '/workspace' });
      exited.then(() => { runnerSettled = true; }, () => { runnerSettled = true; });
      while (!child) await new Promise(accept => setImmediate(accept));
      await once(child, 'connected');
      const requested = once(child, 'stop-request');
      stopped = supervisor.stop();
      stopped.then(() => { supervisorSettled = true; }, () => { supervisorSettled = true; });
      expect((await requested)[0]).toBe('q');
      desktop.destroy();
      child.emit('exit', 0, null); // Neither lifetime EOF nor controller exit proves App cleanup.
      await new Promise(accept => setTimeout(accept, 20));
      expect(runnerSettled).toBe(false);
      expect(supervisorSettled).toBe(false);
      expect(forced).toBe(false);
      await expect(acquireMacosDevGeneration(supervisor.socketPath)).rejects.toThrow();
      child.emit('application-exit', { code: 0, signal: null, cleanupVerified: true, restartRequested: false, pid: 42001 });
      expect(await exited).toMatchObject({ event: 'exit', code: 0, signal: null, cleanupVerified: true, pid: 42001 });
      await stopped;
      expect(supervisorSettled).toBe(true);
      expect(forced).toBe(false);
    } finally {
      desktop?.destroy();
      child?.emit('application-exit', { code: 0, signal: null, cleanupVerified: true, restartRequested: false, pid: 42001 });
      generation.close();
      await Promise.allSettled([exited, stopped ?? supervisor.stop()]);
    }
  });

  test.each(['before-acquire', 'before-launch'])('CLI stop cancels a %s control lease without inventing App exit proof', async phase => {
    let launches = 0;
    let cleanups = 0;
    const supervisor = await createMacosDevSupervisor({
      root: '/workspace', shutdownTimeoutMs: 1000,
      createLifetime: async () => ({ socketPath: '/fake-native-lifetime', stop: async () => { cleanups++; } }),
      launchApplication() { launches++; throw new Error('an unlaunched lease cannot start an App'); },
    });
    const control = connect({ path: supervisor.socketPath, allowHalfOpen: true });
    const frames = [];
    control.setEncoding('utf8');
    control.on('data', bytes => { for (const line of bytes.trim().split('\n')) if (line) frames.push(JSON.parse(line)); });
    try {
      await once(control, 'connect');
      if (phase === 'before-launch') {
        const ready = once(control, 'data');
        control.write(`${JSON.stringify({ command: 'acquire' })}\n`);
        await ready;
        expect(frames).toEqual([{ event: 'ready' }]);
      }
      const ended = once(control, 'end');
      await supervisor.stop();
      await ended;
      expect(control.writableEnded).toBe(false); // The peer has not acknowledged EOF with its own FIN.
      expect(launches).toBe(0);
      expect(cleanups).toBe(phase === 'before-launch' ? 1 : 0);
      expect(frames.some(frame => frame.event === 'exit')).toBe(false);
      await expect(acquireMacosDevGeneration(supervisor.socketPath)).rejects.toThrow();
    } finally {
      control.destroy();
      await Promise.allSettled([supervisor.stop()]);
    }
  });

  test('CLI stop cancels queued admission while the active runner still awaits App exit proof', async () => {
    let child;
    let shutdownRequested;
    const requested = new Promise(accept => { shutdownRequested = accept; });
    let launches = 0;
    const supervisor = await createMacosDevSupervisor({
      root: '/workspace', validateLaunch: request => request,
      createLifetime: async () => ({ socketPath: '/fake-native-lifetime', stop: async () => { shutdownRequested(); } }),
      launchApplication() { launches++; child = new EventEmitter(); return fixtureApplication(child); },
    });
    const generation = await acquireMacosDevGeneration(supervisor.socketPath);
    const queued = connect(supervisor.socketPath);
    const queuedConnected = once(queued, 'connect');
    const queuedFrames = [];
    queued.setEncoding('utf8');
    queued.on('data', bytes => { for (const line of bytes.trim().split('\n')) if (line) queuedFrames.push(JSON.parse(line)); });
    let exited;
    let stopped;
    try {
      exited = generation.launch({ program: '/owned.app/Contents/MacOS/nomifun-desktop', args: [], environment: {}, cwd: '/workspace' });
      while (!child) await new Promise(accept => setImmediate(accept));
      await queuedConnected;
      queued.write(`${JSON.stringify({ command: 'acquire' })}\n`);
      const queuedClosed = once(queued, 'close');
      let runnerSettled = false;
      exited.then(() => { runnerSettled = true; }, () => { runnerSettled = true; });
      stopped = supervisor.stop();
      await requested;
      await queuedClosed;
      expect(queuedFrames).toEqual([]);
      expect(launches).toBe(1);
      expect(runnerSettled).toBe(false);
      child.emit('application-exit', { code: 0, signal: null, cleanupVerified: true, restartRequested: false });
      expect((await exited).cleanupVerified).toBe(true);
      await stopped;
      expect(launches).toBe(1);
    } finally {
      child?.emit('application-exit', { code: 0, signal: null, cleanupVerified: true, restartRequested: false });
      queued.destroy(); generation.close();
      await Promise.allSettled([exited, stopped ?? supervisor.stop()]);
    }
  });

  test('a CLI stop timeout remains sticky and never turns control EOF into cleanup proof or force termination', async () => {
    let child;
    let forced = false;
    const supervisor = await createMacosDevSupervisor({
      root: '/workspace', validateLaunch: request => request, shutdownTimeoutMs: 25,
      createLifetime: async () => ({ socketPath: '/fake-native-lifetime', stop: async () => {} }),
      launchApplication() {
        child = new EventEmitter(); child.kill = () => { forced = true; };
        return fixtureApplication(child);
      },
    });
    const generation = await acquireMacosDevGeneration(supervisor.socketPath);
    let exited;
    try {
      exited = generation.launch({ program: '/owned.app/Contents/MacOS/nomifun-desktop', args: [], environment: {}, cwd: '/workspace' });
      let runnerSettled = false;
      exited.then(() => { runnerSettled = true; }, () => { runnerSettled = true; });
      while (!child) await new Promise(accept => setImmediate(accept));
      await expect(supervisor.stop()).rejects.toThrow('verified shutdown');
      expect(runnerSettled).toBe(false);
      expect(forced).toBe(false);
      await expect(acquireMacosDevGeneration(supervisor.socketPath)).rejects.toThrow();
      child.emit('application-exit', { code: 0, signal: null, cleanupVerified: true, restartRequested: false });
      expect((await exited).cleanupVerified).toBe(true);
      await expect(supervisor.stop()).rejects.toThrow('verified shutdown');
      expect(forced).toBe(false);
    } finally {
      child?.emit('application-exit', { code: 0, signal: null, cleanupVerified: true, restartRequested: false });
      generation.close();
      await Promise.allSettled([exited, supervisor.stop()]);
    }
  });

  test('rejects arbitrary launch paths and keeps each control socket private', async () => {
    const root = join(tmpdir(), 'nomifun-owned-workspace');
    const cache = join(root, 'target', 'macos-dev-app');
    const request = { program: join(cache, 'key', 'NomiFun Dev.app', 'Contents', 'MacOS', 'nomifun-desktop'), controllerProgram: join(cache, 'controller-fixture', 'macos_application_controller'), args: ['中文'], environment: { NOMIFUN_DATA_DIR: '/isolated' }, cwd: join(root, 'apps', 'desktop') };
    const validated = validateDevelopmentLaunch(request, root);
    expect(validated.environment.NOMIFUN_DATA_DIR).toBe('/isolated');
    expect(validated.controllerProgram).toBe(request.controllerProgram);
    for (const controllerProgram of [
      undefined,
      join(tmpdir(), 'external-controller', 'macos_application_controller'),
      join(root, 'target', 'macos-dev-app-other', 'macos_application_controller'),
      join(cache, 'controller-fixture', 'other-controller'),
      join(cache, 'controller-fixture', 'unrelated_macos_application_controller'),
      join(cache, '..', 'outside-controller', 'macos_application_controller'),
    ]) {
      expect(() => validateDevelopmentLaunch({ ...request, controllerProgram }, root)).toThrow('owned compiled LaunchServices controller');
    }
    expect(() => validateDevelopmentLaunch({ ...request, program: join(tmpdir(), 'Applications', 'Other.app', 'Contents', 'MacOS', 'nomifun-desktop') }, root)).toThrow('owned');
    expect(() => validateDevelopmentLaunch({ ...request, program: join(cache, 'key', 'NomiFun Dev.app', 'Contents', 'MacOS', 'other-binary') }, root)).toThrow('owned');
    expect(() => validateDevelopmentLaunch({ ...request, program: join(root, 'target', 'macos-dev-app-other', 'Other.app', 'Contents', 'MacOS', 'nomifun-desktop') }, root)).toThrow('owned');
    expect(() => validateDevelopmentLaunch({ ...request, cwd: join(tmpdir(), 'outside') }, root)).toThrow('workspace');
  });

  test('unproven native cleanup closes admission and never force kills the app', async () => {
    let child;
    let forced = false;
    let stopFailed;
    const failed = new Promise(accept => { stopFailed = accept; });
    const supervisor = await createMacosDevSupervisor({
      root: '/workspace', validateLaunch: request => request,
      createLifetime: async () => ({ socketPath: '/fake-native-lifetime', stop: async () => { stopFailed(); throw new Error('native cleanup unconfirmed'); } }),
      launchApplication() {
        child = new EventEmitter();
        child.kill = () => { forced = true; };
        return fixtureApplication(child);
      },
    });
    const first = await acquireMacosDevGeneration(supervisor.socketPath);
    try {
      first.launch({ program: '/owned.app/Contents/MacOS/nomifun-desktop', args: [], environment: {}, cwd: '/workspace' }).catch(() => {});
      while (!child) await new Promise(accept => setImmediate(accept));
      first.close();
      await failed;
      let admitted = false;
      const next = acquireMacosDevGeneration(supervisor.socketPath);
      next.then(() => { admitted = true; }, () => {});
      expect(admitted).toBe(false);
      // Stop failure cannot abandon the still-owned actual App exit waiter.
      // Normal App proof cannot repair the independent lifetime stop failure.
      child.emit('application-exit', { code: 0, signal: null, cleanupVerified: true, restartRequested: false });
      await expect(next).rejects.toThrow('cleanup unconfirmed');
      await expect(supervisor.stop()).rejects.toThrow('cleanup unconfirmed');
      expect(forced).toBe(false);
    } finally {
      child?.emit('application-exit', { code: 0, signal: null, cleanupVerified: true, restartRequested: false });
      first.close();
      await Promise.allSettled([supervisor.stop()]);
      await rm(dirname(supervisor.socketPath), { recursive: true, force: true });
    }
  });

  test('an abnormal app exit cannot start another development generation before native cleanup', async () => {
    let child;
    const supervisor = await createMacosDevSupervisor({
      root: '/workspace', validateLaunch: request => request,
      createLifetime: async () => ({ socketPath: '/fake-native-lifetime', stop: async () => {} }),
      launchApplication() { child = new EventEmitter(); return fixtureApplication(child); },
    });
    const first = await acquireMacosDevGeneration(supervisor.socketPath);
    try {
      const exited = first.launch({ program: '/owned.app/Contents/MacOS/nomifun-desktop', args: [], environment: {}, cwd: '/workspace' });
      while (!child) await new Promise(accept => setImmediate(accept));
      child.emit('application-exit', { code: null, signal: 'SIGKILL', cleanupVerified: false, restartRequested: false });
      await expect(exited).rejects.toThrow('cleanup is unconfirmed');
      await expect(acquireMacosDevGeneration(supervisor.socketPath)).rejects.toThrow('cleanup is unconfirmed');
      await expect(supervisor.stop()).rejects.toThrow('cleanup is unconfirmed');
    } finally {
      child?.emit('application-exit', { code: 0, signal: null, cleanupVerified: true, restartRequested: false });
      first.close();
      await Promise.allSettled([supervisor.stop()]);
      await rm(dirname(supervisor.socketPath), { recursive: true, force: true });
    }
  });
});
