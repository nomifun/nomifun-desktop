import { describe, expect, test } from 'bun:test';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import {
  MACOS_BROWSER_RUNTIME, MACOS_CEF_HELPER_NAMES, compileBrowserEnvironment, createMacosUpdaterArchive,
  discoverCefRuntime, inspectMacosBrowserBundle, resolveMacosBuildSettings,
  verifyMacosUpdaterArchive,
} from './macos-browser-bundle.mjs';
import { desktopBuildCommand } from '../run-desktop-build.mjs';

const REPO = fileURLToPath(new URL('../../', import.meta.url));

function file(path, content, executable = false) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
  if (executable) chmodSync(path, 0o755);
}

function appFixture(root) {
  const app = join(root, 'NomiFun.app');
  file(join(app, 'Contents/MacOS/nomifun-desktop'), 'final host bytes', true);
  file(join(app, 'Contents/Info.plist'), '<plist/>');
  file(join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework'), 'framework bytes', true);
  file(join(app, 'Contents/Resources/browser-cef/runtime.json'), JSON.stringify(MACOS_BROWSER_RUNTIME));
  file(join(app, 'Contents/Resources/browser-cef/CREDITS.html'), 'runtime credits');
  for (const name of MACOS_BROWSER_RUNTIME.resources) file(join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework/Resources', name), `${name} bytes`);
  for (const name of MACOS_CEF_HELPER_NAMES) {
    file(join(app, `Contents/Frameworks/${name}.app/Contents/MacOS/${name}`), `${name} bytes`, true);
    file(join(app, `Contents/Frameworks/${name}.app/Contents/Info.plist`), '<plist/>');
  }
  return app;
}

describe('complete macOS Browser build routing', () => {
  test('generic builds use the macOS CEF packaging pipeline and preserve other platform builds', () => {
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

  test('Cargo runtime discovery keeps host and explicit target outputs separate and rejects altered archive identity', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-discovery-'));
    try {
      const host = join(root, 'build.noindex/debug/build/cef-dll-sys-host/out/cef_macos_aarch64');
      const explicit = join(root, 'build.noindex/aarch64-apple-darwin/release/build/cef-dll-sys-explicit/out/cef_macos_aarch64');
      for (const path of [host, explicit]) {
        file(join(path, 'archive.json'), JSON.stringify({ name: MACOS_BROWSER_RUNTIME.archive, sha1: MACOS_BROWSER_RUNTIME.archive_sha1 }));
        file(join(path, 'Chromium Embedded Framework.framework/Chromium Embedded Framework'), 'native');
      }
      expect(await discoverCefRuntime({ root })).toBe(host);
      const compiled = await compileBrowserEnvironment({ root, target: 'aarch64-apple-darwin', profile: 'debug', environment: { CEF_PATH: '/arbitrary', FLATPAK: '1', KEEP: 'normal' } });
      expect(compiled.runtimePath).toBe(host);
      expect(compiled.environment).toEqual({ CEF_PATH: host, KEEP: 'normal' });
      expect(await discoverCefRuntime({ root, target: 'aarch64-apple-darwin', profile: 'release' })).toBe(explicit);
      file(join(host, 'archive.json'), JSON.stringify({ name: MACOS_BROWSER_RUNTIME.archive, sha1: '0'.repeat(40) }));
      await expect(discoverCefRuntime({ root })).rejects.toThrow('pinned macOS');
      await expect(discoverCefRuntime({ root, target: 'x86_64-apple-darwin' })).rejects.toThrow('only macOS arm64');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  test('bundle inspection catches missing secondary helpers and changed pinned runtime metadata', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-bundle-'));
    try {
      const app = appFixture(root);
      expect((await inspectMacosBrowserBundle(app)).status).toBe('pass');
      file(join(app, 'Contents/Resources/browser-cef/runtime.json'), JSON.stringify({ ...MACOS_BROWSER_RUNTIME, chromium: 'wrong' }));
      expect((await inspectMacosBrowserBundle(app)).missing[0].label).toBe('pinned CEF runtime identity');
      file(join(app, 'Contents/Resources/browser-cef/runtime.json'), JSON.stringify(MACOS_BROWSER_RUNTIME));
      const resource = join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework/Resources/icudtl.dat');
      rmSync(resource);
      expect((await inspectMacosBrowserBundle(app)).missing.map(item => item.label)).toContain('icudtl.dat');
      file(resource, 'restored ICU data');
      rmSync(join(app, 'Contents/Frameworks/NomiFun Helper (GPU).app'), { recursive: true });
      expect((await inspectMacosBrowserBundle(app)).missing.map(item => item.label)).toContain('NomiFun Helper (GPU)');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});

describe.skipIf(process.platform === 'win32')('final App updater archives', () => {
  test.each(['._NomiFun.app', 'NomiFun.app/Contents/._Info.plist', '__MACOSX/metadata'])(
    'rejects metadata entry %s before approving an updater', async (entry) => {
      const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-updater-metadata-'));
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
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-updater-xattrs-'));
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

  test.each(['permissions', 'helper', 'identity'])(
    'rejects an extracted bundle with invalid %s', async (defect) => {
      const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-updater-invalid-'));
      try {
        const app = appFixture(root);
        if (defect === 'permissions') chmodSync(join(app, 'Contents/MacOS/nomifun-desktop'), 0o644);
        if (defect === 'helper') rmSync(join(app, 'Contents/Frameworks/NomiFun Helper (GPU).app'), { recursive: true });
        if (defect === 'identity') {
          file(join(app, 'Contents/Resources/browser-cef/runtime.json'), JSON.stringify({ ...MACOS_BROWSER_RUNTIME, architecture: 'wrong' }));
        }
        const archive = join(root, 'invalid.tar.gz');
        const packed = spawnSync('/usr/bin/tar', ['-czf', archive, '--no-xattrs', '-C', root, 'NomiFun.app'],
          { env: { ...process.env, COPYFILE_DISABLE: '1' } });
        expect(packed.status).toBe(0);
        await expect(verifyMacosUpdaterArchive(archive, app)).rejects.toThrow('incomplete macOS Browser bundle');
      } finally { rmSync(root, { recursive: true, force: true }); }
    },
  );

  test('archives all five helpers and current host bytes, then produces a signature with an isolated temporary key', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-updater-'));
    try {
      const app = appFixture(root);
      file(join(app, 'Contents/._Info.plist'), 'stale AppleDouble metadata');
      file(join(app, '__MACOSX/metadata'), 'stale archive metadata');
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
      expect((await inspectMacosBrowserBundle(extracted)).status).toBe('pass');
      expect(statSync(join(extracted, 'Contents/MacOS/nomifun-desktop')).mode & 0o111).toBe(0o111);
      expect(readFileSync(join(extracted, 'Contents/MacOS/host-link'), 'utf8')).toBe('final host bytes');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  test('rejects an incomplete App before creating or signing an updater', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-incomplete-'));
    try {
      const app = appFixture(root);
      rmSync(join(app, 'Contents/Frameworks/NomiFun Helper (Alerts).app'), { recursive: true });
      await expect(createMacosUpdaterArchive({ appPath: app, projectRoot: REPO, environment: {} })).rejects.toThrow('incomplete macOS Browser bundle');
      expect(existsSync(`${app}.tar.gz`)).toBe(false);
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});
