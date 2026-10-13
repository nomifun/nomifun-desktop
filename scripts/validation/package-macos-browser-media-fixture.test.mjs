import { expect, test } from 'bun:test';
import { cp, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import { packageMacosBrowserMediaFixture } from './package-macos-browser-media-fixture.mjs';

test.skipIf(process.platform !== 'darwin')('proves exact supply before real Mach-O signing and preserves the signed final copy', async () => {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-media-signing-test-'));
  try {
    const source = join(root, 'fixture.c');
    const binary = join(root, 'fixture');
    await writeFile(source, 'int main(void) { return 0; }\n');
    const compiled = spawnSync('/usr/bin/clang', ['-o', binary, source], { encoding: 'utf8', timeout: 15000 });
    expect(compiled.status, compiled.stderr).toBe(0);
    const before = await readFile(binary);
    const report = await packageMacosBrowserMediaFixture({ binary, outputRoot: join(root, 'new-audit'), environment: { ...process.env, NOMIFUN_MACOS_DEV_SIGN_IDENTITY: '-' } });
    expect(report.supply_verified_before_signing).toBe(true);
    expect(report.pre_signing_copy_sha256).toBe(report.fixture_binary_sha256);
    expect(report.codesigned_binary_sha256).not.toBe(report.fixture_binary_sha256);
    expect(report.packaged_binary_sha256).toBe(report.codesigned_binary_sha256);
    expect(await readFile(binary)).toEqual(before);
    expect(report.productName).toBe('NomiFun WK Media Audit');
    expect(report.identifier).toBe('com.nomifun.desktop.audit.media');
    expect(report.packaging_only).toBe(true);
    expect(report.grants_media).toBe(false);
    expect(spawnSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', report.app]).status).toBe(0);
  } finally { await rm(root, { recursive: true, force: true }); }
}, 30000);

test.skipIf(process.platform !== 'darwin')('rejects mismatched copied supply before signing or launching', async () => {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-media-supply-test-'));
  let signed = false;
  let launched = false;
  try {
    const binary = join(root, 'fixture');
    await writeFile(binary, 'exact supplied bytes');
    await expect(packageMacosBrowserMediaFixture({ binary, controllerProgram: binary, outputRoot: join(root, 'new-audit'), launch: true }, {
      ensureBundle: async (_request, hooks) => {
        const appPath = join(root, 'bad-copy.app');
        const host = join(appPath, 'Contents', 'MacOS', 'nomifun-desktop');
        await mkdir(join(appPath, 'Contents', 'MacOS'), { recursive: true });
        await writeFile(host, 'different bytes');
        await hooks.sign({ appPath });
        return host;
      },
      signBundle: async () => { signed = true; },
      launchApplication: async () => { launched = true; },
    })).rejects.toThrow('Pre-signing fixture copy differs');
    expect(signed).toBe(false);
    expect(launched).toBe(false);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test.skipIf(process.platform !== 'darwin')('launches only through the fixed LaunchServices audit scope and requires actual application proof', async () => {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-media-controller-test-'));
  try {
    const binary = join(root, 'fixture');
    const controllerProgram = join(root, 'macos_application_controller');
    await writeFile(binary, 'exact fixture bytes');
    await writeFile(controllerProgram, 'compiled controller fixture');
    const hooks = {
      ensureBundle: async (_request, hooks) => {
        const appPath = join(await mkdtemp(join(root, 'cache-')), 'Audit.app');
        const program = join(appPath, 'Contents', 'MacOS', 'nomifun-desktop');
        await mkdir(join(appPath, 'Contents', 'MacOS'), { recursive: true });
        await cp(binary, program);
        await hooks.sign({ appPath });
        return program;
      },
      signBundle: async () => {},
      run: async (program, args) => {
        expect(program).toBe('/usr/bin/codesign');
        expect(args.slice(0, 3)).toEqual(['--verify', '--deep', '--strict']);
      },
    };
    const outputRoot = join(root, 'accepted');
    const report = await packageMacosBrowserMediaFixture({ binary, controllerProgram, outputRoot, launch: true, userReady: true,
      environment: { PATH: '/usr/bin:/bin', NOMIFUN_DATA_DIR: '/discarded-input' } }, {
      ...hooks,
      launchApplication: async launch => {
        expect(launch.applicationScope).toBe('browser_media_audit');
        expect(launch.controllerProgram).toBe(controllerProgram);
        expect(launch.args).toEqual(['--media-permissions', '--media-user-ready', '-ApplePersistenceIgnoreState', 'YES']);
        expect(launch.environment.NOMIFUN_DATA_DIR).toBe(join(outputRoot, 'data'));
        expect(launch.cwd).toBe(root);
        return { exit: Promise.resolve({ pid: 4242, code: 0, signal: null, cleanupVerified: true }) };
      },
    });
    expect(report.launch_transport).toBe('public_nsworkspace');
    expect(report.application_exit).toEqual({ pid: 4242, code: 0, signal: null, cleanup_verified: true });
    const refused = join(root, 'refused');
    await expect(packageMacosBrowserMediaFixture({ binary, controllerProgram, outputRoot: refused, launch: true }, {
      ...hooks,
      launchApplication: async () => ({ exit: Promise.resolve({ code: 0, signal: null, cleanupVerified: false }) }),
    })).rejects.toThrow('actual normal exit were not verified');
    expect(JSON.parse(await readFile(join(refused, 'package-report.json'), 'utf8')).application_exit).toBeUndefined();
    const missing = join(root, 'missing-controller');
    await expect(packageMacosBrowserMediaFixture({ binary, outputRoot: missing, launch: true })).rejects.toThrow('compiled --controller');
    await expect(stat(missing)).rejects.toHaveProperty('code', 'ENOENT');
  } finally { await rm(root, { recursive: true, force: true }); }
});

test.skipIf(process.platform !== 'darwin')('rejects an existing output and an unproved cached signed binary', async () => {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-media-output-test-'));
  try {
    const binary = join(root, 'fixture');
    await writeFile(binary, 'exact supplied bytes');
    const existing = join(root, 'existing');
    await mkdir(existing);
    await writeFile(join(existing, 'preserved'), 'unchanged');
    await expect(packageMacosBrowserMediaFixture({ binary, outputRoot: existing })).rejects.toThrow('EEXIST');
    expect(await readFile(join(existing, 'preserved'), 'utf8')).toBe('unchanged');
    await expect(packageMacosBrowserMediaFixture({ binary, outputRoot: join(root, 'new-cache-output') }, {
      ensureBundle: async () => binary,
    })).rejects.toThrow('pre-signing supply witness');
  } finally { await rm(root, { recursive: true, force: true }); }
});
