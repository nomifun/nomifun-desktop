// Shared staging of the pinned CEF runtime into an already-built NomiFun.app.
// never discovers an arbitrary app/helper through PATH and never signs with a
// credential value; callers provide an identity name/hash or '-' for ad-hoc.

import { spawn } from 'node:child_process';
import {
  cp,
  mkdir,
  mkdtemp,
  open,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stat } from 'node:fs/promises';
import { readFileSync } from 'node:fs';

export const MACOS_BROWSER_RUNTIME = Object.freeze(JSON.parse(readFileSync(
  new URL('../../apps/desktop/browser-runtime.json', import.meta.url), 'utf8',
)));
export const MACOS_INTEL_BROWSER_RUNTIME = Object.freeze(JSON.parse(readFileSync(
  new URL('../../apps/desktop/browser-runtime-intel.json', import.meta.url), 'utf8',
)));
const hostTarget = () => process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
export function macosBrowserRuntime(target = hostTarget()) {
  if (target === 'aarch64-apple-darwin') return MACOS_BROWSER_RUNTIME;
  if (target === 'x86_64-apple-darwin') return MACOS_INTEL_BROWSER_RUNTIME;
  throw new Error(`unsupported macOS CEF target: ${target}`);
}
export const MACOS_CEF_ARCHIVE = MACOS_BROWSER_RUNTIME.archive;
export const MACOS_CEF_ARCHIVE_SHA1 = MACOS_BROWSER_RUNTIME.archive_sha1;
export const MACOS_CEF_HELPER_NAMES = Object.freeze([...MACOS_BROWSER_RUNTIME.helpers]);
export const MACOS_CEF_REQUIRED_RESOURCES = Object.freeze([...MACOS_BROWSER_RUNTIME.resources]);

// Discover only the pinned cef-dll-sys Cargo outputs for this exact build.
// Host builds omit a target component; explicit --target builds include it.
export async function discoverCefRuntime({ root, target = null, profile = 'debug' }) {
  if (!['debug', 'release'].includes(profile)) throw new Error('CEF runtime discovery requires debug or release profile');
  const contract = macosBrowserRuntime(target ?? hostTarget());
  const runtimeDirectory = contract.architecture === 'arm64' ? 'cef_macos_aarch64' : 'cef_macos_x86_64';
  const candidates = [];
  for (const directory of ['build.noindex', 'target']) {
    const build = join(resolve(root), directory, ...(target ? [target] : []), profile, 'build');
    for (const entry of await readdir(build, { withFileTypes: true }).catch(error => {
      if (error.code === 'ENOENT') return [];
      throw error;
    })) {
      if (!entry.isDirectory() || !entry.name.startsWith('cef-dll-sys-')) continue;
      const runtime = join(build, entry.name, 'out', runtimeDirectory);
      const metadata = join(runtime, 'archive.json');
      try {
        const archive = JSON.parse(await readFile(metadata, 'utf8'));
        if (archive.name !== contract.archive || archive.sha1 !== contract.archive_sha1) continue;
        const framework = await stat(join(runtime, 'Chromium Embedded Framework.framework', 'Chromium Embedded Framework')).catch(error => {
          if (error.code === 'ENOENT') return null;
          throw error;
        });
        if (!framework?.isFile()) continue;
        candidates.push({ runtime, modified: (await stat(metadata)).mtimeMs });
      } catch (error) {
        if (error.code === 'ENOENT' || error instanceof SyntaxError) continue;
        throw error;
      }
    }
  }
  candidates.sort((left, right) => right.modified - left.modified || left.runtime.localeCompare(right.runtime));
  if (!candidates.length) throw new Error('Cargo did not produce the pinned macOS CEF runtime');
  return candidates[0].runtime;
}

export async function compileBrowserEnvironment({ root, target = null, profile = 'debug', environment = process.env }) {
  const compiled = { ...environment };
  // Only a Cargo output verified against our manifest may override downloading.
  // These overrides are for compilation; native launch always uses its bundle.
  delete compiled.CEF_PATH;
  delete compiled.FLATPAK;
  const requestedTarget = target ?? hostTarget();
  macosBrowserRuntime(requestedTarget);
  const profiles = [profile, profile === 'release' ? 'debug' : 'release'];
  const searches = profiles.flatMap(profile => [
    { target: requestedTarget, profile },
    ...(requestedTarget === hostTarget() ? [{ target: null, profile }] : []),
  ]);
  const visited = new Set();
  let runtimePath = null;
  for (const search of searches) {
    const key = `${search.target ?? 'host'}:${search.profile}`;
    if (visited.has(key)) continue;
    visited.add(key);
    try { runtimePath = await discoverCefRuntime({ root, ...search }); break; }
    catch (error) {
      if (error.message !== 'Cargo did not produce the pinned macOS CEF runtime') throw error;
    }
  }
  if (runtimePath) compiled.CEF_PATH = runtimePath;
  return { environment: compiled, runtimePath };
}

