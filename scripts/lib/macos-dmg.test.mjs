import { describe, expect, test } from 'bun:test';
import { createHash } from 'node:crypto';
import { execFile } from 'node:child_process';
import { chmod, mkdir, mkdtemp, readFile, readdir, readlink, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { promisify } from 'node:util';
import { createMacosDmg, resolveMacosDmgFormat } from './macos-dmg.mjs';

const command = promisify(execFile);

describe('macOS DMG compression selection', () => {
  test('defaults to LZMA and accepts only the explicit zlib compatibility override', () => {
    expect(resolveMacosDmgFormat({})).toBe('ULMO');
    expect(resolveMacosDmgFormat({ NOMIFUN_MACOS_DMG_FORMAT: 'ULMO' })).toBe('ULMO');
    expect(resolveMacosDmgFormat({ NOMIFUN_MACOS_DMG_FORMAT: 'UDZO' })).toBe('UDZO');
    for (const value of ['', 'ULFO', 'udzo', 'ULMO -imagekey zlib-level=1']) {
      expect(() => resolveMacosDmgFormat({ NOMIFUN_MACOS_DMG_FORMAT: value })).toThrow('must be ULMO');
    }
  });

  test('rejects unsupported compression before touching source or output paths', async () => {
    await expect(createMacosDmg({ appPath: '/nonexistent/NomiFun.app', outputPath: '/nonexistent/NomiFun.dmg',
      environment: { NOMIFUN_MACOS_DMG_FORMAT: 'ULFO' } })).rejects.toThrow('must be ULMO');
  });
});

describe.skipIf(process.platform !== 'darwin')('real macOS installation containers', () => {
  test('refuses to create an image inside the signed source App', async () => {
    const root = await mkdtemp(join(tmpdir(), 'nomifun-dmg-source-'));
    const app = join(root, 'NomiFun.app');
    try {
      await mkdir(app);
      await expect(createMacosDmg({ appPath: app, outputPath: join(app, 'NomiFun.dmg'), environment: {} }))
        .rejects.toThrow('outside the signed App');
      expect(await readdir(app)).toEqual([]);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  for (const format of ['ULMO', 'UDZO']) {
    test(`${format} verifies and mounts with intact App files, permissions and Applications link`, async () => {
      const root = await mkdtemp(join(tmpdir(), 'nomifun-dmg-fixture-'));
      const app = join(root, 'NomiFun.app');
      const output = join(root, 'output', 'NomiFun.dmg');
      const mount = join(root, 'mounted');
      const fixtures = new Map([
        ['Contents/MacOS/nomifun-desktop', Buffer.from('fixture executable\n')],
        ['Contents/Info.plist', Buffer.from('<plist><dict><key>CFBundleName</key><string>NomiFun</string></dict></plist>')],
        ['Contents/Resources/中文.bin', Buffer.alloc(1024 * 1024, 0xa7)],
        ['Contents/_CodeSignature/CodeResources', Buffer.from('fixture signed resource bytes')],
      ]);
      let attached = false;
      try {
        for (const [path, bytes] of fixtures) {
          await mkdir(dirname(join(app, path)), { recursive: true });
          await writeFile(join(app, path), bytes);
        }
        await chmod(join(app, 'Contents/MacOS/nomifun-desktop'), 0o755);
        await symlink('中文.bin', join(app, 'Contents/Resources/asset-link'));
        const originalModes = new Map(await Promise.all([...fixtures.keys()].map(async path =>
          [path, (await stat(join(app, path))).mode & 0o777])));
        const environment = { ...process.env };
        // Exercise the actual default for ULMO, not an equivalent explicit value.
        if (format === 'ULMO') delete environment.NOMIFUN_MACOS_DMG_FORMAT;
        else environment.NOMIFUN_MACOS_DMG_FORMAT = format;
        const result = await createMacosDmg({ appPath: app, outputPath: output, environment });
        expect(result).toEqual({ output, format });
        const info = await command('/usr/bin/hdiutil', ['imageinfo', '-plist', output]);
        expect(info.stdout).toMatch(new RegExp(`<key>Format</key>\\s*<string>${format}</string>`));
        await command('/usr/bin/hdiutil', ['verify', '-quiet', output]);
        await mkdir(mount);
        await command('/usr/bin/hdiutil', ['attach', '-readonly', '-nobrowse', '-quiet', '-mountpoint', mount, output]);
        attached = true;
        const hash = bytes => createHash('sha256').update(bytes).digest('hex');
        for (const [path, bytes] of fixtures) {
          expect(hash(await readFile(join(mount, 'NomiFun.app', path)))).toBe(hash(bytes));
          expect(hash(await readFile(join(app, path)))).toBe(hash(bytes));
          expect((await stat(join(mount, 'NomiFun.app', path))).mode & 0o777).toBe(originalModes.get(path));
        }
        expect(await readlink(join(mount, 'Applications'))).toBe('/Applications');
        expect(await readlink(join(mount, 'NomiFun.app/Contents/Resources/asset-link'))).toBe('中文.bin');
        expect(await readdir(dirname(output))).toEqual(['NomiFun.dmg']);
      } finally {
        if (attached) await command('/usr/bin/hdiutil', ['detach', '-quiet', mount]);
        await rm(root, { recursive: true, force: true });
      }
    }, 60_000);
  }
});
