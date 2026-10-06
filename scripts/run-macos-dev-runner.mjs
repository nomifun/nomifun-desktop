#!/usr/bin/env bun
// Tauri's --runner replaces cargo, receiving `run [cargo args] -- [app args]`.
// Compile with Cargo's original options, then launch that exact artifact from
// a complete signed app. The surviving run-dev supervisor owns native exit.
import { spawn } from 'node:child_process';
import { constants, createReadStream } from 'node:fs';
import { cp, mkdir, mkdtemp, readFile, writeFile, rename } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { createInterface } from 'node:readline';
import { dirname, join, resolve, relative, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  MACOS_BROWSER_RUNTIME, compileBrowserEnvironment, inspectMacosBrowserBundle, stageMacosBrowserBundle,
} from './lib/macos-browser-bundle.mjs';
import { acquireMacosDevGeneration } from './lib/macos-dev-supervisor.mjs';
import { macosDevelopmentSigningIdentity, macosDevelopmentSealArguments } from './lib/macos-dev-signing.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

function option(args, name) {
  const inline = args.find(arg => arg.startsWith(`${name}=`));
  if (inline) return inline.slice(name.length + 1);
  const index = args.indexOf(name);
  return index < 0 ? null : args[index + 1];
}

export function parseMacosCargoRunnerArguments(args, architecture = process.arch) {
  if (architecture !== 'arm64') throw new Error('the pinned macOS Browser runtime requires Apple Silicon arm64');
  if (args[0] !== 'run') throw new Error('the macOS development runner accepts only Tauri cargo run');
  const separator = args.indexOf('--');
  const cargo = args.slice(1, separator < 0 ? undefined : separator);
  const application = separator < 0 ? [] : args.slice(separator + 1);
  const target = option(cargo, '--target');
  if (target && target !== 'aarch64-apple-darwin') {
    throw new Error('the pinned macOS Browser runtime supports only aarch64-apple-darwin');
  }
  const binary = option(cargo, '--bin');
  if (binary && binary !== 'nomifun-desktop') throw new Error('Tauri development must run the nomifun-desktop host');
  if (cargo.some(arg => ['--example', '--examples', '--lib', '--bins', '--tests', '--benches', '--all-targets'].includes(arg))) {
    throw new Error('Tauri development requires its desktop binary target');
  }
  if (option(cargo, '--message-format')) throw new Error('the macOS development runner owns Cargo artifact reporting');
  if (!binary) cargo.push('--bin', 'nomifun-desktop');
  const helper = ['build', '--locked', '-p', 'nomifun-browser-macos', '--bin', 'nomifun-browser-cef-helper'];
  if (target) helper.push('--target', target);
  const profile = option(cargo, '--profile');
  if (profile) helper.push('--profile', profile);
  else if (cargo.includes('--release') || cargo.includes('-r')) helper.push('--release');
  return {
    build: ['build', ...cargo, '--message-format=json-render-diagnostics'],
    helper: [...helper, '--message-format=json-render-diagnostics'],
    application,
    target,
    profile: profile ?? (cargo.includes('--release') || cargo.includes('-r') ? 'release' : 'debug'),
  };
}

export function nativeBrowserLaunchEnvironment(environment = process.env) {
  const result = { ...environment };
  delete result.CEF_PATH;
  delete result.FLATPAK;
  return result;
}

export async function macosBrowserCompilerEnvironment({
  root = ROOT, target = null, profile = 'debug', environment = process.env,
}) {
  // CEF bytes are profile-independent. Custom Cargo profiles may reuse the
  // same verified debug/release runtime without changing their compile flags.
  return compileBrowserEnvironment({ root, target, profile: profile === 'release' ? 'release' : 'debug', environment });
}

export function runCommand(program, args, { cwd = ROOT, environment = process.env, capture = false } = {}) {
  return new Promise((accept, reject) => {
    const child = spawn(program, args, { cwd, env: environment, stdio: capture ? ['ignore', 'pipe', 'inherit'] : 'inherit' });
    let output = '';
    if (capture) child.stdout.on('data', data => { output += data; });
    child.once('error', reject);
    child.once('exit', (code, signal) => code === 0
      ? accept(output.trim())
      : reject(new Error(`${program} exited ${code ?? signal}`)));
  });
}