export async function inspectMacosBrowserBundle(appPath) {
  const contents = join(resolve(appPath), 'Contents');
  const ownedRoot = await realpath(appPath).catch(error => {
    if (error.code === 'ENOENT') return null;
    throw error;
  });
  const files = [
    ['host', join(contents, 'MacOS', 'nomifun-desktop'), true],
    ['Info.plist', join(contents, 'Info.plist'), false],
    ['CEF framework', join(contents, 'Frameworks', 'Chromium Embedded Framework.framework', 'Chromium Embedded Framework'), true],
    ['CEF runtime metadata', join(contents, 'Resources', 'browser-cef', 'runtime.json'), false],
    ['CEF credits', join(contents, 'Resources', 'browser-cef', 'CREDITS.html'), false],
    ...MACOS_CEF_REQUIRED_RESOURCES.map(name => [name, join(contents, 'Frameworks', 'Chromium Embedded Framework.framework', 'Resources', name), false]),
    ...MACOS_CEF_HELPER_NAMES.flatMap(name => [
      [name, join(contents, 'Frameworks', `${name}.app`, 'Contents', 'MacOS', name), true],
      [`${name} Info.plist`, join(contents, 'Frameworks', `${name}.app`, 'Contents', 'Info.plist'), false],
    ]),
  ];
  const missing = [];
  for (const [label, path, executable] of files) {
    const info = await stat(path).catch(error => {
      if (error.code === 'ENOENT') return null;
      throw error;
    });
    if (!info?.isFile() || (executable && !(info.mode & 0o111))) {
      missing.push({ label, path });
      continue;
    }
    const resolved = await realpath(path);
    const owned = ownedRoot && relative(ownedRoot, resolved);
    if (!owned || owned === '..' || owned.startsWith(`..${process.platform === 'win32' ? '\\' : '/'}`) || isAbsolute(owned)) {
      missing.push({ label: `${label} outside its application bundle`, path });
    }
  }
  if (!missing.length) {
    const metadata = await readFile(files[3][1], 'utf8').then(JSON.parse).catch(error => {
      if (error instanceof SyntaxError) return null;
      throw error;
    });
    const contract = metadata?.architecture === 'arm64' ? MACOS_BROWSER_RUNTIME
      : metadata?.architecture === 'x86_64' ? MACOS_INTEL_BROWSER_RUNTIME : null;
    if (!contract || ['cef', 'chromium', 'crate', 'architecture', 'archive', 'archive_sha1'].some(field => metadata?.[field] !== contract[field])) {
      missing.push({ label: 'pinned CEF runtime identity', path: files[3][1] });
    }
  }
  return { status: missing.length ? 'fail' : 'pass', appPath: resolve(appPath), missing };
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
  for (const entry of entries) {
    const parts = entry.replace(/\/$/, '').split('/');
    if (parts[0] !== prefix || (parts.length === 1 && !entry.endsWith('/')) ||
        parts.some(part => !part || part === '.' || part === '..' || part.startsWith('._') || part === '__MACOSX')) {
      throw new Error(`updater archive entry is incompatible with Tauri macOS installation: ${entry}`);
    }
  }

  const temporary = await mkdtemp(join(tmpdir(), 'nomifun-updater-verify-'));
  try {
    const extracted = join(temporary, prefix);
    await mkdir(extracted);
    await run('/usr/bin/tar', ['-xzf', archivePath, '-C', extracted, '--strip-components=1'], true);
    // Reuse the App contract after extraction instead of maintaining a second
    // list of required runtime files that only checks archive names.
    const inspected = await inspectMacosBrowserBundle(extracted);
    if (inspected.status !== 'pass') {
      throw new Error(`updater archive contains an incomplete macOS Browser bundle: ${inspected.missing.map(item => item.label).join(', ')}`);
    }
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

// Updater bytes must be derived from the same final, nested-signed App as DMG.
// Credentials remain in the signer child's environment, never its arguments.
export async function createMacosUpdaterArchive({ appPath, projectRoot, environment = process.env }) {
  const inspected = await inspectMacosBrowserBundle(appPath);
  if (inspected.status !== 'pass') throw new Error(`incomplete macOS Browser bundle: ${inspected.missing.map(item => item.label).join(', ')}`);
  if (!environment.TAURI_SIGNING_PRIVATE_KEY && !environment.TAURI_SIGNING_PRIVATE_KEY_PATH) throw new Error('updater signing requires TAURI_SIGNING_PRIVATE_KEY or TAURI_SIGNING_PRIVATE_KEY_PATH');
  const temporary = await mkdtemp(join(dirname(resolve(appPath)), '.nomifun-updater-'));
  const archive = `${resolve(appPath)}.tar.gz`;
  const staged = join(temporary, `${basename(appPath)}.tar.gz`);
  try {
    await run('/usr/bin/tar', ['-czf', staged, '--format=pax', '--no-xattrs',
      '--exclude=._*', '--exclude=__MACOSX', '-C', dirname(resolve(appPath)), basename(appPath)],
    true, { ...environment, COPYFILE_DISABLE: '1' });
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
      child.once('exit', code => code === 0 ? accept() : reject(new Error(`updater signer exited ${code}`)));
    });
    await requireFile(`${staged}.sig`, 'updater signature');
    await rename(staged, archive);
    await rename(`${staged}.sig`, `${archive}.sig`);
    return { archive, signature: `${archive}.sig` };
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

const plist = object => `<?xml version="1.0" encoding="UTF-8"?>\n` +
  `<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n` +
  `<plist version="1.0"><dict>${Object.entries(object).map(([key, value]) =>
    `<key>${key}</key>${typeof value === 'boolean' ? `<${value}/>` : `<string>${value}</string>`}`
  ).join('')}</dict></plist>\n`;

async function requireFile(path, label) {
  const descriptor = await open(path, 'r').catch(() => null);
  if (!descriptor) throw new Error(`${label} is missing: ${path}`);
  try {
    if (!(await descriptor.stat()).isFile()) throw new Error(`${label} must be a regular file: ${path}`);
  } finally { await descriptor.close(); }
}

// Chromium reconstructs the outer bundle's in-memory Info.plist as canonical
// XML when it validates sandboxed peer processes. The bytes in the code
// signature therefore need to use the same serialization; otherwise macOS
// reports errSecCSInfoPlistFailed even though a path-only codesign check passes.
const canonicalizeInfoPlist = path => run('/usr/bin/plutil', ['-convert', 'xml1', path]);

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

export async function stageMacosBrowserBundle({ appPath, helperPath, runtimePath, identity = '-', target = hostTarget() }) {
  const contract = macosBrowserRuntime(target);
  const app = resolve(appPath);
  const helper = resolve(helperPath);
  const runtime = resolve(runtimePath);
  const contents = join(app, 'Contents');
  const frameworks = join(contents, 'Frameworks');
  const frameworkSource = join(runtime, 'Chromium Embedded Framework.framework');
  const framework = join(frameworks, 'Chromium Embedded Framework.framework');
  const helperNames = MACOS_CEF_HELPER_NAMES;
  let temporary;
  try {
    if (process.platform !== 'darwin') {
      throw new Error('CEF product staging requires a macOS host');
    }
    await requireFile(join(contents, 'MacOS', 'nomifun-desktop'), 'NomiFun host');
    await requireFile(helper, 'CEF helper');
    await requireFile(join(frameworkSource, 'Chromium Embedded Framework'), 'CEF framework');
    for (const name of MACOS_CEF_REQUIRED_RESOURCES) await requireFile(join(frameworkSource, 'Resources', name), name);
    const archive = JSON.parse(await readFile(join(runtime, 'archive.json'), 'utf8'));
    if (archive.name !== contract.archive || archive.sha1 !== contract.archive_sha1) {
      throw new Error(`unexpected CEF archive: ${archive.name || 'unknown'}`);
    }
    const helperArch = await run('lipo', ['-archs', helper], true);
    const hostArch = await run('lipo', ['-archs', join(contents, 'MacOS', 'nomifun-desktop')], true);
    const frameworkArch = await run(
      'lipo', ['-archs', join(frameworkSource, 'Chromium Embedded Framework')], true,
    );
    if ([hostArch, helperArch, frameworkArch].some(arch => arch !== contract.architecture)) {
      throw new Error(`CEF ${contract.architecture} staging received host=${hostArch}, helper=${helperArch}, framework=${frameworkArch}`);
    }

    await mkdir(frameworks, { recursive: true });
    await rm(framework, { recursive: true, force: true });
    await cp(frameworkSource, framework, {
      recursive: true,
      dereference: false,
      verbatimSymlinks: true,
    });

    const common = {
      CFBundlePackageType: 'APPL',
      CFBundleVersion: '1',
      CFBundleShortVersionString: '1.0',
      LSMinimumSystemVersion: '14.0',
      NSHighResolutionCapable: true,
      LSUIElement: true,
      NSMicrophoneUsageDescription: 'NomiFun lets websites use the microphone only after you explicitly allow the request in the built-in browser.',
      NSCameraUsageDescription: 'NomiFun lets websites use the camera only after you explicitly allow the request in the built-in browser.',
      NSLocationUsageDescription: 'NomiFun shares your location with a website only after you explicitly allow the request in the built-in browser.',
      NSLocalNetworkUsageDescription: 'NomiFun connects to local websites and services only when you ask it to.',
    };
    for (const name of helperNames) {
      const helperApp = join(frameworks, `${name}.app`);
      await rm(helperApp, { recursive: true, force: true });
      await mkdir(join(helperApp, 'Contents/MacOS'), { recursive: true });
      await cp(helper, join(helperApp, 'Contents/MacOS', name));
      const suffix = name.replace('NomiFun Helper', '').replace(/[^A-Za-z]/g, '').toLowerCase();
      await writeFile(join(helperApp, 'Contents/Info.plist'), plist({
        ...common,
        CFBundleIdentifier: `com.nomifun.desktop.helper${suffix ? `.${suffix}` : ''}`,
        CFBundleName: name,
        CFBundleExecutable: name,
      }));
    }

    await canonicalizeInfoPlist(join(contents, 'Info.plist'));
    for (const name of helperNames) {
      await canonicalizeInfoPlist(join(frameworks, `${name}.app`, 'Contents/Info.plist'));
    }

    const legal = join(contents, 'Resources', 'browser-cef');
    await mkdir(legal, { recursive: true });
    await cp(join(runtime, 'CREDITS.html'), join(legal, 'CREDITS.html'));
    await writeFile(join(legal, 'runtime.json'), `${JSON.stringify({
      cef: contract.cef,
      chromium: contract.chromium,
      crate: contract.crate,
      archive: archive.name,
      archive_sha1: archive.sha1,
      architecture: contract.architecture,
    }, null, 2)}\n`);

    temporary = await mkdtemp(join(tmpdir(), 'nomifun-cef-sign-'));
    const entitlements = join(temporary, 'helper-entitlements.plist');
    await writeFile(entitlements, plist({ 'com.apple.security.cs.allow-jit': true }));
    const sign = (path, jit = false) => run('codesign', [
      '--force',
      ...(identity === '-' ? [] : ['--options', 'runtime', '--timestamp']),
      '--sign', identity,
      ...(jit ? ['--entitlements', entitlements] : []),
      path,
    ]);
    await signMachO(frameworks, sign);
    await sign(framework);
    for (const name of helperNames) await sign(join(frameworks, `${name}.app`), true);
    await sign(app);
    await run('codesign', ['--verify', '--deep', '--strict', app]);
    const inspected = await inspectMacosBrowserBundle(app);
    if (inspected.status !== 'pass') throw new Error(`CEF staged bundle is incomplete: ${inspected.missing.map(item => item.label).join(', ')}`);

    return {
      status: 'pass',
      app,
      identity: identity === '-' ? 'adhoc' : 'provided',
      cef: contract.cef,
      chromium: contract.chromium,
      architecture: contract.architecture,
      framework: `Contents/Frameworks/${basename(framework)}`,
      helpers: helperNames.map(name => `Contents/Frameworks/${name}.app`),
      credits: 'Contents/Resources/browser-cef/CREDITS.html',
      info_plist_serialization: 'canonical-xml',
    };
  } finally {
    if (temporary) await rm(temporary, { recursive: true, force: true });
  }
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
      case 'runtime':
        process.stdout.write(`${await discoverCefRuntime({ root: option('--root'), target: option('--target', null), profile: option('--profile', 'release') })}\n`);
        break;
      case 'compile-runtime': {
        const compiled = await compileBrowserEnvironment({ root: option('--root'), target: option('--target', null), profile: option('--profile', 'release') });
        process.stdout.write(`${compiled.runtimePath ?? ''}\n`);
        break;
      }
      case 'build-settings': {
        const separator = args.indexOf('--');
        const settings = await resolveMacosBuildSettings(separator < 0 ? [] : args.slice(separator + 1), { root: option('--root') });
        process.stdout.write(`${settings.createUpdaterArtifacts ? 'true' : 'false'}\n`);
        break;
      }
      case 'updater':
        process.stdout.write(`${JSON.stringify(await createMacosUpdaterArchive({ appPath: option('--app'), projectRoot: option('--root') }))}\n`);
        break;
      default: throw new Error('expected runtime, compile-runtime, build-settings, or updater');
    }
  } catch (error) {
    process.stderr.write(`MACOS_BROWSER_BUNDLE_FAIL ${error.message}\n`);
    process.exitCode = 1;
  }
}
