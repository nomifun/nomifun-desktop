import { describe, expect, test } from 'bun:test';
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import {
  MACOS_BROWSER_RUNTIME, MACOS_INTEL_BROWSER_RUNTIME, MACOS_CEF_HELPER_NAMES, MACOS_CEF_LOCALE_DIRECTORIES, compileBrowserEnvironment, createMacosUpdaterArchive,
  discoverCefRuntime, inspectMacosBrowserBundle, resolveMacosBuildSettings,
  pruneMacosCefLocales, verifyMacosUpdaterArchive, macosBrowserRuntime, macosCefLocaleDirectories,
} from './macos-browser-bundle.mjs';
import { desktopBuildCommand } from '../run-desktop-build.mjs';

const REPO = fileURLToPath(new URL('../../', import.meta.url));

function file(path, content, executable = false) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
  if (executable) chmodSync(path, 0o755);
}

function appFixture(root, contract = MACOS_BROWSER_RUNTIME) {
  const app = join(root, 'NomiFun.app');
  file(join(app, 'Contents/MacOS/nomifun-desktop'), 'final host bytes', true);
  file(join(app, 'Contents/Info.plist'), '<plist/>');
  file(join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework'), 'framework bytes', true);
  file(join(app, 'Contents/Resources/browser-cef/runtime.json'), JSON.stringify(contract));
  file(join(app, 'Contents/Resources/browser-cef/CREDITS.html'), 'runtime credits');
  for (const name of contract.resources) file(join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework/Resources', name), `${name} bytes`);
  for (const name of macosCefLocaleDirectories(contract)) file(join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework/Resources', name, 'locale.pak'), `${name} bytes`);
  for (const name of contract.helpers) {
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
      const hostContract = process.arch === 'arm64' ? MACOS_BROWSER_RUNTIME : MACOS_INTEL_BROWSER_RUNTIME;
      const hostTarget = process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
      const hostDirectory = process.arch === 'arm64' ? 'cef_macos_aarch64' : 'cef_macos_x86_64';
      const host = join(root, 'build.noindex/debug/build/cef-dll-sys-host/out', hostDirectory);
      const explicit = join(root, 'build.noindex/aarch64-apple-darwin/release/build/cef-dll-sys-explicit/out/cef_macos_aarch64');
      for (const [path, contract] of [[host, hostContract], [explicit, MACOS_BROWSER_RUNTIME]]) {
        file(join(path, 'archive.json'), JSON.stringify({ name: contract.archive, sha1: contract.archive_sha1 }));
        file(join(path, 'Chromium Embedded Framework.framework/Chromium Embedded Framework'), 'native');
      }
      expect(await discoverCefRuntime({ root })).toBe(host);
      const compiled = await compileBrowserEnvironment({ root, target: hostTarget, profile: 'debug', environment: { CEF_PATH: '/arbitrary', FLATPAK: '1', KEEP: 'normal' } });
      expect(compiled.runtimePath).toBe(host);
      expect(compiled.environment).toEqual({ CEF_PATH: host, KEEP: 'normal' });
      expect(await discoverCefRuntime({ root, target: 'aarch64-apple-darwin', profile: 'release' })).toBe(explicit);
      file(join(host, 'archive.json'), JSON.stringify({ name: MACOS_BROWSER_RUNTIME.archive, sha1: '0'.repeat(40) }));
      await expect(discoverCefRuntime({ root })).rejects.toThrow('pinned macOS');
      await expect(discoverCefRuntime({ root, target: 'x86_64-apple-darwin' })).rejects.toThrow('pinned macOS');
      const missingIntel = await compileBrowserEnvironment({ root, target: 'x86_64-apple-darwin' });
      expect(missingIntel.runtimePath).toBeNull();
      const intel = join(root, 'build.noindex/x86_64-apple-darwin/release/build/cef-dll-sys-intel/out/cef_macos_x86_64');
      file(join(intel, 'archive.json'), JSON.stringify({ name: MACOS_INTEL_BROWSER_RUNTIME.archive, sha1: MACOS_INTEL_BROWSER_RUNTIME.archive_sha1 }));
      file(join(intel, 'Chromium Embedded Framework.framework/Chromium Embedded Framework'), 'intel native');
      expect(await discoverCefRuntime({ root, target: 'x86_64-apple-darwin', profile: 'release' })).toBe(intel);
      expect((await compileBrowserEnvironment({ root, target: 'x86_64-apple-darwin' })).runtimePath).toBe(intel);
      file(join(intel, 'archive.json'), JSON.stringify({ name: MACOS_BROWSER_RUNTIME.archive, sha1: MACOS_BROWSER_RUNTIME.archive_sha1 }));
      await expect(discoverCefRuntime({ root, target: 'x86_64-apple-darwin', profile: 'release' })).rejects.toThrow('pinned macOS');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  test('pins independent runtime identities and rejects mixed Intel metadata', async () => {
    expect(macosBrowserRuntime('aarch64-apple-darwin')).toBe(MACOS_BROWSER_RUNTIME);
    expect(macosBrowserRuntime('x86_64-apple-darwin')).toBe(MACOS_INTEL_BROWSER_RUNTIME);
    expect(() => macosBrowserRuntime('universal-apple-darwin')).toThrow('unsupported');
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-intel-bundle-'));
    try {
      const app = appFixture(root, MACOS_INTEL_BROWSER_RUNTIME);
      expect((await inspectMacosBrowserBundle(app)).status).toBe('pass');
      file(join(app, 'Contents/Resources/browser-cef/runtime.json'), JSON.stringify({ ...MACOS_INTEL_BROWSER_RUNTIME, archive_sha1: MACOS_BROWSER_RUNTIME.archive_sha1 }));
      expect((await inspectMacosBrowserBundle(app)).missing[0].label).toBe('pinned CEF runtime identity');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  test.skipIf(process.platform === 'win32')('POSIX bundle inspection catches missing secondary helpers and changed pinned runtime metadata', async () => {
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

  test.skipIf(process.platform !== 'win32')('bundle inspection fails closed when Windows cannot provide POSIX executable bits', async () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-bundle-'));
    try {
      const inspection = await inspectMacosBrowserBundle(appFixture(root));
      expect(inspection.status).toBe('fail');
      const missing = inspection.missing.map(item => item.label);
      expect(missing).toContain('host');
      expect(missing).toContain('CEF framework');
      for (const name of MACOS_CEF_HELPER_NAMES) expect(missing).toContain(name);
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  test.skipIf(process.platform === 'win32').each(['aarch64-apple-darwin', 'x86_64-apple-darwin'])('bundle inspection requires the complete %s locale policy and rejects surplus languages', async target => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-locales-'));
    try {
      const contract = macosBrowserRuntime(target);
      const app = appFixture(root, contract);
      const framework = join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework');
      const locale = join(framework, 'Resources/en_NEUTER.lproj/locale.pak');
      rmSync(locale);
      expect((await inspectMacosBrowserBundle(app)).missing.map(item => item.label)).toContain('en_NEUTER.lproj/locale.pak');
      file(locale, 'restored locale');
      file(join(framework, 'Resources/ja.lproj/locale.pak'), 'unneeded Japanese pack');
      expect((await inspectMacosBrowserBundle(app)).missing.map(item => item.label)).toContain('unexpected CEF locale resources');
      rmSync(join(framework, 'Resources/ja.lproj'), { recursive: true });
      file(join(app, 'Contents/Resources/browser-cef/runtime.json'), JSON.stringify({ ...contract, locales: ['en'] }));
      expect((await inspectMacosBrowserBundle(app)).missing.map(item => item.label)).toContain('CEF locale distribution policy');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
});

describe('copied CEF locale resources', () => {
  test.each(['aarch64-apple-darwin', 'x86_64-apple-darwin'])('prunes only the %s App copy while preserving every retained variant and all non-locale files', async target => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-prune-'));
    try {
      const contract = macosBrowserRuntime(target);
      const localeDirectories = macosCefLocaleDirectories(contract);
      const app = appFixture(root, contract);
      const framework = join(app, 'Contents/Frameworks/Chromium Embedded Framework.framework');
      file(join(framework, 'Resources/ja.lproj/locale.pak'), 'Japanese pack');
      file(join(framework, 'Resources/fr_NEUTER.lproj/locale.pak'), 'French pack');
      const original = join(root, 'cargo-runtime/framework');
      cpSync(framework, original, { recursive: true, dereference: false });
      const result = await pruneMacosCefLocales({ appPath: app, frameworkPath: framework, target });
      expect(result).toEqual({ retained: [...localeDirectories], removed: 2, removedBytes: 24 });
      expect(readdirSync(join(framework, 'Resources')).filter(name => name.endsWith('.lproj')).sort()).toEqual([...localeDirectories].sort());
      for (const name of localeDirectories) {
        expect(readFileSync(join(framework, 'Resources', name, 'locale.pak'))).toEqual(readFileSync(join(original, 'Resources', name, 'locale.pak')));
      }
      for (const name of contract.resources) {
        expect(readFileSync(join(framework, 'Resources', name))).toEqual(readFileSync(join(original, 'Resources', name)));
      }
      expect(readFileSync(join(original, 'Resources/ja.lproj/locale.pak'), 'utf8')).toBe('Japanese pack');
      expect(readFileSync(join(original, 'Resources/fr_NEUTER.lproj/locale.pak'), 'utf8')).toBe('French pack');
      await expect(pruneMacosCefLocales({ appPath: app, frameworkPath: original, target })).rejects.toThrow('staged application framework');
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

  test.each(['arm64', 'x86_64'])('archives all five %s helpers and current host bytes, then produces a signature with an isolated temporary key', async architecture => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-cef-updater-'));
    try {
      const app = appFixture(root, architecture === 'arm64' ? MACOS_BROWSER_RUNTIME : MACOS_INTEL_BROWSER_RUNTIME);
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
      expect((await inspectMacosBrowserBundle(extracted)).status).toBe('pass');
      expect(statSync(join(extracted, 'Contents/MacOS/nomifun-desktop')).mode & 0o111).toBe(0o111);
      expect(readFileSync(join(extracted, 'Contents/MacOS/host-link'), 'utf8')).toBe('final host bytes');
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
