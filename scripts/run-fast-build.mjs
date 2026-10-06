#!/usr/bin/env bun
// Fast macOS builds still need the same complete Browser runtime as releases.
// They stop at a signed development .app; other platforms keep no-bundle.
import { readdir, readFile, mkdir, mkdtemp, cp, writeFile } from 'node:fs/promises';
import { constants, createReadStream } from 'node:fs';
import { createHash } from 'node:crypto';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { developmentEnvironment } from './run-dev.mjs';
import { runCargoArtifact, runCommand, macosBrowserCompilerEnvironment, nativeBrowserLaunchEnvironment } from './run-macos-dev-runner.mjs';
import { stageMacosBrowserBundle } from './lib/macos-browser-bundle.mjs';
import { macosDevelopmentSigningIdentity, macosDevelopmentSigningNotice } from './lib/macos-dev-signing.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export function fastBuildInvocation(args, platform = process.platform, architecture = process.arch) {
  const value = name => {
    const inline = args.find(arg => arg.startsWith(`${name}=`));
    if (inline) return inline.slice(name.length + 1);
    const index = args.indexOf(name);
    return index < 0 ? null : args[index + 1];
  };
  const target = value('--target') ?? value('-t');
  if (platform === 'darwin') {
    if (architecture !== 'arm64' || (target && target !== 'aarch64-apple-darwin')) {
      throw new Error('the pinned macOS Browser runtime requires an Apple Silicon arm64 build');
    }
    if (args.some(arg => arg === '--no-bundle' || arg === '--bundles' || arg === '-b' || arg.startsWith('--bundles='))) {
      throw new Error('macOS build:fast produces a complete app bundle; bundle overrides are unsupported');
    }
    if (args.some(arg => arg === '--profile' || arg.startsWith('--profile=') || arg === '--release' || arg === '-r')) {
      throw new Error('macOS build:fast uses the debug profile; use build:mac for release builds');
    }
  }
  return {
    target,
    args: ['build', '--debug', ...(platform === 'darwin' ? ['--bundles', 'app', '--no-sign'] : ['--no-bundle']),
      '--config', 'apps/desktop/tauri.conf.json', '--config', 'apps/desktop/tauri.dev.conf.json', ...args],
  };
}

async function digestFile(path) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}

export async function findBuiltMacosApp(outDir, expectedSha256) {
  const expected = expectedSha256 ?? await digestFile(join(outDir, 'nomifun-desktop'));
  const apps = [];
  const directory = join(outDir, 'bundle/macos');
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.endsWith('.app')) continue;
    const appPath = join(directory, entry.name);
    const binary = join(appPath, 'Contents/MacOS/nomifun-desktop');
    try {
      if (await digestFile(binary) === expected) {
        apps.push(appPath);
      }
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
  }
  if (!apps.length) throw new Error('Tauri did not produce an app containing this exact desktop build');
  if (apps.length !== 1) throw new Error('The exact desktop build matches multiple app bundles; refusing an ambiguous build identity');
  return apps[0];
}

export async function freezeBuiltMacosApp(outDir) {
  // Do this immediately after Tauri returns, before another Cargo build can
  // replace the raw host artifact while waiting for the build-directory lock.
  const hostSha256 = await digestFile(join(outDir, 'nomifun-desktop'));
  const sourceAppPath = await findBuiltMacosApp(outDir, hostSha256);
  const sourceBinary = join(sourceAppPath, 'Contents/MacOS/nomifun-desktop');
  if (await digestFile(sourceBinary) !== hostSha256) {
    throw new Error('The verified Tauri app changed before freezing this fast build');
  }
  const sourcePlist = await readFile(join(sourceAppPath, 'Contents/Info.plist'));
  const infoPlistSha256 = createHash('sha256').update(sourcePlist).digest('hex');
  const destination = join(outDir, 'bundle/macos-fast');
  await mkdir(destination, { recursive: true });
  const generation = await mkdtemp(join(destination, 'run-'));
  const appPath = join(generation, basename(sourceAppPath));
  await cp(sourceAppPath, appPath, {
    recursive: true, dereference: false, verbatimSymlinks: true,
    mode: constants.COPYFILE_FICLONE,
  });
  const copiedHost = await digestFile(join(appPath, 'Contents/MacOS/nomifun-desktop'));
  const copiedPlist = createHash('sha256').update(await readFile(join(appPath, 'Contents/Info.plist'))).digest('hex');
  if (copiedHost !== hostSha256 || copiedPlist !== infoPlistSha256) {
    throw new Error('The verified Tauri app changed while freezing this fast build');
  }
  const receipt = { sourceAppPath, appPath, hostSha256, infoPlistSha256 };
  await writeFile(join(generation, 'build-identity.json'), `${JSON.stringify(receipt, null, 2)}\n`);
  return receipt;
}

export async function completeFastMacosApp({ outDir, buildHelper, environment = process.env, stage = stageMacosBrowserBundle }) {
  const identity = macosDevelopmentSigningIdentity(environment);
  const built = await freezeBuiltMacosApp(outDir);
  const { helperPath, runtimePath } = await buildHelper();
  // The helper can wait behind an active watcher for minutes. Only this
  // immutable verified copy is staged; raw Cargo/Tauri outputs are not reread.
  await stage({ appPath: built.appPath, helperPath, runtimePath, identity });
  return built.appPath;
}

export async function main(args = process.argv.slice(2)) {
  try {
    const invocation = fastBuildInvocation(args);
    const environment = {
      ...(process.platform === 'darwin' ? developmentEnvironment(process.env) : process.env),
      NOMI_CHANNEL: 'dev', CI: 'true',
    };
    if (process.platform === 'darwin') {
      const notice = macosDevelopmentSigningNotice(environment);
      if (notice) console.error(notice);
    }
    const tauri = join(ROOT, 'node_modules/.bin', process.platform === 'win32' ? 'tauri.exe' : 'tauri');
    const compiler = process.platform === 'darwin'
      ? await macosBrowserCompilerEnvironment({ target: invocation.target, environment })
      : { environment, runtimePath: null };
    const metadata = process.platform === 'darwin'
      ? JSON.parse(await runCommand('cargo', ['metadata', '--format-version', '1', '--no-deps'], { capture: true, environment }))
      : null;
    const outDir = metadata ? join(metadata.target_directory, ...(invocation.target ? [invocation.target] : []), 'debug') : null;
    await runCommand(tauri, invocation.args, { environment: compiler.environment });
    if (process.platform !== 'darwin') return;
    const appPath = await completeFastMacosApp({
      outDir,
      environment,
      buildHelper: async () => {
        const helperArgs = ['build', '--locked', '-p', 'nomifun-browser-macos', '--bin', 'nomifun-browser-cef-helper',
          ...(invocation.target ? ['--target', invocation.target] : []), '--message-format=json-render-diagnostics'];
        const helperPath = await runCargoArtifact(helperArgs, 'nomifun-browser-cef-helper', { environment: compiler.environment });
        const runtimeEnvironment = nativeBrowserLaunchEnvironment(environment);
        const runtimePath = compiler.runtimePath ?? await runCommand(helperPath, ['--print-runtime-path'], { capture: true, environment: runtimeEnvironment });
        return { helperPath, runtimePath };
      },
    });
    console.log(`[build:fast] complete native Browser app: ${appPath}`);
  } catch (error) {
    console.error(`[build:fast] ${error.message}`);
    process.exitCode = error.exitCode ?? 1;
  }
}

if (import.meta.main) await main();
