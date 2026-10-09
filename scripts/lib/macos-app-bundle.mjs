// Generic macOS application signing and distribution integrity.
// Browser content uses system WKWebView; no runtime is downloaded or bundled.

import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  mkdir,
  mkdtemp,
  open,
  readFile,
  readdir,
  readlink,
  realpath,
  rename,
  rm,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stat } from 'node:fs/promises';
import { createReadStream } from 'node:fs';

const isOwned = (root, path) => {
  const owned = relative(root, path);
  return !!owned && owned !== '..' && !owned.startsWith(`..${process.platform === 'win32' ? '\\' : '/'}`) && !isAbsolute(owned);
};

// WKWebView is supplied by macOS. A product must never carry a browser runtime.
export async function inspectMacosAppBundle(appPath) {
  const app = resolve(appPath);
  const contents = join(app, 'Contents');
  const ownedRoot = await realpath(app).catch(error => {
    if (error.code === 'ENOENT') return null;
    throw error;
  });
  const missing = [];
  for (const [label, path, executable] of [
    ['host', join(contents, 'MacOS', 'nomifun-desktop'), true],
    ['Info.plist', join(contents, 'Info.plist'), false],
  ]) {
    const info = await stat(path).catch(error => {
      if (error.code === 'ENOENT') return null;
      throw error;
    });
    if (!info?.isFile() || (executable && !(info.mode & 0o111))) {
      missing.push({ label, path });
    } else if (!ownedRoot || !isOwned(ownedRoot, await realpath(path))) {
      missing.push({ label: `${label} outside its application bundle`, path });
    }
  }
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true }).catch(error => {
      if (error.code === 'ENOENT') return [];
      throw error;
    })) {
      const path = join(directory, entry.name);
      if (['Chromium Embedded Framework.framework', 'WebKit.framework', 'browser-cef', 'nomifun-browser-cef-helper'].includes(entry.name)
          || /^NomiFun Helper(?: \(.*\))?\.app$/.test(entry.name)) {
        missing.push({ label: `unexpected bundled browser runtime: ${entry.name}`, path });
        continue;
      }
      if (entry.isSymbolicLink()) {
        const target = await realpath(path).catch(() => null);
        if (!ownedRoot || !target || !isOwned(ownedRoot, target)) missing.push({ label: 'application symlink escapes its bundle', path });
      } else if (entry.isDirectory()) await visit(path);
    }
  }
  await visit(contents);
  return { status: missing.length ? 'fail' : 'pass', appPath: app, missing };
}

const run = (command, argv, capture = false, environment = process.env) => new Promise((accept, reject) => {
  const child = spawn(command, argv, {
    stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
    env: Object.fromEntries(['PATH', 'HOME', 'TMPDIR', 'LANG', 'DEVELOPER_DIR', 'COPYFILE_DISABLE']
      .filter(key => environment[key])
      .map(key => [key, environment[key]])),
  });
  let output = '';
  let diagnostics = '';
  if (capture) {
    child.stdout.on('data', chunk => { output += chunk; });
    child.stderr.on('data', chunk => { diagnostics += chunk; });
  }
  child.once('error', reject);
  child.once('close', code => code === 0
    ? accept(output.trim())
    : reject(new Error(`${command} exited ${code}${capture ? `: ${diagnostics || output}` : ''}`)));
});

export async function resolveMacosBuildSettings(args, { root }) {
  const configurations = ['apps/desktop/tauri.conf.json', 'apps/desktop/tauri.macos.conf.json'];
  for (let index = 0; index < args.length; index++) {
    const argument = args[index];
    if (argument === '--config' || argument === '-c') {
      const value = args[++index];
      if (!value || value.startsWith('--')) throw new Error(`${argument} requires a value`);
      configurations.push(value);
    } else if (argument.startsWith('--config=')) configurations.push(argument.slice('--config='.length));
  }
  let createUpdaterArtifacts = false;
  let productName;
  for (const configuration of configurations) {
    let content;
    try {
      content = configuration.trim().startsWith('{') ? JSON.parse(configuration)
        : JSON.parse(await readFile(resolve(root, configuration), 'utf8'));
    } catch {
      throw new Error('macOS packaging requires readable JSON --config overlays');
    }
    if (Object.hasOwn(content.bundle || {}, 'createUpdaterArtifacts')) {
      const enabled = content.bundle.createUpdaterArtifacts;
      if (![true, false, 'v1Compatible'].includes(enabled)) throw new Error('invalid createUpdaterArtifacts value');
      createUpdaterArtifacts = enabled !== false;
    }
    if (Object.hasOwn(content, 'productName')) productName = content.productName;
  }
  if (productName !== 'NomiFun') throw new Error('macOS product packaging requires the NomiFun bundle name; use build:fast for development products');
  return { createUpdaterArtifacts };
}

