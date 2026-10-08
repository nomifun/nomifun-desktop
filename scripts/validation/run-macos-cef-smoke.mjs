#!/usr/bin/env node
// A real Tauri window with a native CEF child. Never a Windows skip/pass.
import { spawn } from 'node:child_process';
import { cp, mkdir, mkdtemp, readFile, readdir, writeFile, open, rename } from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import { createHash } from 'node:crypto';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { compileBrowserEnvironment, macosBrowserRuntime } from '../lib/macos-browser-bundle.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const args = process.argv.slice(2);
const option = (key, fallback) => args.includes(key) ? args[args.indexOf(key) + 1] : fallback;
const target = option('--target', null);
const contract = macosBrowserRuntime(target ?? (process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin'));
const targetArgs = target ? ['--target', target] : [];
const buildOutput = join(root, 'target', ...(target ? [target] : []), 'debug');
const soakOnly = args.includes('--soak-only');
const windowReopenOnly = args.includes('--window-reopen-only');
const contextShutdownOnly = args.includes('--context-shutdown-only');
const sqliteCompatibilityOnly = args.includes('--sqlite-compatibility-only');
const coldNavigationOnly = args.includes('--cold-navigation-only');
const coldNavigationProfile = option('--cold-navigation-profile', 'persistent');
if (!['persistent', 'ephemeral'].includes(coldNavigationProfile) || (args.includes('--cold-navigation-profile') && !coldNavigationOnly)) throw new Error('cold navigation profile requires the focused mode and must be persistent or ephemeral');
if ([soakOnly, windowReopenOnly, contextShutdownOnly, sqliteCompatibilityOnly, coldNavigationOnly].filter(Boolean).length > 1) throw new Error('choose only one native CEF focused mode');
let environment = Object.fromEntries(['PATH', 'HOME', 'TMPDIR', 'LANG', 'DEVELOPER_DIR', 'CARGO_HOME', 'RUSTUP_HOME', 'RUST_MIN_STACK'].filter(key => process.env[key]).map(key => [key, process.env[key]]));
const run = (command, argv, capture = false) => new Promise((resolve, reject) => {
  const child = spawn(command, argv, { cwd: root, env: environment, stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit' });
  let output = '';
  if (capture) { child.stdout.on('data', data => { output += data; }); child.stderr.on('data', data => { output += data; }); }
  child.once('error', reject);
  child.once('exit', code => code === 0 ? resolve(output.trim()) : reject(new Error(`${command} exited ${code}${capture ? `: ${output}` : ''}`)));
});
const hash = async path => {
  const digest = createHash('sha256');
  for await (const chunk of createReadStream(path)) digest.update(chunk);
  return digest.digest('hex');
};
const plist = object => '<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>' + Object.entries(object).map(([key,value]) => `<key>${key}</key>${typeof value === 'boolean' ? `<${value}/>` : `<string>${value}</string>`}`).join('') + '</dict></plist>';

try {
  if (process.platform !== 'darwin') throw new Error('macOS native CEF is required; not a passing test');
  const output = resolve(option('--output', join(root, 'dist/browser-cef-native-smoke')));
  const identity = option('--identity', '-');
  await mkdir(output, { recursive: true });
  const compiled = await compileBrowserEnvironment({ root, target, environment });
  environment = compiled.environment;
  await run('cargo', ['build', '-p', 'nomifun-desktop', '--example', 'browser_cef_smoke', '--no-default-features', ...targetArgs]);
  await run('cargo', ['build', '-p', 'nomifun-browser-macos', '--bin', 'nomifun-browser-cef-helper', ...targetArgs]);
  const helper = join(buildOutput, 'nomifun-browser-cef-helper');
  const runtime = compiled.runtimePath ?? await run(helper, ['--print-runtime-path'], true);
  const archive = JSON.parse(await readFile(join(runtime, 'archive.json'), 'utf8'));
  if (archive.name !== contract.archive || archive.sha1 !== contract.archive_sha1) throw new Error('native fixture requires the pinned CEF archive');
  delete environment.CEF_PATH;
  delete environment.FLATPAK;
  const stage = await mkdtemp(join(output, 'run-'));
  const app = join(stage, 'NomiCEFSmoke.app');
  const contents = join(app, 'Contents');
  const framework = join(contents, 'Frameworks/Chromium Embedded Framework.framework');
  const helperNames = ['', ' (GPU)', ' (Renderer)', ' (Plugin)', ' (Alerts)'].map(suffix => `NomiCEFSmoke Helper${suffix}`);
  const executable = join(contents, 'MacOS/browser_cef_smoke');
  await mkdir(join(contents, 'MacOS'), { recursive: true });
  // Copy into a fresh bundle. Do not sign hard-linked build-cache executables.
  await cp(join(buildOutput, 'examples/browser_cef_smoke'), executable);
  await cp(join(runtime, 'Chromium Embedded Framework.framework'), framework, { recursive: true, dereference: false, verbatimSymlinks: true });
  const common = {
    CFBundlePackageType: 'APPL',
    CFBundleVersion: '1',
    CFBundleShortVersionString: '1.0',
    LSMinimumSystemVersion: '14.0',
    NSHighResolutionCapable: true,
    NSMicrophoneUsageDescription: 'NomiFun lets websites use the microphone only after you explicitly allow the request in the built-in browser.',
    NSCameraUsageDescription: 'NomiFun lets websites use the camera only after you explicitly allow the request in the built-in browser.',
    NSLocationUsageDescription: 'NomiFun shares your location with a website only after you explicitly allow the request in the built-in browser.',
    NSLocalNetworkUsageDescription: 'NomiFun connects to local websites and services only when you ask it to.',
  };
  // The Tauri build embeds these two keys in the Mach-O __info_plist section.
  // macOS process-requirement validation rejects the signed process when the
  // external bundle plist disagrees with the embedded values (OSStatus -67030).
  await writeFile(join(contents, 'Info.plist'), plist({
    ...common,
    CFBundleIdentifier: 'com.nomifun.cef-native-smoke',
    CFBundleName: 'NomiFun',
    CFBundleExecutable: 'browser_cef_smoke',
  }));
  for (const name of helperNames) {
    const helperApp = join(contents, 'Frameworks', `${name}.app`);
    await mkdir(join(helperApp, 'Contents/MacOS'), { recursive: true });
    await cp(helper, join(helperApp, 'Contents/MacOS', name));
    const suffix = name.replace('NomiCEFSmoke Helper', '').replace(/[^A-Za-z]/g, '').toLowerCase();
    await writeFile(join(helperApp, 'Contents/Info.plist'), plist({ ...common, CFBundleIdentifier: `com.nomifun.cef-native-smoke.helper${suffix ? `.${suffix}` : ''}`, CFBundleName: name, CFBundleExecutable: name, LSUIElement: true }));
  }
  await run('/usr/bin/plutil', ['-convert', 'xml1', join(contents, 'Info.plist')]);
  for (const name of helperNames) {
    await run('/usr/bin/plutil', ['-convert', 'xml1', join(contents, 'Frameworks', `${name}.app`, 'Contents/Info.plist')]);
  }
  const entitlements = join(stage, 'helper-entitlements.plist');
  await writeFile(entitlements, plist({ 'com.apple.security.cs.allow-jit': true }));
  // Hardened runtime enforces library validation. A Developer ID build signs
  // every nested component with one Team ID and may enable it. An ad-hoc local
  // fixture has no Team ID, so enabling hardened runtime would make macOS
  // reject its equally ad-hoc CEF framework even though both seals verify.
  // Do not weaken library validation with an entitlement; simply keep the
  // unsigned engineering fixture non-hardened.
  const sign = (path, jit = false) => run('codesign', [
    '--force',
    ...(identity === '-' ? [] : ['--options', 'runtime']),
    '--sign', identity,
    ...(jit ? ['--entitlements', entitlements] : []),
    path,
  ]);
  async function signMachO(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) await signMachO(path);
      else if (entry.isFile()) {
        const fd = await open(path, 'r');
        const bytes = Buffer.alloc(4);
        try { await fd.read(bytes, 0, 4, 0); } finally { await fd.close(); }
        if (['cffaedfe', 'cefaedfe', 'cafebabe'].includes(bytes.toString('hex'))) await sign(path);
      }
    }
  }
  await signMachO(framework);
  await sign(framework);
  for (const name of helperNames) await sign(join(contents, 'Frameworks', `${name}.app`), true);
  await sign(app);
  // Execute fresh inodes, not signing-tool staging aliases retained by macOS.
  const sealed = join(stage, 'sealed-source.app');
  await rename(app, sealed);
  await cp(sealed, app, { recursive: true, dereference: false, verbatimSymlinks: true });
  await run('codesign', ['--verify', '--deep', '--strict', app]);
  const report = join(stage, 'native-result.json');
  const helpers = [];
  for (const name of helperNames) helpers.push({ name, sha256: await hash(join(contents, 'Frameworks', `${name}.app`, 'Contents/MacOS', name)) });
  const receipt = { scope: coldNavigationOnly ? 'product-host-cold-navigation' : sqliteCompatibilityOnly ? 'early-cef-library-sqlite-compatibility' : contextShutdownOnly ? 'tauri-native-cef-context-shutdown' : soakOnly ? 'tauri-native-cef-100-cycle-soak' : windowReopenOnly ? 'tauri-native-cef-window-reopen' : 'tauri-native-cef-input', productAcceptance: false, app, report, executableSha256: await hash(executable), helpers, architecture: await run('lipo', ['-archs', executable], true), infoPlistSerialization: 'canonical-xml' };
  await writeFile(join(stage, 'artifact.json'), JSON.stringify(receipt, null, 2));
  const launchArgs = ['-W', '-n', '--env', `NOMIFUN_CEF_REPORT=${report}`];
  if (soakOnly) launchArgs.push('--env', 'NOMIFUN_CEF_SOAK_ONLY=1');
  if (windowReopenOnly) launchArgs.push('--env', 'NOMIFUN_CEF_WINDOW_REOPEN_ONLY=1');
  if (contextShutdownOnly) launchArgs.push('--env', 'NOMIFUN_CEF_CONTEXT_SHUTDOWN_ONLY=1');
  if (sqliteCompatibilityOnly) launchArgs.push('--env', 'NOMIFUN_CEF_SQLITE_COMPATIBILITY_ONLY=1');
  if (coldNavigationOnly) launchArgs.push('--env', 'NOMIFUN_CEF_COLD_NAVIGATION_ONLY=1', '--env', `NOMIFUN_CEF_COLD_NAVIGATION_PROFILE=${coldNavigationProfile}`);
  if (process.env.NOMIFUN_CEF_PROTOCOL_TRACE) launchArgs.push('--env', 'NOMIFUN_CEF_PROTOCOL_TRACE=1');
  launchArgs.push('-o', join(stage, 'stdout.log'), '--stderr', join(stage, 'stderr.log'), app);
  // The allocator/SQLite branch creates no NSApplication or window. Execute
  // its same packaged binary directly so LaunchServices and screen lock do not
  // become accidental prerequisites for this headless native compatibility test.
  const logs = sqliteCompatibilityOnly
    ? await Promise.all([open(join(stage, 'stdout.log'), 'w'), open(join(stage, 'stderr.log'), 'w')])
    : [];
  const launch = sqliteCompatibilityOnly
    ? spawn(executable, [], {
      cwd: root,
      env: { ...environment, NOMIFUN_CEF_REPORT: report, NOMIFUN_CEF_SQLITE_COMPATIBILITY_ONLY: '1' },
      stdio: ['ignore', logs[0].fd, logs[1].fd],
    })
    : spawn('open', launchArgs, { cwd: root, env: environment, stdio: 'inherit' });
  const exited = new Promise((resolve, reject) => {
    launch.once('error', reject);
    launch.once('exit', (code, signal) => code === 0 ? resolve() : reject(new Error(`Native CEF fixture exited ${code ?? signal}`)));
  });
  let timer;
  try {
    await Promise.race([exited, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('Native CEF fixture timed out')), 260_000); })]);
    const result = JSON.parse(await readFile(report, 'utf8'));
    if (!result.checks || !Object.keys(result.checks).length || result.passed !== true || result.shutdown_complete !== true) throw new Error(`Native CEF conformance failed; inspect ${report}`);
    if (sqliteCompatibilityOnly && (result.cef_initialized !== false || result.helper_processes_started !== 0 || result.sqlite_workers !== 8 || result.schema_queries !== 40000)) {
      throw new Error(`CEF/SQLite fixture exceeded or did not complete its intended scope; inspect ${report}`);
    }
    console.log(JSON.stringify({ ...receipt, result }, null, 2));
  } finally {
    clearTimeout(timer);
    await Promise.all(logs.map(log => log.close()));
    // A timed-out launcher is not proof that the native app exited.
    const pid = Number(await readFile(join(stage, 'native-result.pid'), 'utf8').catch(() => ''));
    if (Number.isInteger(pid) && pid > 1) {
      const actual = await run('ps', ['-p', String(pid), '-o', 'comm='], true).catch(() => '');
      if (actual === executable) {
        process.kill(pid, 'SIGKILL');
        throw new Error('The owned CEF fixture required forced cleanup; not a passing shutdown');
      }
    }
  }
} catch (error) {
  console.error(`CEF_SMOKE_FAIL ${error.message}`);
  process.exitCode = 1;
}