export async function runCargoArtifact(args, binary, { cwd = ROOT, environment = process.env } = {}) {
  const child = spawn('cargo', args, { cwd, env: environment, stdio: ['inherit', 'pipe', 'inherit'] });
  const completion = new Promise((accept, reject) => {
    child.once('error', reject);
    child.once('exit', (code, signal) => code === 0 ? accept() : reject(Object.assign(
      new Error(`cargo ${args[0]} exited ${code ?? signal}`), { exitCode: code ?? 1 },
    )));
  });
  // Subscribe before reading: an early cargo spawn error must stay observed.
  completion.catch(() => {});
  let executable;
  for await (const line of createInterface({ input: child.stdout, crlfDelay: Infinity })) {
    let result;
    try { result = JSON.parse(line); }
    catch { process.stdout.write(`${line}\n`); continue; }
    if (result.reason === 'compiler-message' && result.message?.rendered) process.stderr.write(result.message.rendered);
    if (result.reason === 'compiler-artifact' && result.target?.name === binary && result.executable) executable = result.executable;
  }
  await completion;
  if (!executable) throw new Error(`Cargo did not report the ${binary} executable`);
  return resolve(cwd, executable);
}

async function digestFile(path) {
  const digest = createHash('sha256');
  for await (const chunk of createReadStream(path)) digest.update(chunk);
  return digest.digest('hex');
}

export async function developmentInfoPlist(root = ROOT, environment = process.env) {
  const base = JSON.parse(await readFile(join(root, 'apps/desktop/tauri.conf.json'), 'utf8'));
  const dev = JSON.parse(await readFile(join(root, 'apps/desktop/tauri.dev.conf.json'), 'utf8'));
  const additional = environment.TAURI_CONFIG ? JSON.parse(environment.TAURI_CONFIG) : {};
  const name = additional.productName ?? dev.productName ?? base.productName;
  const identifier = additional.identifier ?? dev.identifier ?? base.identifier;
  const version = additional.version ?? dev.version ?? base.version
    ?? JSON.parse(await readFile(join(root, 'package.json'), 'utf8')).version;
  const escape = value => String(value).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
  const values = {
    CFBundlePackageType: 'APPL', CFBundleExecutable: 'nomifun-desktop',
    CFBundleName: name, CFBundleDisplayName: name, CFBundleIdentifier: identifier,
    CFBundleVersion: version, CFBundleShortVersionString: version,
    LSMinimumSystemVersion: '14.0', NSHighResolutionCapable: true,
  };
  const permissions = await readFile(join(root, 'apps/desktop/Info.plist'), 'utf8');
  const native = permissions.match(/<dict>([\s\S]*?)<\/dict>/)?.[1];
  if (!native) throw new Error('desktop Info.plist has no permission dictionary');
  return `<?xml version="1.0" encoding="UTF-8"?>\n` +
    `<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n` +
    `<plist version="1.0"><dict>${Object.entries(values).map(([key, value]) =>
      `<key>${key}</key>${typeof value === 'boolean' ? `<${value}/>` : `<string>${escape(value)}</string>`}`,
    ).join('')}${native}</dict></plist>\n`;
}