export async function verifyMacosUpdaterArchive(archivePath, appPath) {
  // libarchive normally hides AppleDouble entries when listing a macOS archive.
  // Tauri's Rust tar reader sees them and strips one path component from EVERY
  // entry, turning a top-level ._NomiFun.app file into the extraction directory.
  const entries = (await run('/usr/bin/tar', ['-tzf', archivePath,
    ...(process.platform === 'darwin' ? ['--options=!mac-ext'] : [])], true)).split('\n');
  const prefix = basename(appPath);
  const seen = new Set();
  for (const entry of entries) {
    const normalized = entry.replace(/\/$/, '');
    const parts = normalized.split('/');
    if (parts[0] !== prefix || (parts.length === 1 && !entry.endsWith('/')) || seen.has(normalized) ||
        parts.some(part => !part || part === '.' || part === '..' || part.startsWith('._') || part === '__MACOSX')) {
      throw new Error(`updater archive entry is incompatible with Tauri macOS installation: ${entry}`);
    }
    seen.add(normalized);
  }
  const temporary = await mkdtemp(join(tmpdir(), 'nomifun-updater-verify-'));
  try {
    const extracted = join(temporary, prefix);
    await mkdir(extracted);
    await run('/usr/bin/tar', ['-xzf', archivePath, '-C', extracted, '--strip-components=1'], true,
      { ...process.env, COPYFILE_DISABLE: '1' });
    const inspected = await inspectMacosAppBundle(extracted);
    if (inspected.status !== 'pass') {
      throw new Error(`updater archive contains an incomplete macOS application bundle: ${inspected.missing.map(item => item.label).join(', ')}`);
    }
    // Matching bytes, modes and symlinks includes every nested code signature;
    // metadata identity and archive member names alone cannot prove parity.
    if (JSON.stringify(await appTreeFingerprint(extracted)) !== JSON.stringify(await appTreeFingerprint(appPath))) {
      throw new Error('updater archive contents differ from the final application bundle');
    }
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

async function appTreeFingerprint(appPath) {
  const root = await realpath(appPath);
  const files = [];
  async function visit(directory) {
    for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      // Match the archive's established metadata exclusions; all deliverable
      // files, permissions and symlinks still require exact App parity.
      if (entry.name.startsWith('._') || entry.name === '__MACOSX') continue;
      const path = join(directory, entry.name);
      const name = relative(root, path);
      if (entry.isSymbolicLink()) {
        if (!isOwned(root, await realpath(path))) throw new Error(`application symlink escapes its bundle: ${name}`);
        files.push([name, 'symlink', await readlink(path)]);
      } else if (entry.isDirectory()) {
        files.push([name, 'directory']);
        await visit(path);
      } else if (entry.isFile()) {
        const digest = createHash('sha256');
        for await (const chunk of createReadStream(path)) digest.update(chunk);
        files.push([name, 'file', (await stat(path)).mode & 0o777, digest.digest('hex')]);
      } else {
        throw new Error(`application contains an unsupported file: ${name}`);
      }
    }
  }
  await visit(root);
  return files;
}

// Updater bytes must be derived from the same final, nested-signed App as DMG.
// Credentials remain in the signer child's environment, never its arguments.
export async function createMacosUpdaterArchive({ appPath, projectRoot, environment = process.env }) {
  const inspected = await inspectMacosAppBundle(appPath);
  if (inspected.status !== 'pass') throw new Error(`incomplete macOS application bundle: ${inspected.missing.map(item => item.label).join(', ')}`);
  if (!environment.TAURI_SIGNING_PRIVATE_KEY && !environment.TAURI_SIGNING_PRIVATE_KEY_PATH) throw new Error('updater signing requires TAURI_SIGNING_PRIVATE_KEY or TAURI_SIGNING_PRIVATE_KEY_PATH');
  const temporary = await mkdtemp(join(dirname(resolve(appPath)), '.nomifun-updater-'));
  const archive = `${resolve(appPath)}.tar.gz`;
  const staged = join(temporary, `${basename(appPath)}.tar.gz`);
  try {
    await run('/usr/bin/tar', ['-czf', staged, '--format=pax', '--no-xattrs',
      '--exclude=._*', '--exclude=__MACOSX',
      ...(process.platform === 'darwin' ? ['--options=gzip:compression-level=9'] : []),
      '-C', dirname(resolve(appPath)), basename(appPath)], true,
    { ...environment, COPYFILE_DISABLE: '1' });
    await verifyMacosUpdaterArchive(staged, appPath);
    await new Promise((accept, reject) => {
      const child = spawn('bun', ['x', 'tauri', 'signer', 'sign', staged], {
        cwd: resolve(projectRoot), stdio: ['ignore', 'ignore', 'pipe'],
        env: Object.fromEntries(['PATH', 'HOME', 'TMPDIR', 'LANG', 'TAURI_SIGNING_PRIVATE_KEY', 'TAURI_SIGNING_PRIVATE_KEY_PATH', 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD']
          .filter(key => environment[key] !== undefined).map(key => [key, environment[key]])),
      });
      // Do not echo signer diagnostics: malformed key errors could contain input.
      child.stderr.resume();
      child.once('error', reject);
      child.once('close', code => code === 0 ? accept() : reject(new Error(`updater signer exited ${code}`)));
    });
    await requireFile(`${staged}.sig`, 'updater signature');
    await rename(staged, archive);
    await rename(`${staged}.sig`, `${archive}.sig`);
    return { archive, signature: `${archive}.sig` };
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

async function requireFile(path, label) {
  const descriptor = await open(path, 'r').catch(() => null);
  if (!descriptor) throw new Error(`${label} is missing: ${path}`);
  try {
    if (!(await descriptor.stat()).isFile()) throw new Error(`${label} must be a regular file: ${path}`);
  } finally { await descriptor.close(); }
}

async function signMachO(directory, sign) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) await signMachO(path, sign);
    else if (entry.isFile()) {
      const descriptor = await open(path, 'r');
      const bytes = Buffer.alloc(4);
      try { await descriptor.read(bytes, 0, 4, 0); } finally { await descriptor.close(); }
      if (['cffaedfe', 'cefaedfe', 'cafebabe'].includes(bytes.toString('hex'))) await sign(path);
    }
  }
}

// Sign only application-owned native code. WebKit and its WebContent processes
// remain system components, so this application needs no browser JIT entitlement.
export async function signMacosAppBundle({ appPath, identity = '-', target = process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin' }) {
  if (process.platform !== 'darwin') throw new Error('macOS application signing requires a macOS host');
  const architecture = { 'aarch64-apple-darwin': 'arm64', 'x86_64-apple-darwin': 'x86_64' }[target];
  if (!architecture) throw new Error(`unsupported macOS application target: ${target}`);
  if (typeof identity !== 'string' || !identity.trim() || /[\r\n\0]/.test(identity) || (identity.startsWith('-') && identity !== '-')) throw new Error('an explicit codesign identity or ad-hoc identity is required');
  const app = resolve(appPath);
  const inspected = await inspectMacosAppBundle(app);
  if (inspected.status !== 'pass') throw new Error(`incomplete macOS application bundle: ${inspected.missing.map(item => item.label).join(', ')}`);
  const host = join(app, 'Contents/MacOS/nomifun-desktop');
  const actual = await run('lipo', ['-archs', host], true);
  if (actual !== architecture) throw new Error(`expected ${architecture} application, received ${actual}`);
  const sign = path => run('codesign', ['--force',
    ...(identity === '-' ? [] : ['--options', 'runtime', '--timestamp']), '--sign', identity, path]);
  const frameworks = join(app, 'Contents/Frameworks');
  if (await stat(frameworks).catch(error => { if (error.code === 'ENOENT') return null; throw error; })) await signMachO(frameworks, sign);
  await run('/usr/bin/plutil', ['-convert', 'xml1', join(app, 'Contents/Info.plist')]);
  await sign(app);
  await run('codesign', ['--verify', '--deep', '--strict', app]);
  return { status: 'pass', app, identity: identity === '-' ? 'adhoc' : 'provided', architecture, browser: 'system-wkwebview' };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const option = (key, fallback) => {
    const index = args.indexOf(key);
    if (index < 0) {
      if (fallback !== undefined) return fallback;
      throw new Error(`${key} requires a value`);
    }
    if (!args[index + 1] || args[index + 1].startsWith('--')) throw new Error(`${key} requires a value`);
    return args[index + 1];
  };
  try {
    switch (args[0]) {
      case 'inspect': {
        const result = await inspectMacosAppBundle(option('--app'));
        if (result.status !== 'pass') throw new Error(result.missing.map(item => item.label).join(', '));
        process.stdout.write(`${JSON.stringify(result)}\n`);
        break;
      }
      case 'sign':
        process.stdout.write(`${JSON.stringify(await signMacosAppBundle({ appPath: option('--app'), identity: option('--identity', '-'), target: option('--target', process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin') }))}\n`);
        break;
      case 'build-settings': {
        const separator = args.indexOf('--');
        const settings = await resolveMacosBuildSettings(separator < 0 ? [] : args.slice(separator + 1), { root: option('--root') });
        process.stdout.write(`${settings.createUpdaterArtifacts ? 'true' : 'false'}\n`);
        break;
      }
      case 'updater':
        process.stdout.write(`${JSON.stringify(await createMacosUpdaterArchive({ appPath: option('--app'), projectRoot: option('--root') }))}\n`);
        break;
      default: throw new Error('expected inspect, sign, build-settings, or updater');
    }
  } catch (error) {
    process.stderr.write(`MACOS_APP_BUNDLE_FAIL ${error.message}\n`);
    process.exitCode = 1;
  }
}
