import { describe, expect, test } from 'bun:test';
import { copyFile, lstat, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { linuxSystemExecutionBuildPlan, prepareLinuxSystemExecution } from './linux-system-execution.mjs';
import { fastBuildInvocation } from '../run-fast-build.mjs';

async function fixture(machine = 62) {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-linux-system-execution-'));
  const assets = join(root, 'apps/desktop/linux-system-execution');
  await mkdir(assets, { recursive: true });
  for (const name of ['com.nomifun.system-execution.policy', 'install.sh', 'README.md']) {
    await copyFile(new URL(`../../apps/desktop/linux-system-execution/${name}`, import.meta.url), join(assets, name));
  }
  const executable = join(root, 'compiled-runner');
  const bytes = Buffer.alloc(64);
  bytes.set([0x7f, 0x45, 0x4c, 0x46, 2, 1]); bytes.writeUInt16LE(machine, 18);
  await writeFile(executable, bytes);
  const execute = async (program, args) => {
    expect(program).toBe('cargo');
    expect(args).toContain('--locked');
    return `${JSON.stringify({ reason: 'compiler-artifact', target: { name: 'nomifun-system-runner', kind: ['bin'] }, executable })}\n`;
  };
  return { root, executable, bytes, execute };
}

describe('Linux system execution packaging', () => {
  test('binds one native action to the installed stdio runner and retains only native temporary administrator authorization', async () => {
    const policy = await readFile(new URL('../../apps/desktop/linux-system-execution/com.nomifun.system-execution.policy', import.meta.url), 'utf8');
    expect((policy.match(/<action\b/g) ?? []).length).toBe(1);
    expect(policy.match(/<action\s+id="([^"]+)"/)[1]).toBe('com.nomifun.system-execution.run');
    const defaults = Object.fromEntries([...policy.matchAll(/<allow_(any|inactive|active)>([^<]+)<\/allow_\1>/g)]
      .map(([, subject, authorization]) => [subject, authorization]));
    expect(defaults).toEqual({ any: 'auth_admin_keep', inactive: 'auth_admin_keep', active: 'auth_admin_keep' });
    const annotations = Object.fromEntries([...policy.matchAll(/<annotate\s+key="([^"]+)">([^<]+)<\/annotate>/g)]
      .map(([, key, value]) => [key, value]));
    expect(annotations).toEqual({
      'org.freedesktop.policykit.exec.path': '/usr/libexec/nomifun-system-runner',
      'org.freedesktop.policykit.exec.argv1': '--stdio',
    });
  });

  test('rejects unsupported targets and profiles before compiling or creating outputs', () => {
    expect(() => linuxSystemExecutionBuildPlan({ target: 'x86_64-apple-darwin' })).toThrow('unsupported Linux');
    expect(() => linuxSystemExecutionBuildPlan({ target: 'x86_64-unknown-linux-gnu', profile: '../release' })).toThrow('profile');
  });

  test('fast builds cannot emit packages without the formal system component overlay or mix profiles', () => {
    for (const args of [['--bundles', 'deb'], ['--bundles=rpm'], ['-bdeb']]) {
      expect(() => fastBuildInvocation(args, 'linux', 'x64')).toThrow('build:linux');
    }
    expect(() => fastBuildInvocation(['--profile', 'release'], 'linux', 'x64')).toThrow('debug profile');
    expect(fastBuildInvocation([], 'linux', 'x64').args).toContain('--no-bundle');
  });

  test.skipIf(process.platform !== 'linux')('freezes the exact Cargo ELF and stages fixed native paths plus a complete AppImage installer', async () => {
    const item = await fixture();
    try {
      const prepared = await prepareLinuxSystemExecution({ root: item.root, target: 'x86_64-unknown-linux-gnu' }, { execute: item.execute });
      await writeFile(item.executable, 'another build');
      expect(await readFile(join(prepared.directory, 'nomifun-system-runner'))).toEqual(item.bytes);
      expect((await lstat(join(prepared.directory, 'nomifun-system-runner'))).mode & 0o7777).toBe(0o755);
      const config = JSON.parse(await readFile(prepared.config, 'utf8')).bundle.linux;
      for (const format of ['deb', 'rpm']) {
        expect(Object.keys(config[format].files).sort()).toEqual([
          '/usr/libexec/nomifun-system-runner', '/usr/share/polkit-1/actions/com.nomifun.system-execution.policy',
        ]);
      }
      for (const name of ['nomifun-system-runner', 'install.sh', 'com.nomifun.system-execution.policy', 'README.md']) {
        const source = config.appimage.files[`/usr/lib/nomifun-system-execution/${name}`];
        expect((await lstat(source)).isFile()).toBe(true);
      }
      expect(config.deb.depends).toContain('pkexec | policykit-1');
      expect(config.rpm.depends).toContain('polkit');
      for (const dependency of ['xdg-desktop-portal', 'libasound2t64 | libasound2', 'xwayland']) {
        expect(config.deb.depends).toContain(dependency);
      }
      for (const dependency of ['xdg-desktop-portal', 'alsa-lib', 'xorg-x11-server-Xwayland']) {
        expect(config.rpm.depends).toContain(dependency);
      }
    } finally { await rm(item.root, { recursive: true, force: true }); }
  });

  test.skipIf(process.platform !== 'linux')('refuses the wrong architecture instead of packaging a stale runner', async () => {
    const item = await fixture(183);
    try {
      await expect(prepareLinuxSystemExecution({ root: item.root, target: 'x86_64-unknown-linux-gnu' }, { execute: item.execute })).rejects.toThrow('ELF');
    } finally { await rm(item.root, { recursive: true, force: true }); }
  });

  test.skipIf(process.platform !== 'linux')('refuses a Cargo artifact symlink', async () => {
    const item = await fixture();
    try {
      await symlink(item.executable, join(item.root, 'runner-link'));
      const execute = async () => JSON.stringify({ reason: 'compiler-artifact', target: { name: 'nomifun-system-runner', kind: ['bin'] }, executable: join(item.root, 'runner-link') });
      await expect(prepareLinuxSystemExecution({ root: item.root, target: 'x86_64-unknown-linux-gnu' }, { execute })).rejects.toThrow('exact Linux');
    } finally { await rm(item.root, { recursive: true, force: true }); }
  });
});
