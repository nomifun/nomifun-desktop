#!/usr/bin/env bun
// Tauri's --runner replaces cargo, receiving `run [cargo args] -- [app args]`.
// Compile with Cargo's original options, then launch that exact artifact from
// a complete signed app. The surviving run-dev supervisor owns native exit.
import { spawn } from 'node:child_process';
import { constants, createReadStream } from 'node:fs';
import { cp, mkdir, mkdtemp, readFile, writeFile, rename } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { StringDecoder } from 'node:string_decoder';
import { dirname, join, resolve, relative, isAbsolute, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { inspectMacosAppBundle, signMacosAppBundle } from './lib/macos-app-bundle.mjs';
import { prepareMacosBuildEnvironment, resolveMacosTarget } from './lib/macos-build-environment.mjs';
import { acquireMacosDevGeneration } from './lib/macos-dev-supervisor.mjs';
import { macosDevelopmentSigningIdentity, macosDevelopmentSealArguments } from './lib/macos-dev-signing.mjs';
import { buildMacosSystemExecution } from './lib/macos-system-execution.mjs';
import { buildComputerWorker } from './computer/build-worker.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

function option(args, name) {
  const inline = args.find(arg => arg.startsWith(`${name}=`));
  if (inline) return inline.slice(name.length + 1);
  const index = args.indexOf(name);
  return index < 0 ? null : args[index + 1];
}

export function parseMacosCargoRunnerArguments(args, architecture = process.arch) {
  if (args[0] !== 'run') throw new Error('the macOS development runner accepts only Tauri cargo run');
  const separator = args.indexOf('--');
  const cargo = args.slice(1, separator < 0 ? undefined : separator);
  const application = separator < 0 ? [] : args.slice(separator + 1);
  const target = option(cargo, '--target');
  resolveMacosTarget(target, architecture);
  const binary = option(cargo, '--bin');
  if (binary && binary !== 'nomifun-desktop') throw new Error('Tauri development must run the nomifun-desktop host');
  if (cargo.some(arg => ['--example', '--examples', '--lib', '--bins', '--tests', '--benches', '--all-targets'].includes(arg))) {
    throw new Error('Tauri development requires its desktop binary target');
  }
  if (option(cargo, '--message-format')) throw new Error('the macOS development runner owns Cargo artifact reporting');
  if (!binary) cargo.push('--bin', 'nomifun-desktop');
  const profile = option(cargo, '--profile');
  return {
    build: ['build', ...cargo, '--message-format=json-render-diagnostics'],
    application,
    target,
    profile: profile ?? (cargo.includes('--release') || cargo.includes('-r') ? 'release' : 'debug'),
  };
}

export function macosDevelopmentApplicationArguments(args) {
  // AppKit documents this debug/test default for ignoring existing window
  // restoration state. NSArgumentDomain is process-local: a prior crashed dev
  // build cannot hold Tauri Ready/backend startup behind its recovery alert.
  // Do not write user defaults, remove saved state, or alter the signed bundle.
  // Keep caller arguments in their original order; the desktop main parses its
  // own fixed CLI argv, while Foundation consumes this native launch default.
  return [...args, '-ApplePersistenceIgnoreState', 'YES'];
}

export function runCommand(program, args, { cwd = ROOT, environment = process.env, capture = false, spawnCommand = spawn } = {}) {
  return new Promise((accept, reject) => {
    const child = spawnCommand(program, args, { cwd, env: environment, stdio: capture ? ['ignore', 'pipe', 'inherit'] : 'inherit' });
    let output = '';
    if (capture) {
      child.stdout.setEncoding('utf8');
      child.stdout.on('data', data => { output += data; });
    }
    child.once('error', reject);
    // close follows the process exit and the closing of its stdio. Metadata
    // JSON must include bytes retained by inherited writer descriptors.
    child.once('close', (code, signal) => code === 0
      ? accept(output.trim())
      : reject(new Error(`${program} exited ${code ?? signal}`)));
  });
}

export async function runCargoArtifact(args, binary, options = {}) {
  return (await runCargoArtifacts(args, [binary], options))[binary];
}

export async function runCargoArtifacts(args, binaries, { cwd = ROOT, environment = process.env, spawnCargo = spawn } = {}) {
  const child = spawnCargo('cargo', args, { cwd, env: environment, stdio: ['inherit', 'pipe', 'inherit'] });
  const completion = new Promise((accept, reject) => {
    child.once('error', reject);
    child.once('exit', (code, signal) => code === 0 ? accept() : reject(Object.assign(
      new Error(`cargo ${args[0]} exited ${code ?? signal}`), { exitCode: code ?? 1 },
    )));
  });
  // Subscribe before reading: an early cargo spawn error must stay observed.
  completion.catch(() => {});
  const executables = {};
  const decoder = new StringDecoder('utf8');
  const readReports = async () => {
    const report = line => {
      let result;
      try { result = JSON.parse(line); }
      catch { process.stdout.write(`${line}\n`); return; }
      if (result?.reason === 'compiler-message' && result.message?.rendered) process.stderr.write(result.message.rendered);
      if (result?.reason === 'compiler-artifact' && binaries.includes(result.target?.name) && result.executable) {
        executables[result.target.name] = resolve(cwd, result.executable);
      }
    };
    let buffered = '';
    for await (const bytes of child.stdout) {
      buffered += decoder.write(bytes);
      let end;
      while ((end = buffered.indexOf('\n')) >= 0) {
        report(buffered.slice(0, end));
        buffered = buffered.slice(end + 1);
      }
    }
    buffered += decoder.end();
    if (buffered) report(buffered);
  };
  try {
    await Promise.all([readReports(), completion]);
  } finally {
    child.stdout.destroy();
  }
  for (const binary of binaries) if (!executables[binary]) throw new Error(`Cargo did not report the ${binary} executable`);
  return executables;
}

async function digestFile(path) {
  const digest = createHash('sha256');
  for await (const chunk of createReadStream(path)) digest.update(chunk);
  return digest.digest('hex');
}

export async function freezeMacosDevelopmentController(controllerPath, root = ROOT) {
  const digest = await digestFile(controllerPath);
  const cache = join(root, 'target', 'macos-dev-app');
  await mkdir(cache, { recursive: true });
  const generation = await mkdtemp(join(cache, 'controller-'));
  const program = join(generation, 'macos_application_controller');
  await cp(controllerPath, program);
  if (await digestFile(program) !== digest || await digestFile(controllerPath) !== digest) {
    throw new Error('LaunchServices controller changed while freezing its exact build');
  }
  return program;
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
  // Preserve nested native dictionaries (for example web-content ATS) and
  // subsequent privacy keys. The last dict before </plist> is the root close.
  const native = permissions.match(/<dict>([\s\S]*)<\/dict>\s*<\/plist>/)?.[1];
  if (!native) throw new Error('desktop Info.plist has no permission dictionary');
  return `<?xml version="1.0" encoding="UTF-8"?>\n` +
    `<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n` +
    `<plist version="1.0"><dict>${Object.entries(values).map(([key, value]) =>
      `<key>${key}</key>${typeof value === 'boolean' ? `<${value}/>` : `<string>${escape(value)}</string>`}`,
    ).join('')}${native}</dict></plist>\n`;
}

export async function ensureMacosDevelopmentBundle({
  hostPath, target = resolveMacosTarget(), bundleFiles = {}, root = ROOT, environment = process.env,
}, { sign = signMacosAppBundle, inspect = inspectMacosAppBundle, run = runCommand } = {}) {
  const plist = await developmentInfoPlist(root, environment);
  const signingIdentity = macosDevelopmentSigningIdentity(environment);
  const entitlementsPath = join(root, 'apps/desktop/entitlements.plist');
  // Separate test identifiers/datasets can run beside an existing dev app.
  const identity = createHash('sha256').update(plist)
    .update(environment.NOMIFUN_DATA_DIR ?? '').update(target).update(JSON.stringify(signingIdentity)).digest('hex').slice(0, 16);
  const cache = join(root, 'target', 'macos-dev-app', identity);
  const fingerprintPath = join(cache, 'complete.json');
  const hostDigest = await digestFile(hostPath);
  const files = await Promise.all(Object.entries(bundleFiles).sort(([a], [b]) => a.localeCompare(b))
    .map(async ([destination, source]) => ({ destination, source, digest: await digestFile(source) })));
  const components = createHash('sha256').update(target)
    .update(JSON.stringify(signingIdentity))
    .update(JSON.stringify(files))
    .update(await readFile(join(root, 'scripts/lib/macos-app-bundle.mjs')))
    .update(await readFile(join(root, 'scripts/lib/macos-signing.mjs')))
    .update(await readFile(entitlementsPath))
    .digest('hex');
  const host = createHash('sha256').update(hostDigest).update(plist).digest('hex');
  let cached;
  try { cached = JSON.parse(await readFile(fingerprintPath, 'utf8')); } catch {}
  let complete = false;
  const cachedPath = typeof cached?.appPath === 'string' ? relative(cache, resolve(cached.appPath)) : '';
  if (cached?.components === components && cachedPath.startsWith('generation-')
    && !cachedPath.startsWith('..') && !isAbsolute(cachedPath)
    && cachedPath.endsWith(`${sep}NomiFun Dev.app`)) {
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
      // APFS can share immutable resource bytes; fall back to copying on
      // filesystems without clone support, preserving the same bundle contract.
      mode: constants.COPYFILE_FICLONE,
    });
  }
  await mkdir(join(contents, 'MacOS'), { recursive: true });
  await cp(hostPath, program);
  if (await digestFile(program) !== hostDigest) throw new Error('desktop build changed while freezing the development app');
  await writeFile(join(contents, 'Info.plist'), plist);
  if (complete) {
    // The resource fingerprint is unchanged. Reuse signed native libraries
    // and refresh the app seal in this new immutable generation.
    await run('/usr/bin/plutil', ['-convert', 'xml1', join(contents, 'Info.plist')]);
    await run('codesign', macosDevelopmentSealArguments(appPath, signingIdentity, entitlementsPath));
    await run('codesign', ['--verify', '--deep', '--strict', appPath]);
  } else {
    for (const file of files) {
      const destination = resolve(contents, file.destination);
      const owned = relative(contents, destination);
      if (!owned || owned.startsWith('..') || isAbsolute(owned)) throw new Error('development bundle resource must stay within Contents');
      await mkdir(dirname(destination), { recursive: true });
      await cp(file.source, destination);
      if (await digestFile(destination) !== file.digest) throw new Error('native resource changed while freezing the development app');
    }
    await sign({ appPath, target, identity: signingIdentity, entitlementsPath });
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
    const compiler = await prepareMacosBuildEnvironment({ target: plan.target });
    const artifacts = await runCargoArtifacts([...plan.build, '--example', 'macos_application_controller'],
      ['nomifun-desktop', 'macos_application_controller'], { cwd, environment: compiler.environment });
    const hostPath = artifacts['nomifun-desktop'];
    const systemFiles = await buildMacosSystemExecution({ root: ROOT, target: compiler.target, profile: plan.profile, bundleIdentifier: 'com.nomifun.desktop.dev', environment: compiler.environment });
    const worker = await buildComputerWorker({ root: ROOT, target: compiler.target, profile: plan.profile, environment: compiler.environment });
    const program = await ensureMacosDevelopmentBundle({ hostPath, target: compiler.target, bundleFiles: { ...compiler.bundleFiles, ...systemFiles, 'MacOS/nomifun-computer-worker': worker.binary } });
    const controllerProgram = await freezeMacosDevelopmentController(artifacts.macos_application_controller);
    console.error(`[dev] native app: ${dirname(dirname(dirname(program)))}`);
    const result = await generation.launch({ program, controllerProgram, args: macosDevelopmentApplicationArguments(plan.application), environment: { ...process.env, NOMIFUN_COMPUTER_WORKER: join(dirname(program), 'nomifun-computer-worker') }, cwd });
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
