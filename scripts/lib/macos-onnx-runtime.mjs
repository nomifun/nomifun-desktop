import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rename, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const INTEL_ONNX_RUNTIME = Object.freeze(JSON.parse(await readFile(
  join(root, 'apps/desktop/onnx-runtime-intel.json'), 'utf8',
)));

export function verifyOnnxArchive(bytes) {
  const digest = createHash('sha256').update(bytes).digest('hex');
  if (digest !== INTEL_ONNX_RUNTIME.archive_sha256) throw new Error('Intel ONNX Runtime archive SHA-256 mismatch');
}

const run = (command, args) => {
  const result = spawnSync(command, args, { encoding: 'utf8' });
  if (result.error || result.status !== 0) throw new Error(`${command} failed: ${result.stderr || result.error?.message}`);
  return result.stdout.trim();
};

export async function prepareIntelOnnxRuntime() {
  if (process.platform !== 'darwin') throw new Error('Intel ONNX packaging requires macOS');
  const cache = join(root, 'build.noindex/onnxruntime-macos-intel', INTEL_ONNX_RUNTIME.version);
  const archive = join(cache, INTEL_ONNX_RUNTIME.archive);
  await mkdir(cache, { recursive: true });
  let bytes = await readFile(archive).catch(error => {
    if (error.code === 'ENOENT') return null;
    throw error;
  });
  if (!bytes) {
    const response = await fetch(INTEL_ONNX_RUNTIME.url);
    if (!response.ok) throw new Error(`Intel ONNX Runtime download HTTP ${response.status}`);
    bytes = Buffer.from(await response.arrayBuffer());
    verifyOnnxArchive(bytes);
    await writeFile(archive, bytes);
  } else verifyOnnxArchive(bytes);

  const temporary = await mkdtemp(join(tmpdir(), 'nomifun-intel-onnx-'));
  try {
    run('tar', ['-xzf', archive, '-C', temporary]);
    const extracted = join(temporary, `onnxruntime-osx-universal2-${INTEL_ONNX_RUNTIME.version}`);
    const libraries = join(cache, 'lib');
    const library = join(libraries, INTEL_ONNX_RUNTIME.library);
    await mkdir(libraries, { recursive: true });
    const stagedLibrary = join(temporary, INTEL_ONNX_RUNTIME.library);
    run('lipo', ['-thin', 'x86_64', join(extracted, 'lib', INTEL_ONNX_RUNTIME.library), '-output', stagedLibrary]);
    if (run('lipo', ['-archs', stagedLibrary]) !== 'x86_64') throw new Error('Intel ONNX Runtime must contain only x86_64');
    await rename(stagedLibrary, library);
    const alias = join(libraries, 'libonnxruntime.dylib');
    await rm(alias, { force: true });
    await symlink(INTEL_ONNX_RUNTIME.library, alias);
    const files = { [`Frameworks/${INTEL_ONNX_RUNTIME.library}`]: library };
    for (const name of ['LICENSE', 'ThirdPartyNotices.txt']) {
      const output = join(cache, name);
      await writeFile(output, await readFile(join(extracted, name)));
      files[`Resources/onnxruntime-${name}`] = output;
    }
    await writeFile(join(cache, 'tauri.conf.json'), `${JSON.stringify({ bundle: { macOS: { files } } }, null, 2)}\n`);
    return libraries;
  } finally { await rm(temporary, { recursive: true, force: true }); }
}

if (import.meta.main) {
  try { process.stdout.write(`${await prepareIntelOnnxRuntime()}\n`); }
  catch (error) { process.stderr.write(`MACOS_INTEL_ONNX_FAIL ${error.message}\n`); process.exitCode = 1; }
}
