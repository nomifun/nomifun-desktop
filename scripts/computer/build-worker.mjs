#!/usr/bin/env node
// Build the independent worker workspace and stage Tauri's exact sidecar name.
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { chmod, copyFile, mkdir, stat } from 'node:fs/promises';
import { createInterface } from 'node:readline';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
export function workerBuildPlan({ root = ROOT, target, profile = 'release', environment = process.env } = {}) {
  if (!target || !/^[a-z0-9_]+-(?:[a-z0-9_]+-)+[a-z0-9_]+$/.test(target)) throw new Error('A concrete Rust target triple is required for the Computer worker');
  if (!['debug', 'release'].includes(profile)) throw new Error(`Unsupported Computer worker profile: ${profile}`);
  const worker = join(root, 'apps/computer-worker');
  const targetDirectory = resolve(root, environment.CARGO_TARGET_DIR || join(worker, 'target'));
  const buildEnvironment = { ...environment, CARGO_TARGET_DIR: targetDirectory };
  // Cargo's configured build-dir can otherwise mix the root and independent
  // workspaces, or native and cross-target intermediate files.
  buildEnvironment.CARGO_BUILD_BUILD_DIR ||= join(targetDirectory, `build-${target}.noindex`);
  const suffix = target.includes('-windows-') ? '.exe' : '';
  return {
    args: ['build', '--locked', '--manifest-path', join(worker, 'Cargo.toml'), '--bin', 'nomifun-computer-worker', '--target', target, '--profile', profile === 'debug' ? 'dev' : profile, '--message-format=json-render-diagnostics'],
    environment: buildEnvironment,
    destination: join(root, 'apps/desktop/binaries', `nomifun-computer-worker-${target}${suffix}`),
  };
}

export function cargoWorkerArtifact(line) {
  let value;
  try { value = JSON.parse(line); } catch { return null; }
  return value.reason === 'compiler-artifact' && value.target?.name === 'nomifun-computer-worker'
    && value.target.kind?.includes('bin') && typeof value.executable === 'string' ? value.executable : null;
}

async function inspectStagedWorker(artifact, destination) {
  let staged;
  try { staged = await stat(destination); }
  catch (error) { if (error.code === 'ENOENT') return { matches: false }; throw error; }
  const source = await stat(artifact);
  if (!staged.isFile() || staged.size !== source.size) return { matches: false };
  const digest = async file => {
    const hash = createHash('sha256');
    for await (const bytes of createReadStream(file)) hash.update(bytes);
    return hash.digest('hex');
  };
  const [sourceDigest, stagedDigest] = await Promise.all([digest(artifact), digest(destination)]);
  return { matches: sourceDigest === stagedDigest, mode: staged.mode };
}

export async function buildComputerWorker({ root = ROOT, target, profile = 'release', environment = process.env } = {}, { spawnCargo = spawn, stage = copyFile, createDirectory = mkdir, executableMode = chmod, inspectStaged = inspectStagedWorker } = {}) {
  const plan = workerBuildPlan({ root, target, profile, environment });
  const child = spawnCargo('cargo', plan.args, { cwd: root, env: plan.environment, stdio: ['inherit', 'pipe', 'inherit'], windowsHide: true });
  const finished = new Promise((accept, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => code === 0 ? accept() : reject(new Error(`Computer worker build failed: ${code ?? signal}`)));
  });
  // Attach rejection handling before reading a possibly empty stdout stream.
  finished.catch(() => {});
  let artifact;
  for await (const line of createInterface({ input: child.stdout, crlfDelay: Infinity })) {
    const candidate = cargoWorkerArtifact(line);
    if (candidate) artifact = resolve(root, candidate);
    else {
      try { const event = JSON.parse(line); if (event.message?.rendered) process.stderr.write(event.message.rendered); } catch { process.stderr.write(line + '\n'); }
    }
  }
  await finished;
  if (!artifact) throw new Error('Cargo did not report the actual Computer worker executable');
  const staged = await inspectStaged(artifact, plan.destination);
  if (!staged.matches) {
    await createDirectory(dirname(plan.destination), { recursive: true });
    await stage(artifact, plan.destination);
  }
  // Tauri watches this sidecar and Cargo tracks its mtime as a build input.
  // Preserve both content and metadata when a cached worker is already staged.
  if (!target.includes('-windows-') && (!staged.matches || (staged.mode & 0o111) !== 0o111)) {
    await executableMode(plan.destination, 0o755);
  }
  return { binary: plan.destination, artifact, target, profile };
}

export function rustHostTarget(environment = process.env) {
  const result = spawnSync('rustc', ['-vV'], { env: environment, encoding: 'utf8', windowsHide: true });
  if (result.error || result.status !== 0) throw new Error(`Cannot identify Rust host target: ${result.error ?? result.stderr}`);
  const target = result.stdout.match(/^host:\s*(\S+)$/m)?.[1];
  if (!target) throw new Error('rustc did not report its host target');
  return target;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const args = process.argv.slice(2);
  let target, profile = 'release';
  for (let index = 0; index < args.length; index++) {
    if (args[index] === '--target') target = args[++index];
    else if (args[index] === '--profile') profile = args[++index];
    else throw new Error(`Unknown Computer build argument: ${args[index]}`);
  }
  const result = await buildComputerWorker({ target: target ?? rustHostTarget(), profile });
  process.stdout.write(`${result.binary}\n`);
}
