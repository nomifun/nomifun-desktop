// The system service is part of the same signed App as its authenticated XPC
// client. Build it after Cargo returns; never invoke Cargo from a build script.
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmod, copyFile, mkdir, mkdtemp, readFile, readdir, rename, rm, stat, writeFile } from 'node:fs/promises';
import { dirname, join, resolve, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const SYSTEM_EXECUTION_IDENTIFIER = 'com.nomifun.desktop.system-execution';
export const SYSTEM_EXECUTION_HELPER = 'Library/HelperTools/nomifun-system-execution';
export const SYSTEM_EXECUTION_PLIST = `Library/LaunchDaemons/${SYSTEM_EXECUTION_IDENTIFIER}.plist`;
export function systemExecutionServiceIdentifier(bundleIdentifier = 'com.nomifun.desktop') {
  if (!['com.nomifun.desktop', 'com.nomifun.desktop.dev'].includes(bundleIdentifier)) throw new Error(`native administrator execution has no supported application identity: ${bundleIdentifier}`);
  return `${bundleIdentifier}.system-execution`;
}

export function systemExecutionBuildPlan({ root = ROOT, target, profile = 'debug' }) {
  const architecture = { 'aarch64-apple-darwin': 'arm64', 'x86_64-apple-darwin': 'x86_64' }[target];
  if (!architecture) throw new Error(`unsupported native system execution target: ${target}`);
  const cargoProfile = profile === 'debug' ? 'dev' : profile;
  const sources = join(root, 'apps/desktop/macos-system-execution');
  return {
    cargo: ['build', '-p', 'nomi-system-execution', '--target', target, '--profile', cargoProfile, '--message-format=json-render-diagnostics'],
    sources,
    swift: ['swiftc', '-parse-as-library', '-swift-version', '5', '-O', '-module-name', 'NomiFunSystemExecutionHelper', '-target', `${architecture}-apple-macosx14.0`, join(sources, 'Protocol.swift'), join(sources, 'Helper.swift')],
  };
}

const run = (program, args, { root = ROOT, environment = process.env, capture = false } = {}) => new Promise((accept, reject) => {
  const child = spawn(program, args, { cwd: root, env: environment, stdio: capture ? ['ignore', 'pipe', 'inherit'] : 'inherit' });
  let output = '';
  if (capture) child.stdout.on('data', bytes => { output += bytes; });
  child.once('error', reject);
  child.once('close', code => code === 0 ? accept(output) : reject(new Error(`${program} exited ${code}`)));
});

export async function buildMacosSystemExecution({ root = ROOT, target, profile = 'debug', bundleIdentifier = 'com.nomifun.desktop', environment = process.env }, { execute = run } = {}) {
  if (process.platform !== 'darwin') throw new Error('the native macOS system execution service requires a macOS build host');
  const plan = systemExecutionBuildPlan({ root, target, profile });
  const serviceIdentifier = systemExecutionServiceIdentifier(bundleIdentifier);
  const plistDestination = `Library/LaunchDaemons/${serviceIdentifier}.plist`;
  const sourceFingerprint = async () => {
    const hash = createHash('sha256');
    const file = async path => {
      const name = Buffer.from(relative(root, path));
      const bytes = await readFile(path);
      const length = Buffer.alloc(8);
      length.writeBigUInt64LE(BigInt(name.length)); hash.update(length); hash.update(name);
      length.writeBigUInt64LE(BigInt(bytes.length)); hash.update(length); hash.update(bytes);
    };
    const visit = async directory => {
      for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) await visit(path);
        else if (/\.(rs|swift|toml|plist)$/.test(entry.name)) await file(path);
      }
    };
    for (const path of ['Cargo.toml', 'Cargo.lock', 'crates/shared/nomi-system-execution/Cargo.toml', 'crates/shared/nomi-process-runtime/Cargo.toml', 'crates/shared/nomi-native-files/Cargo.toml']) await file(join(root,path));
    for (const directory of [plan.sources, join(root, 'crates/shared/nomi-system-execution/src'), join(root, 'crates/shared/nomi-process-runtime/src'), join(root, 'crates/shared/nomi-native-files/src')]) await visit(directory);
    // A checkout-specific injected test root still fingerprints the actual
    // packaging implementation, without treating its path as build identity.
    hash.update(await readFile(fileURLToPath(import.meta.url)));
    return hash.digest('hex');
  };
  const sourceBefore = await sourceFingerprint();
  const reports = await execute('cargo', plan.cargo, { root, environment, capture: true });
  if (sourceBefore !== await sourceFingerprint()) throw new Error('native system execution sources changed during compilation; rebuild the complete App from stable sources');
  let library;
  for (const line of reports.split('\n')) {
    let report; try { report = JSON.parse(line); } catch { continue; }
    if (report.reason === 'compiler-artifact' && report.target?.name === 'nomi_system_execution') {
      library = report.filenames?.find(path => path.endsWith('.a')) ?? library;
    }
  }
  if (!library || !(await stat(library)).isFile()) throw new Error('Cargo did not identify the exact native system execution static library');
  const libraryBytes = await readFile(library);
  const digest = createHash('sha256').update(target).update(profile).update(serviceIdentifier).update(sourceBefore).update(libraryBytes);
  for (const name of ['Protocol.swift', 'Helper.swift', `${SYSTEM_EXECUTION_IDENTIFIER}.plist`]) digest.update(await readFile(join(plan.sources, name)));
  digest.update(await readFile(fileURLToPath(import.meta.url)));
  const cache = join(root, 'target/macos-system-execution', target, profile, digest.digest('hex'));
  const executable = join(cache, 'nomifun-system-execution');
  const plist = join(cache, `${serviceIdentifier}.plist`);
  const fingerprint = async path => createHash('sha256').update(await readFile(path)).digest('hex');
  try {
    const receipt = JSON.parse(await readFile(join(cache, 'complete.json'), 'utf8'));
    if (receipt.executable === await fingerprint(executable) && receipt.plist === await fingerprint(plist)) {
      return { [SYSTEM_EXECUTION_HELPER]: executable, [plistDestination]: plist };
    }
  } catch { /* A partial or changed owned cache is rebuilt and verified. */ }
  await rm(cache, { recursive: true, force: true });
  await mkdir(dirname(cache), { recursive: true });
  const temporary = await mkdtemp(`${cache}.preparing-`);
  try {
    const output = join(temporary, 'nomifun-system-execution');
    const frozenLibrary = join(temporary, 'libnomi_system_execution.a');
    await writeFile(frozenLibrary, libraryBytes);
    await execute('xcrun', [...plan.swift, frozenLibrary,
      '-Xlinker', '-dead_strip', '-framework', 'Foundation', '-framework', 'Security', '-framework', 'ServiceManagement', '-framework', 'AppKit', '-liconv', '-lresolv', '-o', output], { root, environment });
    if (sourceBefore !== await sourceFingerprint()) throw new Error('native system execution sources changed during native linking; rebuild the complete App from stable sources');
    await rm(frozenLibrary);
    await chmod(output, 0o755);
    const plistSource = await readFile(join(plan.sources, `${SYSTEM_EXECUTION_IDENTIFIER}.plist`), 'utf8');
    await writeFile(join(temporary, `${serviceIdentifier}.plist`), plistSource.replaceAll(SYSTEM_EXECUTION_IDENTIFIER, serviceIdentifier));
    await writeFile(join(temporary, 'complete.json'), `${JSON.stringify({ executable: await fingerprint(output), plist: await fingerprint(join(temporary, `${serviceIdentifier}.plist`)) })}\n`);
    try { await rename(temporary, cache); } catch (error) { if (error.code !== 'EEXIST' && error.code !== 'ENOTEMPTY') throw error; }
  } finally { await rm(temporary, { recursive: true, force: true }); }
  return { [SYSTEM_EXECUTION_HELPER]: executable, [plistDestination]: plist };
}