export async function ensureMacosDevelopmentBundle({
  hostPath, helperPath, runtimePath, root = ROOT, environment = process.env,
}, { stage = stageMacosBrowserBundle, inspect = inspectMacosBrowserBundle, run = runCommand } = {}) {
  const plist = await developmentInfoPlist(root, environment);
  const signingIdentity = macosDevelopmentSigningIdentity(environment);
  // Separate test identifiers/datasets can run beside an existing dev app.
  const identity = createHash('sha256').update(plist)
    .update(environment.NOMIFUN_DATA_DIR ?? '').update(JSON.stringify(signingIdentity)).digest('hex').slice(0, 16);
  const cache = join(root, 'target', 'macos-dev-browser', identity);
  const fingerprintPath = join(cache, 'complete.json');
  const hostDigest = await digestFile(hostPath);
  const components = createHash('sha256').update(JSON.stringify(MACOS_BROWSER_RUNTIME))
    .update(JSON.stringify(signingIdentity))
    .update(await digestFile(helperPath))
    .update(await readFile(join(root, 'scripts/lib/macos-browser-bundle.mjs')))
    .digest('hex');
  const host = createHash('sha256').update(hostDigest).update(plist).digest('hex');
  let cached;
  try { cached = JSON.parse(await readFile(fingerprintPath, 'utf8')); } catch {}
  let complete = false;
  const cachedPath = typeof cached?.appPath === 'string' ? relative(cache, resolve(cached.appPath)) : '';
  if (cached?.components === components && cachedPath.startsWith('generation-')
    && !cachedPath.startsWith('..') && !isAbsolute(cachedPath)
    && cachedPath.endsWith('/NomiFun Dev.app')) {
    try {
      const inspected = await inspect(cached.appPath);
      if (inspected.status === 'pass') {
        await run('codesign', ['--verify', '--deep', '--strict', cached.appPath]);
        complete = true;
      }
    } catch { /* A damaged completed cache is rebuilt in a new owned directory. */ }
  }
  if (complete && cached.host === host) return join(cached.appPath, 'Contents/MacOS/nomifun-desktop');
  // A watch SIGKILL can leave a codesign child finishing its old work. Every
  // unfinished build gets a private directory, and published apps are immutable.
  // A late subprocess therefore cannot rewrite the next generation's app.
  await mkdir(cache, { recursive: true });
  const generation = await mkdtemp(join(cache, 'generation-'));
  const appPath = join(generation, 'NomiFun Dev.app');
  const contents = join(appPath, 'Contents');
  const program = join(contents, 'MacOS', 'nomifun-desktop');
  if (complete) {
    await cp(cached.appPath, appPath, {
      recursive: true, dereference: false, verbatimSymlinks: true,
      // APFS can share the immutable framework bytes; fall back to copying on
      // filesystems without clone support, preserving the same bundle contract.
      mode: constants.COPYFILE_FICLONE,
    });
  }
  await mkdir(join(contents, 'MacOS'), { recursive: true });
  await cp(hostPath, program);
  await writeFile(join(contents, 'Info.plist'), plist);
  if (complete) {
    // Cargo's incremental host rebuild does not change the pinned signed
    // framework/helpers. Reuse those components and refresh only the app seal.
    await run('/usr/bin/plutil', ['-convert', 'xml1', join(contents, 'Info.plist')]);
    await run('codesign', macosDevelopmentSealArguments(appPath, signingIdentity));
    await run('codesign', ['--verify', '--deep', '--strict', appPath]);
  } else {
    await stage({ appPath, helperPath, runtimePath, identity: signingIdentity });
  }
  const publication = join(generation, 'complete.json');
  await writeFile(publication, `${JSON.stringify({ components, host, appPath })}\n`);
  await rename(publication, fingerprintPath);
  return program;
}

export async function main(args = process.argv.slice(2)) {
  let generation;
  try {
    if (process.platform !== 'darwin') throw new Error('the macOS development runner requires macOS');
    const plan = parseMacosCargoRunnerArguments(args);
    generation = await acquireMacosDevGeneration(process.env.NOMIFUN_DEV_SUPERVISOR_SOCKET);
    const cwd = process.cwd();
    const compiler = await macosBrowserCompilerEnvironment({ target: plan.target, profile: plan.profile });
    const hostPath = await runCargoArtifact(plan.build, 'nomifun-desktop', { cwd, environment: compiler.environment });
    const helperPath = await runCargoArtifact(plan.helper, 'nomifun-browser-cef-helper', { environment: compiler.environment });
    const runtimeEnvironment = nativeBrowserLaunchEnvironment();
    // This private helper command reports its exact compiled Cargo runtime
    // path without loading CEF or discovering an installation through PATH.
    const runtimePath = compiler.runtimePath ?? await runCommand(helperPath, ['--print-runtime-path'], { capture: true, environment: runtimeEnvironment });
    const program = await ensureMacosDevelopmentBundle({ hostPath, helperPath, runtimePath });
    console.error(`[dev] native Browser app: ${dirname(dirname(dirname(program)))}`);
    const result = await generation.launch({ program, args: plan.application, environment: runtimeEnvironment, cwd });
    process.exitCode = result.code ?? (result.signal ? 1 : 0);
  } catch (error) {
    // Tauri 2.11 recognizes a Cargo compilation failure by exit 101 and the
    // last stderr line containing this phrase. Preserve watch-after-error.
    console.error(`[dev] ${error.exitCode === 101 ? 'could not compile: ' : ''}${error.message}`);
    process.exitCode = error.exitCode ?? 1;
  } finally {
    generation?.close();
  }
}

if (import.meta.main) await main();
