// Build the installation container from the final, signed and stapled App.
// Compression only changes the DMG; never mutate the source App's signed bytes.
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, rename, rm, stat, symlink } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function resolveMacosDmgFormat(environment = process.env) {
  const format = environment.NOMIFUN_MACOS_DMG_FORMAT ?? 'ULMO';
  if (!['ULMO', 'UDZO'].includes(format)) {
    throw new Error('NOMIFUN_MACOS_DMG_FORMAT must be ULMO (LZMA) or UDZO (zlib level 9)');
  }
  return format;
}

const run = (command, args, environment) => new Promise((accept, reject) => {
  const child = spawn(command, args, {
    stdio: ['ignore', 'ignore', 'pipe'],
    env: Object.fromEntries(['PATH', 'HOME', 'TMPDIR', 'LANG', 'DEVELOPER_DIR', 'COPYFILE_DISABLE']
      .filter(key => environment[key] !== undefined).map(key => [key, environment[key]])),
  });
  let diagnostics = '';
  child.stderr.on('data', chunk => { diagnostics += chunk; });
  child.once('error', reject);
  child.once('close', code => code === 0 ? accept()
    : reject(new Error(`${command} exited ${code}: ${diagnostics.trim()}`)));
});

export async function createMacosDmg({ appPath, outputPath, environment = process.env }) {
  const format = resolveMacosDmgFormat(environment);
  if (process.platform !== 'darwin') throw new Error('macOS DMG creation requires a macOS host');
  const app = resolve(appPath);
  const output = resolve(outputPath);
  if (!(await stat(app)).isDirectory() || !app.endsWith('.app')) {
    throw new Error('macOS DMG creation requires an existing .app directory');
  }
  if (!output.endsWith('.dmg')) throw new Error('macOS DMG output must have the .dmg extension');
  if (output.startsWith(`${app}/`)) throw new Error('macOS DMG output must be outside the signed App');
  await mkdir(dirname(output), { recursive: true });
  // Keep temporary and final images on the same volume for atomic publication.
  const temporary = await mkdtemp(join(dirname(output), '.nomifun-dmg-'));
  const staging = join(temporary, 'root');
  const image = join(temporary, 'NomiFun.dmg');
  try {
    await mkdir(staging);
    await run('/usr/bin/ditto', ['--noqtn', app, join(staging, basename(app))], environment);
    await symlink('/Applications', join(staging, 'Applications'));
    await run('/usr/bin/hdiutil', [
      'create', '-quiet', '-ov', '-fs', 'HFS+', '-format', format,
      ...(format === 'UDZO' ? ['-imagekey', 'zlib-level=9'] : []),
      '-volname', 'NomiFun', '-srcfolder', staging, image,
    ], environment);
    await run('/usr/bin/hdiutil', ['verify', '-quiet', image], environment);
    await rename(image, output);
    return { output, format };
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const option = key => {
    const index = args.indexOf(key);
    if (index < 0 || !args[index + 1] || args[index + 1].startsWith('--')) {
      throw new Error(`${key} requires a value`);
    }
    return args[index + 1];
  };
  try {
    if (args[0] === 'format') process.stdout.write(`${resolveMacosDmgFormat()}\n`);
    else if (args[0] === 'create') {
      process.stdout.write(`${JSON.stringify(await createMacosDmg({ appPath: option('--app'), outputPath: option('--output') }))}\n`);
    } else throw new Error('expected format or create');
  } catch (error) {
    process.stderr.write(`MACOS_DMG_FAIL ${error.message}\n`);
    process.exitCode = 1;
  }
}
