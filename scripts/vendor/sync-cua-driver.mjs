#!/usr/bin/env node
// Cua Driver's immutable source identity and D03 extraction live here. Never
// resolve a release tag as authority for the contents of an existing snapshot.
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, mkdirSync, writeFileSync, mkdtempSync, rmSync, lstatSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const vendor = join(root, 'vendor/cua-driver');
const repository = 'https://github.com/trycua/cua.git';
const tag = 'cua-driver-rs-v0.37.0';
const commit = '4d959bbfa57f0c89f310330e849500b222084194';
const upstreamRoot = 'libs/cua-driver/rust/';
const crates = ['cua-driver-sdk', 'cua-driver-core', 'cua-driver-contract', 'platform-macos', 'platform-windows', 'platform-linux', 'cursor-overlay', 'cua-cursor-motion', 'pip-preview'];
const patchPaths = [
  'vendor/patches/cua-driver/0001-platform-core-without-default-features.patch',
  'vendor/patches/cua-driver/0002-x11-owned-input-crash-cleanup.patch',
  'vendor/patches/cua-driver/0003-windows-user-app-lifetime.patch',
];
const metadataName = 'SOURCE.json';
const patchApplication = 'git-apply-text-lf';
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
const gitBlob = (bytes) => createHash('sha1').update(`blob ${bytes.length}\0`).update(bytes).digest('hex');

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, encoding: null, maxBuffer: 128 * 1024 * 1024, ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} ${args.join(' ')} failed (${result.status}):\n${result.stderr?.toString() ?? ''}`);
  return result.stdout;
}

function filesUnder(directory, prefix = '') {
  if (!existsSync(directory)) return [];
  return readdirSync(directory).sort().flatMap((name) => {
    const file = join(directory, name);
    const path = `${prefix}${name}`;
    const stat = lstatSync(file);
    if (stat.isSymbolicLink()) throw new Error(`Symlink is not allowed in the snapshot: ${file}`);
    return stat.isDirectory() ? filesUnder(file, `${path}/`) : [path];
  });
}

function validSnapshotPath(path) {
  return typeof path === 'string' && /^[A-Za-z0-9._/-]+$/.test(path) && !path.startsWith('/') && !path.split('/').some((part) => !part || part === '.' || part === '..');
}

function patchInventory(paths = patchPaths) {
  const records = [];
  const files = new Map();
  const labels = new Set();
  for (const path of paths) {
    const bytes = readFileSync(join(root, path));
    const text = bytes.toString();
    const label = text.match(/^Subject:\s*\[([^\]]+)\]/m)?.[1] ?? basename(path, '.patch');
    if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(label) || label === 'workspace-members' || labels.has(label)) throw new Error(`Invalid or duplicate registered patch label: ${path}`);
    labels.add(label);
    records.push({ path, sha256: sha256(bytes) });
    const sections = text.split(/^diff --git /m).slice(1);
    if (!sections.length) throw new Error(`Registered patch has no file diffs: ${path}`);
    const seenPaths = new Set();
    for (const section of sections) {
      const header = section.match(/^a\/([^\s]+) b\/([^\s]+)\r?\n/);
      const target = header?.[1];
      if (!header || target !== header[2] || !validSnapshotPath(target) || seenPaths.has(target)) throw new Error(`Invalid registered patch target: ${path}`);
      seenPaths.add(target);
      if (/^(?:deleted file mode|old mode|new mode|rename |copy |similarity index|Binary files|GIT binary patch)/m.test(section)) throw new Error(`Only text modifications and new source files are supported: ${path}`);
      const added = /^new file mode 100644\r?$/m.test(section);
      if (/^new file mode /m.test(section) && !added) throw new Error(`Unsupported added-file mode: ${path}`);
      const lines = section.split(/\r?\n/);
      if (!lines.includes(`--- ${added ? '/dev/null' : `a/${target}`}`) || !lines.includes(`+++ b/${target}`) || !/^@@ /m.test(section)) throw new Error(`Invalid text patch headers: ${path}`);
      if (added && (!target.startsWith('crates/') || !crates.includes(target.split('/')[1]) || target.split('/')[2] !== 'src')) throw new Error(`New patch files must be sources of an approved D03 crate: ${target}`);
      const effect = files.get(target) ?? { labels: [] };
      if (added) {
        if (files.has(target)) throw new Error(`Patch adds an already changed path: ${target}`);
        effect.added_by_patch = path;
      }
      effect.labels.push(label);
      files.set(target, effect);
    }
  }
  return { records, files };
}

function transformation(path, inventory) {
  return [...(path === 'Cargo.toml' ? ['workspace-members'] : []), ...(inventory.files.get(path)?.labels ?? [])].join('+') || undefined;
}

function upstreamPathForSnapshot(path) {
  if (path === 'LICENSE.md') return path;
  if (path === 'contract/manifest.json') return 'libs/cua-driver/contract/manifest.json';
  return `${upstreamRoot}${path}`;
}

function checkLocal({ forUpdate = false } = {}) {
  const metadata = JSON.parse(readFileSync(join(vendor, metadataName), 'utf8'));
  if (metadata.format !== 1 || metadata.repository !== repository || metadata.commit !== commit || metadata.tag !== tag) throw new Error('Unexpected vendor source identity');
  if (!forUpdate && metadata.patch_application !== patchApplication) throw new Error('Unexpected vendor patch application mode');
  if (!Array.isArray(metadata.patches) || metadata.patches.some((patch) => typeof patch.path !== 'string' || !/^[a-f0-9]{64}$/.test(patch.sha256))) throw new Error('Invalid recorded vendor patches');
  const recordedPaths = metadata.patches.map((patch) => patch.path);
  const inventory = forUpdate ? null : patchInventory();
  if (forUpdate) {
    // An approved patch may have changed since this owned snapshot was made.
    // Validate the exact recorded output below, without interpreting its old
    // transformation labels through the newly edited patch contents.
    if (JSON.stringify(recordedPaths) !== JSON.stringify(patchPaths.slice(0, recordedPaths.length))) throw new Error('Recorded patches are not an ordered prefix of the registered patch series');
  } else if (JSON.stringify(metadata.patches) !== JSON.stringify(inventory.records)) throw new Error('Registered patch content differs from SOURCE.json');
  const expected = metadata.files.map((file) => file.path).sort();
  if (new Set(expected).size !== expected.length || expected.some((path) => !validSnapshotPath(path))) throw new Error('Invalid vendor file list');
  const actual = filesUnder(vendor).filter((path) => path !== metadataName).sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error('Vendor file inventory differs from SOURCE.json');
  for (const file of metadata.files) {
    const effect = inventory?.files.get(file.path);
    if (inventory && file.transform !== transformation(file.path, inventory)) throw new Error(`Unexpected vendor transformation: ${file.path}`);
    if (file.transform !== undefined && (typeof file.transform !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9._-]*(?:\+[A-Za-z0-9][A-Za-z0-9._-]*)*$/.test(file.transform))) throw new Error(`Invalid recorded vendor transformation: ${file.path}`);
    const addedBy = forUpdate ? file.added_by_patch : effect?.added_by_patch;
    if (addedBy) {
      if (file.upstream_path !== null || file.upstream_blob !== null || file.added_by_patch !== addedBy || !recordedPaths.includes(addedBy) || !file.transform || !file.path.startsWith('crates/') || !crates.includes(file.path.split('/')[1]) || file.path.split('/')[2] !== 'src') throw new Error(`Invalid patch-added source identity: ${file.path}`);
    } else if (file.upstream_path !== upstreamPathForSnapshot(file.path) || !/^[a-f0-9]{40}$/.test(file.upstream_blob) || file.added_by_patch !== undefined) throw new Error(`Invalid upstream source identity: ${file.path}`);
    const bytes = readFileSync(join(vendor, file.path));
    if (sha256(bytes) !== file.sha256) throw new Error(`Vendor content differs: ${file.path}`);
    if (!file.transform && gitBlob(bytes) !== file.upstream_blob) throw new Error(`Unpatched upstream blob differs: ${file.path}`);
  }
  if (inventory) for (const path of inventory.files.keys()) if (!expected.includes(path)) throw new Error(`Registered patch target missing from vendor: ${path}`);
  console.log(`Cua Driver vendor verified: ${metadata.files.length} files, commit ${commit}, ${metadata.patches.length} registered patch`);
  return metadata;
}

function sourceFiles(source) {
  const git = (args) => run('git', ['-C', source, ...args], { env: { ...process.env, GIT_NO_LAZY_FETCH: '1' } });
  const actual = git(['rev-parse', 'HEAD']).toString().trim();
  if (actual !== commit) throw new Error(`Source HEAD is ${actual}; required ${commit}`);
  const tagCommit = git(['rev-parse', `${tag}^{commit}`]).toString().trim();
  if (tagCommit !== commit) console.log(`Source note: ${tag} resolves to ${tagCommit}; immutable commit ${commit} is authoritative.`);
  const entries = git(['ls-tree', '-r', '-z', commit]).toString().split('\0').filter(Boolean).map((entry) => {
    const [modeTypeHash, path] = entry.split('\t');
    const [mode, type, blob] = modeTypeHash.split(' ');
    return { mode, type, blob, path };
  });
  // Cargo parses explicit targets even when a crate is only a dependency on
  // another platform. Retain just those source files, never their fixtures.
  const explicitTargets = new Set();
  for (const crate of crates) {
    const prefix = `${upstreamRoot}crates/${crate}/`;
    const manifest = git(['show', `${commit}:${prefix}Cargo.toml`]).toString();
    for (const match of manifest.matchAll(/^\[\[(test|example)\]\]\r?\n([\s\S]*?)(?=^\[|$(?![\s\S]))/gm)) {
      const name = match[2].match(/^name\s*=\s*"([^"]+)"/m)?.[1];
      const path = match[2].match(/^path\s*=\s*"([^"]+)"/m)?.[1] ?? `${match[1]}s/${name}.rs`;
      if (!name || path.split('/').includes('..')) throw new Error(`Unsupported explicit ${match[1]} target in ${crate}`);
      explicitTargets.add(`${prefix}${path}`);
    }
  }
  const selected = [];
  for (const entry of entries) {
    let destination;
    if (entry.path === 'LICENSE.md') destination = 'LICENSE.md';
    if (entry.path === 'libs/cua-driver/contract/manifest.json') destination = 'contract/manifest.json';
    if (entry.path.startsWith(upstreamRoot)) {
      const path = entry.path.slice(upstreamRoot.length);
      if (['Cargo.toml', 'Cargo.lock', 'THIRD_PARTY_NOTICES.md', 'VERSION'].includes(path) || path.startsWith('Skills/cua-driver/')) destination = path;
      const [cratesDirectory, crate, part] = path.split('/');
      if (cratesDirectory === 'crates' && crates.includes(crate) && (!['tests', 'examples', 'fixtures', 'dev', 'fuzz', 'testdata'].includes(part) || explicitTargets.has(entry.path))) destination = path;
    }
    if (!destination) continue;
    if (entry.type !== 'blob' || entry.mode !== '100644') throw new Error(`Unsupported source object ${entry.mode} ${entry.path}`);
    selected.push({ ...entry, destination });
  }
  return { selected, tagCommit, git };
}

function projectSnapshot(source, staging) {
  const { selected, tagCommit, git } = sourceFiles(source);
  const inventory = patchInventory();
  const records = [];
  for (const entry of selected) {
    let bytes = git(['show', `${commit}:${entry.path}`]);
    if (bytes.subarray(0, 80).toString().startsWith('version https://git-lfs.github.com/spec/v1')) throw new Error(`Unhydrated LFS object: ${entry.path}`);
    const record = { path: entry.destination, upstream_path: entry.path, upstream_blob: entry.blob };
    if (entry.destination === 'Cargo.toml') {
      const sourceToml = bytes.toString();
      if (!sourceToml.includes('members = [')) throw new Error('Upstream workspace members syntax changed');
      bytes = Buffer.from(sourceToml.replace(/members = \[[\s\S]*?\]/, `members = [\n${crates.map((crate) => `    "crates/${crate}",`).join('\n')}\n]`));
      record.transform = 'workspace-members';
    }
    const output = join(staging, entry.destination);
    mkdirSync(dirname(output), { recursive: true });
    writeFileSync(output, bytes);
    records.push(record);
  }
  for (const [path, effect] of inventory.files) {
    const existing = records.some((record) => record.path === path);
    if (effect.added_by_patch) {
      if (existing) throw new Error(`Patch adds an existing upstream source: ${path}`);
      records.push({ path, upstream_path: null, upstream_blob: null, added_by_patch: effect.added_by_patch });
    } else if (!existing) throw new Error(`Patch changes a file outside the D03 snapshot: ${path}`);
  }
  for (const patch of patchPaths) {
    // Preserve the registered patch's exact identity, while applying textual
    // hunks with the repository's LF contract on every platform. Otherwise a
    // CRLF-authored new file drifts from SOURCE.json after Git checkout.
    const input = Buffer.from(readFileSync(join(root, patch), 'utf8').replace(/\r\n/g, '\n'));
    const gitTextConfig = ['-c', 'core.autocrlf=false', '-c', 'core.eol=lf'];
    run('git', [...gitTextConfig, 'apply', '--check', '-'], { cwd: staging, input });
    run('git', [...gitTextConfig, 'apply', '-'], { cwd: staging, input });
  }
  for (const record of records) {
    const transform = transformation(record.path, inventory);
    if (transform) record.transform = transform;
    record.sha256 = sha256(readFileSync(join(staging, record.path)));
  }
  records.sort((left, right) => left.path.localeCompare(right.path, 'en'));
  const metadata = { format: 1, repository, tag, commit, observed_tag_commit: tagCommit, upstream_root: upstreamRoot.slice(0, -1), extraction: 'D03: nine crates, no test fixtures; explicit Cargo target sources and embedded Skills/assets retained; workspace members projected only', patch_application: patchApplication, patches: inventory.records, files: records };
  writeFileSync(join(staging, metadataName), `${JSON.stringify(metadata, null, 2)}\n`);
  return metadata;
}

function compareSnapshot(staging) {
  const expectedFiles = filesUnder(staging);
  const currentFiles = filesUnder(vendor);
  const all = [...new Set([...expectedFiles, ...currentFiles])].sort();
  const changes = all.filter((file) => !existsSync(join(vendor, file)) || !existsSync(join(staging, file)) || !readFileSync(join(vendor, file)).equals(readFileSync(join(staging, file))));
  return { changes, expectedFiles };
}

function installSnapshot(staging, expectedFiles) {
  // A changed local snapshot is never overwritten by sync. Existing snapshots
  // must pass their recorded baseline before replacement; only owned files can
  // be removed. Do not remove or rename the shared checkout directory.
  if (existsSync(vendor)) {
    if (!existsSync(join(vendor, metadataName))) throw new Error('Existing vendor directory has no owned snapshot manifest; refusing overwrite');
    checkLocal({ forUpdate: true });
    const retained = new Set(expectedFiles);
    for (const file of filesUnder(vendor)) if (!retained.has(file)) rmSync(join(vendor, file));
  }
  for (const file of expectedFiles) {
    const output = join(vendor, file);
    mkdirSync(dirname(output), { recursive: true });
    writeFileSync(output, readFileSync(join(staging, file)));
  }
}

function buildWorker(manifest, packageId, env) {
  const args = ['build', '--locked', '--manifest-path', manifest, '--bin', 'nomifun-computer-worker', '--message-format=json-render-diagnostics'];
  const result = spawnSync('cargo', args, { cwd: root, env, encoding: 'utf8', maxBuffer: 128 * 1024 * 1024, stdio: ['inherit', 'pipe', 'inherit'] });
  if (result.error) throw result.error;
  const executables = new Set();
  for (const line of (result.stdout ?? '').split(/\r?\n/).filter(Boolean)) {
    const event = JSON.parse(line);
    if (event.reason === 'compiler-message' && event.message?.rendered) process.stderr.write(event.message.rendered);
    if (event.reason === 'compiler-artifact' && event.package_id === packageId && event.target?.name === 'nomifun-computer-worker' && event.target.kind.includes('bin') && event.executable) executables.add(event.executable);
  }
  if (result.status !== 0) throw new Error(`cargo build failed (${result.status ?? result.signal})`);
  if (executables.size !== 1) throw new Error(`Cargo reported ${executables.size} worker executables; expected exactly one`);
  return [...executables][0];
}

function main() {
  const args = process.argv.slice(2);
  const flags = new Set();
  let source;
  for (let index = 0; index < args.length; index++) {
    const arg = args[index];
    if (arg === '--source') {
      if (!args[index + 1]) throw new Error('--source requires a checkout directory');
      source = resolve(args[++index]);
    } else if (['--check', '--verify-upstream', '--vendor-only', '--help'].includes(arg)) flags.add(arg);
    else throw new Error(`Unknown argument: ${arg}`);
  }
  if (flags.has('--help')) {
    console.log('Usage: node scripts/vendor/sync-cua-driver.mjs [--source <read-only checkout>] [--vendor-only | --check | --verify-upstream]\nDefault: sync vendor, build worker, regenerate tools and run protocol tests.\n--check: offline snapshot/patch integrity. --verify-upstream: rebuild snapshot from pinned upstream and compare without writing.');
    return;
  }
  if (flags.has('--check')) {
    if (flags.size !== 1 || source) throw new Error('--check does not accept other options');
    checkLocal();
    return;
  }
  const temporaryBase = realpathSync(tmpdir());
  const temporary = mkdtempSync(join(temporaryBase, 'nomifun-cua-driver-sync-'));
  try {
    if (!source) {
      source = join(temporary, 'upstream');
      run('git', ['clone', '--filter=blob:none', '--sparse', '--branch', tag, '--single-branch', repository, source], { stdio: 'inherit' });
      run('git', ['-C', source, 'fetch', 'origin', commit], { stdio: 'inherit' });
      run('git', ['-C', source, 'sparse-checkout', 'set', 'libs/cua-driver/rust', 'libs/cua-driver/contract'], { stdio: 'inherit' });
      run('git', ['-C', source, 'checkout', '--detach', commit], { stdio: 'inherit' });
    }
    const staging = join(temporary, 'snapshot');
    mkdirSync(staging);
    const metadata = projectSnapshot(source, staging);
    const { changes, expectedFiles } = compareSnapshot(staging);
    console.log(`Cua Driver snapshot: ${metadata.files.length} files; ${changes.length} file differences.`);
    if (flags.has('--verify-upstream')) {
      checkLocal();
      if (changes.length) throw new Error(`Vendor differs from upstream + registered patch:\n${changes.join('\n')}`);
      return;
    }
    if (changes.length) installSnapshot(staging, expectedFiles);
    checkLocal();
  } finally {
    // Verify the final absolute target before recursive deletion on Windows.
    // Caller-supplied source paths are never modified or removed.
    const resolvedTemporary = realpathSync(temporary);
    if (dirname(resolvedTemporary) !== temporaryBase || !basename(resolvedTemporary).startsWith('nomifun-cua-driver-sync-')) throw new Error(`Unsafe temporary cleanup target: ${resolvedTemporary}`);
    rmSync(temporary, { recursive: true, force: true });
  }
  if (flags.has('--vendor-only')) return;
  const worker = join(root, 'apps/computer-worker');
  const manifest = join(worker, 'Cargo.toml');
  const cargoEnv = { ...process.env };
  // Discovery is read-only. Never let compiling processes in Windows and WSL
  // share root-config build.noindex merely because they share this checkout.
  const cargoMetadata = JSON.parse(run('cargo', ['metadata', '--locked', '--no-deps', '--format-version', '1', '--manifest-path', manifest], { env: cargoEnv }).toString());
  if (typeof cargoMetadata.target_directory !== 'string' || !cargoMetadata.target_directory) throw new Error('Cargo metadata has no worker target_directory');
  const packageId = cargoMetadata.packages.find((pkg) => pkg.name === 'nomifun-computer-worker')?.id;
  if (typeof packageId !== 'string') throw new Error('Cargo metadata has no worker package identity');
  if (cargoEnv.CARGO_BUILD_BUILD_DIR === undefined) cargoEnv.CARGO_BUILD_BUILD_DIR = join(cargoMetadata.target_directory, `build-${process.platform}-${process.arch}.noindex`);
  console.log(`Cua Driver worker build directory: ${cargoEnv.CARGO_BUILD_BUILD_DIR}`);
  const binary = buildWorker(manifest, packageId, cargoEnv);
  console.log(`Cua Driver worker executable: ${binary}`);
  run(binary, ['--self-check'], { stdio: 'inherit' });
  run(process.execPath, [join(root, 'scripts/computer/freeze-tools.mjs'), '--worker', binary], { stdio: 'inherit' });
  run('cargo', ['test', '--locked', '--manifest-path', manifest], { env: cargoEnv, stdio: 'inherit' });
}

try { main(); } catch (error) { console.error(`sync-cua-driver: ${error.message}`); process.exitCode = 1; }
