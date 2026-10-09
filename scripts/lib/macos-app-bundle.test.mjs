import { describe, expect, test } from 'bun:test';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { createMacosUpdaterArchive, inspectMacosAppBundle, resolveMacosBuildSettings, verifyMacosUpdaterArchive } from './macos-app-bundle.mjs';
import { desktopBuildCommand } from '../run-desktop-build.mjs';

const REPO = fileURLToPath(new URL('../../', import.meta.url));

function file(path, content, executable = false) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
  if (executable) chmodSync(path, 0o755);
}

function appFixture(root, architecture = 'arm64') {
  const app = join(root, 'NomiFun.app');
  file(join(app, 'Contents/MacOS/nomifun-desktop'), 'final host bytes', true);
  file(join(app, 'Contents/Info.plist'), '<plist/>');
  if (architecture === 'x86_64') {
    file(join(app, 'Contents/Frameworks/libonnxruntime.dylib'), 'Intel ONNX library', true);
    file(join(app, 'Contents/Resources/onnxruntime-LICENSE'), 'ONNX license');
  }
  return app;
}

describe('complete macOS application build routing', () => {
  test('generic builds use the macOS application packaging pipeline and preserve other platform builds', () => {
    expect(desktopBuildCommand('darwin', ['--config', 'overlay.json'])).toEqual({
      command: 'bash', args: [resolve(REPO, 'scripts/desktop-build-mac.sh'), '--config', 'overlay.json'], prune: false,
    });
    for (const platform of ['linux', 'win32']) {
      expect(desktopBuildCommand(platform, ['--config', 'overlay.json'])).toEqual({
        command: 'bun', args: ['x', 'tauri', 'build', '--config', 'apps/desktop/tauri.conf.json', '--config', 'overlay.json'], prune: true,
      });
    }
  });

  test('updater selection follows the final user overlay before the internal Tauri override', async () => {
    expect(await resolveMacosBuildSettings([], { root: REPO })).toEqual({ createUpdaterArtifacts: false });
    expect(await resolveMacosBuildSettings(['--config', 'apps/desktop/tauri.updater.conf.json'], { root: REPO }))
      .toEqual({ createUpdaterArtifacts: true });
    expect(await resolveMacosBuildSettings(['--config', 'apps/desktop/tauri.updater.conf.json', '--config={"bundle":{"createUpdaterArtifacts":false}}'], { root: REPO }))
      .toEqual({ createUpdaterArtifacts: false });
    await expect(resolveMacosBuildSettings(['--config', 'missing.json'], { root: REPO })).rejects.toThrow('readable JSON');
    await expect(resolveMacosBuildSettings(['--config', '{"productName":"Another App"}'], { root: REPO })).rejects.toThrow('NomiFun bundle name');
  });

  test.skipIf(process.platform === 'win32')('requires executable host and owned plist without a bundled browser', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-app-bundle-'));
    try {
      const app = appFixture(root);
      expect((await inspectMacosAppBundle(app)).status).toBe('pass');
      chmodSync(join(app, 'Contents/MacOS/nomifun-desktop'), 0o644);
      expect((await inspectMacosAppBundle(app)).missing.map(item => item.label)).toContain('host');
      chmodSync(join(app, 'Contents/MacOS/nomifun-desktop'), 0o755);
      rmSync(join(app, 'Contents/Info.plist'));
      file(join(root, 'external.plist'), '<plist/>');
      symlinkSync(join(root, 'external.plist'), join(app, 'Contents/Info.plist'));
      expect((await inspectMacosAppBundle(app)).missing.map(item => item.label)).toContain('Info.plist outside its application bundle');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  test.each(['Chromium Embedded Framework.framework', 'WebKit.framework', 'NomiFun Helper.app', 'NomiFun Helper (GPU).app', 'nomifun-browser-cef-helper', 'browser-cef'])('rejects bundled browser component %s', async name => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-no-browser-runtime-'));
    try {
      const app = appFixture(root);
      file(join(app, 'Contents/Frameworks', name, 'payload'), 'forbidden browser component');
      expect((await inspectMacosAppBundle(app)).missing.map(item => item.label)).toContain(`unexpected bundled browser runtime: ${name}`);
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});

describe.skipIf(process.platform === 'win32')('final App updater archives', () => {
  test.each(['._NomiFun.app', 'NomiFun.app/Contents/._Info.plist', '__MACOSX/metadata'])(
    'rejects metadata entry %s before approving an updater', async (entry) => {
      const root = mkdtempSync(join(tmpdir(), 'nomifun-app-updater-metadata-'));
      try {
        const app = appFixture(root);
        file(join(root, entry), 'AppleDouble metadata');
        const archive = join(root, 'invalid.tar.gz');
        const packed = spawnSync('/usr/bin/tar', ['-czf', archive, '-C', root,
          ...(entry.startsWith('NomiFun.app/') ? [] : [entry]), 'NomiFun.app'],
        { env: { ...process.env, COPYFILE_DISABLE: '1' }, encoding: 'utf8' });
        expect(packed.status).toBe(0);
        await expect(verifyMacosUpdaterArchive(archive, app)).rejects.toThrow(
          `incompatible with Tauri macOS installation: ${entry}`,
        );
      } finally { rmSync(root, { recursive: true, force: true }); }
    },
  );

  test.skipIf(process.platform !== 'darwin')('detects AppleDouble entries hidden by the system tar listing', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-app-updater-xattrs-'));
    try {
      const app = appFixture(root);
      const attributed = spawnSync('/usr/bin/xattr', ['-w', 'com.nomifun.updater.fixture', 'metadata', app]);
      expect(attributed.status).toBe(0);
      const archive = join(root, 'invalid.tar.gz');
      const environment = { ...process.env };
      delete environment.COPYFILE_DISABLE;
      const packed = spawnSync('/usr/bin/tar', ['-czf', archive, '-C', root, 'NomiFun.app'], { env: environment });
      expect(packed.status).toBe(0);
      const listed = spawnSync('/usr/bin/tar', ['-tzf', archive], { encoding: 'utf8' });
      expect(listed.status).toBe(0);
      expect(listed.stdout).not.toContain('._NomiFun.app');
      await expect(verifyMacosUpdaterArchive(archive, app)).rejects.toThrow(
        'incompatible with Tauri macOS installation: ._NomiFun.app',
      );
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  test.each(['permissions', 'plist', 'runtime'])(
    'rejects an extracted bundle with invalid %s', async (defect) => {
      const root = mkdtempSync(join(tmpdir(), 'nomifun-app-updater-invalid-'));
      try {
        const app = appFixture(root);
        if (defect === 'permissions') chmodSync(join(app, 'Contents/MacOS/nomifun-desktop'), 0o644);
        if (defect === 'plist') rmSync(join(app, 'Contents/Info.plist'));
        if (defect === 'runtime') file(join(app, 'Contents/Frameworks/WebKit.framework/WebKit'), 'forbidden system framework');
        const archive = join(root, 'invalid.tar.gz');
        const packed = spawnSync('/usr/bin/tar', ['-czf', archive, '--no-xattrs', '-C', root, 'NomiFun.app'],
          { env: { ...process.env, COPYFILE_DISABLE: '1' } });
        expect(packed.status).toBe(0);
        await expect(verifyMacosUpdaterArchive(archive, app)).rejects.toThrow('incomplete macOS application bundle');
      } finally { rmSync(root, { recursive: true, force: true }); }
    },
  );

  test.each(['arm64', 'x86_64'])('archives current %s host and native resources, then produces a signature with an isolated temporary key', async architecture => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-app-updater-'));
    try {
      const app = appFixture(root, architecture);
      file(join(app, 'Contents/._Info.plist'), 'stale AppleDouble metadata');
      file(join(app, '__MACOSX/metadata'), 'stale archive metadata');
      file(join(app, 'Contents/Resources', 'long-pax-name-'.repeat(10), 'payload.txt'), 'long path preserved');
      symlinkSync('nomifun-desktop', join(app, 'Contents/MacOS/host-link'));
      if (process.platform === 'darwin') {
        for (const path of [app, join(app, 'Contents/MacOS/nomifun-desktop')]) {
          expect(spawnSync('/usr/bin/xattr', ['-w', 'com.nomifun.updater.fixture', 'metadata', path]).status).toBe(0);
        }
      }
      file(`${app}.tar.gz`, 'obsolete updater');
      const key = join(root, 'fixture.key');
      const generated = spawnSync(process.execPath, [resolve(REPO, 'node_modules/@tauri-apps/cli/tauri.js'), 'signer', 'generate', '--ci', '-w', key, '-p', ''], { encoding: 'utf8', timeout: 10_000 });
      expect(generated.status).toBe(0);
      const artifacts = await createMacosUpdaterArchive({ appPath: app, projectRoot: REPO,
        environment: { PATH: process.env.PATH, HOME: process.env.HOME, TMPDIR: process.env.TMPDIR, COPYFILE_DISABLE: '0', TAURI_SIGNING_PRIVATE_KEY_PATH: key, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: '' } });
      await verifyMacosUpdaterArchive(artifacts.archive, app);
      expect(readFileSync(artifacts.signature, 'utf8').length).toBeGreaterThan(40);
      const host = spawnSync('/usr/bin/tar', ['-xOf', artifacts.archive, 'NomiFun.app/Contents/MacOS/nomifun-desktop'], { encoding: 'utf8' });
      expect(host.status).toBe(0);
      expect(host.stdout).toBe('final host bytes');
      const listed = spawnSync('/usr/bin/tar', ['-tzf', artifacts.archive,
        ...(process.platform === 'darwin' ? ['--options=!mac-ext'] : [])], { encoding: 'utf8' });
      expect(listed.status).toBe(0);
      expect(listed.stdout).not.toContain('._');
      expect(listed.stdout).not.toContain('__MACOSX');
      const extracted = join(root, 'installed.app');
      mkdirSync(extracted);
      expect(spawnSync('/usr/bin/tar', ['-xzf', artifacts.archive, '-C', extracted, '--strip-components=1']).status).toBe(0);
      expect((await inspectMacosAppBundle(extracted)).status).toBe('pass');
      expect(statSync(join(extracted, 'Contents/MacOS/nomifun-desktop')).mode & 0o111).toBe(0o111);
      expect(readFileSync(join(extracted, 'Contents/MacOS/host-link'), 'utf8')).toBe('final host bytes');
      file(join(app, 'Contents/MacOS/nomifun-desktop'), 'different final host bytes', true);
      await expect(verifyMacosUpdaterArchive(artifacts.archive, app)).rejects.toThrow('contents differ');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  test('rejects an incomplete App before creating or signing an updater', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-app-incomplete-'));
    try {
      const app = appFixture(root);
      rmSync(join(app, 'Contents/Info.plist'));
      await expect(createMacosUpdaterArchive({ appPath: app, projectRoot: REPO, environment: {} })).rejects.toThrow('incomplete macOS application bundle');
      expect(existsSync(`${app}.tar.gz`)).toBe(false);
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  test('rejects AppleDouble and additional archive roots before extraction', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-app-archive-root-'));
    try {
      const app = appFixture(root);
      const archive = join(root, 'invalid.tar.gz');
      for (const extra of ['._NomiFun.app', 'other-root.txt', 'Other.app/Contents/Info.plist']) {
        file(join(root, extra), 'forbidden extra root');
        const packed = spawnSync('/usr/bin/tar', ['-czf', archive, '--format=pax', '--no-xattrs', '-C', root, 'NomiFun.app', extra],
          { encoding: 'utf8', env: { ...process.env, COPYFILE_DISABLE: '1' } });
        expect(packed.status).toBe(0);
        await expect(verifyMacosUpdaterArchive(archive, app)).rejects.toThrow('incompatible with Tauri macOS installation');
        rmSync(join(root, extra));
      }
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  test('rejects an archive with a native library that differs from the final Intel App', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-app-native-integrity-'));
    try {
      const app = appFixture(root, 'x86_64');
      const archive = join(root, 'invalid.tar.gz');
      const packed = spawnSync('/usr/bin/tar', ['-czf', archive, '--format=pax', '--no-xattrs', '-C', root, 'NomiFun.app'],
        { encoding: 'utf8', env: { ...process.env, COPYFILE_DISABLE: '1' } });
      expect(packed.status).toBe(0);
      file(join(app, 'Contents/Frameworks/libonnxruntime.dylib'), 'changed native bytes', true);
      await expect(verifyMacosUpdaterArchive(archive, app)).rejects.toThrow('contents differ');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});
