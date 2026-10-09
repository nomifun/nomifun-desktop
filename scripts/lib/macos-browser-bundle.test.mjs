import { describe, expect, test } from 'bun:test';
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import {
  MACOS_BROWSER_RUNTIME, MACOS_CEF_HELPER_NAMES, MACOS_CEF_LOCALE_DIRECTORIES, compileBrowserEnvironment, createMacosUpdaterArchive,
  discoverCefRuntime, inspectMacosBrowserBundle, resolveMacosBuildSettings,
  pruneMacosCefLocales, verifyMacosUpdaterArchive,
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
  for (const name of MACOS_CEF_LOCALE_DIRECTORIES) file(join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework/Resources', name, 'locale.pak'), `${name} bytes`);
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

  test('bundle inspection requires the complete distributed locale policy and rejects surplus languages', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-locales-'));
    try {
      const app = appFixture(root);
      const framework = join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework');
      const locale = join(framework, 'Resources/en_NEUTER.lproj/locale.pak');
      rmSync(locale);
      expect((await inspectMacosBrowserBundle(app)).missing.map(item => item.label)).toContain('en_NEUTER.lproj/locale.pak');
      file(locale, 'restored locale');
      file(join(framework, 'Resources/ja.lproj/locale.pak'), 'unneeded Japanese pack');
      expect((await inspectMacosBrowserBundle(app)).missing.map(item => item.label)).toContain('unexpected CEF locale resources');
      rmSync(join(framework, 'Resources/ja.lproj'), { recursive: true });
      file(join(app, 'Contents/Resources/browser-cef/runtime.json'), JSON.stringify({ ...MACOS_BROWSER_RUNTIME, locales: ['en'] }));
      expect((await inspectMacosBrowserBundle(app)).missing.map(item => item.label)).toContain('CEF locale distribution policy');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});

describe('copied CEF locale resources', () => {
  test('prunes only the App copy while preserving every retained variant and all non-locale files', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-prune-'));
    try {
      const app = appFixture(root);
      const framework = join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework');
      file(join(framework, 'Resources/ja.lproj/locale.pak'), 'Japanese pack');
      file(join(framework, 'Resources/fr_NEUTER.lproj/locale.pak'), 'French pack');
      const original = join(root, 'cargo-runtime/framework');
      cpSync(framework, original, { recursive: true, dereference: false });
      const result = await pruneMacosCefLocales({ appPath: app, frameworkPath: framework });
      expect(result).toEqual({ retained: [...MACOS_CEF_LOCALE_DIRECTORIES], removed: 2, removedBytes: 24 });
      expect(readdirSync(join(framework, 'Resources')).filter(name => name.endsWith('.lproj')).sort()).toEqual([...MACOS_CEF_LOCALE_DIRECTORIES].sort());
      for (const name of MACOS_CEF_LOCALE_DIRECTORIES) {
        expect(readFileSync(join(framework, 'Resources', name, 'locale.pak'))).toEqual(readFileSync(join(original, 'Resources', name, 'locale.pak')));
      }
      for (const name of MACOS_BROWSER_RUNTIME.resources) {
        expect(readFileSync(join(framework, 'Resources', name))).toEqual(readFileSync(join(original, 'Resources', name)));
      }
      expect(readFileSync(join(original, 'Resources/ja.lproj/locale.pak'), 'utf8')).toBe('Japanese pack');
      expect(readFileSync(join(original, 'Resources/fr_NEUTER.lproj/locale.pak'), 'utf8')).toBe('French pack');
      await expect(pruneMacosCefLocales({ appPath: app, frameworkPath: original })).rejects.toThrow('staged application framework');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  test('validates all locales before deleting anything and rejects locale symlinks', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-prune-fail-'));
    try {
      const app = appFixture(root);
      const framework = join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework');
      const resources = join(framework, 'Resources');
      file(join(resources, 'ja.lproj/locale.pak'), 'Japanese pack');
      rmSync(join(resources, 'en_NEUTER.lproj'), { recursive: true });
      await expect(pruneMacosCefLocales({ appPath: app, frameworkPath: framework })).rejects.toThrow('required CEF locale is missing');
      expect(existsSync(join(resources, 'ja.lproj/locale.pak'))).toBe(true);
      file(join(resources, 'en_NEUTER.lproj/locale.pak'), 'required pack');
      symlinkSync(join(resources, 'en.lproj'), join(resources, 'fr.lproj'));
      await expect(pruneMacosCefLocales({ appPath: app, frameworkPath: framework })).rejects.toThrow('invalid CEF locale directory');
      expect(existsSync(join(resources, 'ja.lproj/locale.pak'))).toBe(true);
      rmSync(join(resources, 'fr.lproj'));
      rmSync(join(resources, 'en_NEUTER.lproj/locale.pak'));
      file(join(root, 'external.pak'), 'external locale');
      symlinkSync(join(root, 'external.pak'), join(resources, 'en_NEUTER.lproj/locale.pak'));
      await expect(pruneMacosCefLocales({ appPath: app, frameworkPath: framework })).rejects.toThrow('nonempty regular file');
      expect(readFileSync(join(root, 'external.pak'), 'utf8')).toBe('external locale');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});

describe.skipIf(process.platform === 'win32')('final App updater archives', () => {
  test('archives all five helpers and current host bytes, then produces a signature with an isolated temporary key', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-updater-'));
    try {
      const app = appFixture(root);
      file(join(app, 'Contents/Resources', 'long-pax-name-'.repeat(10), 'payload.txt'), 'long path preserved');
      if (process.platform === 'darwin') {
        expect(spawnSync('/usr/bin/xattr', ['-w', 'com.nomifun.updater-fixture', 'fixture metadata', join(app, 'Contents/MacOS/nomifun-desktop')]).status).toBe(0);
      }
      file(`${app}.tar.gz`, 'obsolete updater');
      const key = join(root, 'fixture.key');
      const generated = spawnSync(process.execPath, [resolve(REPO, 'node_modules/@tauri-apps/cli/tauri.js'), 'signer', 'generate', '--ci', '-w', key, '-p', ''], { encoding: 'utf8', timeout: 10_000 });
      expect(generated.status).toBe(0);
      const artifacts = await createMacosUpdaterArchive({ appPath: app, projectRoot: REPO,
        environment: { PATH: process.env.PATH, HOME: process.env.HOME, TMPDIR: process.env.TMPDIR, TAURI_SIGNING_PRIVATE_KEY_PATH: key, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: '' } });
      await verifyMacosUpdaterArchive(artifacts.archive, app);
      expect(readFileSync(artifacts.signature, 'utf8').length).toBeGreaterThan(40);
      const host = spawnSync('/usr/bin/tar', ['-xOf', artifacts.archive, 'NomiFun.app/Contents/MacOS/nomifun-desktop'], { encoding: 'utf8' });
      expect(host.status).toBe(0);
      expect(host.stdout).toBe('final host bytes');
      const rawList = spawnSync('/usr/bin/tar', ['-tzf', artifacts.archive, ...(process.platform === 'darwin' ? ['--options=!mac-ext'] : [])], { encoding: 'utf8' });
      expect(rawList.status).toBe(0);
      expect(rawList.stdout.includes('._')).toBe(false);
      expect(rawList.stdout.includes('__MACOSX')).toBe(false);
      file(join(app, 'Contents/MacOS/nomifun-desktop'), 'different final host bytes', true);
      await expect(verifyMacosUpdaterArchive(artifacts.archive, app)).rejects.toThrow('contents differ');
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

  test('rejects AppleDouble and additional archive roots before extraction', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-archive-root-'));
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

  test('rejects updater contents with missing locale resources or a changed locale policy', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-archive-locales-'));
    try {
      const app = appFixture(root);
      const archive = join(root, 'invalid.tar.gz');
      const locale = join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework/Resources/en.lproj/locale.pak');
      rmSync(locale);
      const pack = () => {
        const packed = spawnSync('/usr/bin/tar', ['-czf', archive, '--format=pax', '--no-xattrs', '-C', root, 'NomiFun.app'],
          { encoding: 'utf8', env: { ...process.env, COPYFILE_DISABLE: '1' } });
        expect(packed.status).toBe(0);
      };
      pack();
      await expect(verifyMacosUpdaterArchive(archive, app)).rejects.toThrow('en.lproj/locale.pak');
      file(locale, 'locale restored');
      file(join(app, 'Contents/Resources/browser-cef/runtime.json'), JSON.stringify({ ...MACOS_BROWSER_RUNTIME, locale_variants: [''] }));
      pack();
      await expect(verifyMacosUpdaterArchive(archive, app)).rejects.toThrow('CEF locale distribution policy');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});
