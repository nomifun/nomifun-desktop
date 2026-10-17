import { describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { buildMacosSystemExecution, systemExecutionServiceIdentifier } from './macos-system-execution.mjs';

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-system-package-'));
  const sources = join(root, 'apps/desktop/macos-system-execution');
  for (const directory of [sources, join(root, 'crates/shared/nomi-system-execution/src'), join(root, 'crates/shared/nomi-process-runtime/src'), join(root, 'crates/shared/nomi-native-files/src')]) await mkdir(directory, { recursive: true });
  for (const path of ['Cargo.toml','Cargo.lock','crates/shared/nomi-system-execution/Cargo.toml','crates/shared/nomi-process-runtime/Cargo.toml','crates/shared/nomi-native-files/Cargo.toml']) await writeFile(join(root,path),'# fixture manifest\n');
  for (const name of ['Protocol.swift', 'Helper.swift', 'Client.swift']) await writeFile(join(sources, name), `// ${name}\n`);
  await writeFile(join(sources, 'com.nomifun.desktop.system-execution.plist'), await readFile(new URL('../../apps/desktop/macos-system-execution/com.nomifun.desktop.system-execution.plist', import.meta.url)));
  const library = join(root, 'libnomi_system_execution.a');
  await writeFile(library, 'exact-cargo-library');
  return { root, sources, library };
}

describe('native macOS system component packaging', () => {
  test('production and development own separate operating system registrations', () => {
    expect(systemExecutionServiceIdentifier('com.nomifun.desktop')).toBe('com.nomifun.desktop.system-execution');
    expect(systemExecutionServiceIdentifier('com.nomifun.desktop.dev')).toBe('com.nomifun.desktop.dev.system-execution');
    expect(() => systemExecutionServiceIdentifier('unrelated.application')).toThrow('no supported application identity');
  });
  test.skipIf(process.platform !== 'darwin')('publishes the exact frozen archive, repairs a changed cache and preserves development service identity', async () => {
    const value = await fixture(); let links = 0;
    const execute = async (program, args) => {
      if (program === 'cargo') return `${JSON.stringify({ reason: 'compiler-artifact', target: { name: 'nomi_system_execution' }, filenames: [value.library] })}\n`;
      expect(program).toBe('xcrun');
      const archive = args.find(arg => arg.endsWith('.a'));
      expect(await readFile(archive, 'utf8')).toBe('exact-cargo-library');
      // A concurrent Cargo output replacement cannot affect the private link.
      await writeFile(value.library, 'later-cargo-library');
      await writeFile(args[args.indexOf('-o') + 1], 'native-executable-fixture'); links++;
      return '';
    };
    try {
      const request = { root: value.root, target: 'aarch64-apple-darwin', profile: 'debug', bundleIdentifier: 'com.nomifun.desktop.dev' };
      const files = await buildMacosSystemExecution(request, { execute });
      const executable = files['Library/HelperTools/nomifun-system-execution'];
      const plist = files['Library/LaunchDaemons/com.nomifun.desktop.dev.system-execution.plist'];
      expect(await readFile(plist, 'utf8')).toContain('<string>com.nomifun.desktop.dev.system-execution</string>');
      expect(await readdir(dirname(executable))).not.toContain('libnomi_system_execution.a');
      await writeFile(value.library, 'exact-cargo-library');
      expect(await buildMacosSystemExecution(request, { execute })).toEqual(files);
      expect(links).toBe(1);
      await writeFile(executable, 'damaged-cache');
      expect(await buildMacosSystemExecution(request, { execute })).toEqual(files);
      expect(links).toBe(2);
      expect(await readFile(executable, 'utf8')).toBe('native-executable-fixture');
    } finally { await rm(value.root, { recursive: true, force: true }); }
  });
  test.skipIf(process.platform !== 'darwin')('rejects changing sources instead of publishing a helper with a stale embedded identity', async () => {
    const value = await fixture(); let links = 0;
    try {
      await expect(buildMacosSystemExecution({ root: value.root, target: 'aarch64-apple-darwin' }, { execute: async (program) => {
        if (program !== 'cargo') { links++; return ''; }
        await writeFile(join(value.sources, 'Helper.swift'), '// newer helper\n');
        return `${JSON.stringify({ reason: 'compiler-artifact', target: { name: 'nomi_system_execution' }, filenames: [value.library] })}\n`;
      } })).rejects.toThrow('sources changed during compilation');
      expect(links).toBe(0);
    } finally { await rm(value.root, { recursive: true, force: true }); }
  });
  test.skipIf(process.platform !== 'darwin')('native reader sources and dependency manifests invalidate a component even when a fixture archive is unchanged', async () => {
    const value = await fixture(); let links = 0;
    const execute = async (program,args) => {
      if (program === 'cargo') return `${JSON.stringify({ reason:'compiler-artifact',target:{name:'nomi_system_execution'},filenames:[value.library] })}\n`;
      links++; await writeFile(args[args.indexOf('-o')+1], 'fixed-fixture-executable'); return '';
    };
    try {
      const request={root:value.root,target:'aarch64-apple-darwin'};
      const first=await buildMacosSystemExecution(request,{execute});
      await writeFile(join(value.root,'crates/shared/nomi-native-files/src/lib.rs'),'// new readonly core\n');
      const second=await buildMacosSystemExecution(request,{execute});
      expect(second['Library/HelperTools/nomifun-system-execution']).not.toBe(first['Library/HelperTools/nomifun-system-execution']);
      await writeFile(join(value.root,'Cargo.lock'),'# changed resolved dependency\n');
      const third=await buildMacosSystemExecution(request,{execute});
      expect(third['Library/HelperTools/nomifun-system-execution']).not.toBe(second['Library/HelperTools/nomifun-system-execution']);
      await writeFile(join(value.root,'crates/shared/nomi-native-files/Cargo.toml'),'# changed native reader manifest\n');
      const fourth=await buildMacosSystemExecution(request,{execute});
      expect(fourth['Library/HelperTools/nomifun-system-execution']).not.toBe(third['Library/HelperTools/nomifun-system-execution']);
      expect(links).toBe(4);
    } finally { await rm(value.root,{recursive:true,force:true}); }
  });
});