export async function installMacosSystemExecution({ appPath, ...build }, options = {}) {
  if (!build.bundleIdentifier) {
    const plist = await (options.execute ?? run)('/usr/bin/plutil', ['-convert', 'json', '-o', '-', join(appPath, 'Contents/Info.plist')], { root: build.root ?? ROOT, environment: build.environment ?? process.env, capture: true });
    build.bundleIdentifier = JSON.parse(plist).CFBundleIdentifier;
  }
  const files = await buildMacosSystemExecution(build, options);
  for (const [destination, source] of Object.entries(files)) {
    const target = join(appPath, 'Contents', destination);
    await mkdir(dirname(target), { recursive: true });
    await copyFile(source, target);
    await chmod(target, destination === SYSTEM_EXECUTION_HELPER ? 0o755 : 0o644);
  }
  return files;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const option = (name, fallback) => { const index = args.indexOf(name); if (index < 0 && fallback !== undefined) return fallback; if (index < 0 || !args[index + 1]) throw new Error(`${name} requires a value`); return args[index + 1]; };
  try {
    const result = await installMacosSystemExecution({ root: option('--root', ROOT), target: option('--target'), profile: option('--profile', 'release'), appPath: option('--app') });
    process.stdout.write(`${JSON.stringify(result)}\n`);
  } catch (error) { process.stderr.write(`MACOS_SYSTEM_EXECUTION_FAIL ${error.message}\n`); process.exitCode = 1; }
}
