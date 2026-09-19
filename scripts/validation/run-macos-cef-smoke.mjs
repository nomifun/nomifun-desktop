#!/usr/bin/env node
// A real Tauri window with a native CEF child. Never a Windows skip/pass.
import { spawn } from 'node:child_process';
import { cp, mkdir, mkdtemp, readFile, readdir, writeFile, open, rename } from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import { createHash } from 'node:crypto';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const args = process.argv.slice(2);
const option = (key, fallback) => args.includes(key) ? args[args.indexOf(key) + 1] : fallback;
const environment = Object.fromEntries(['PATH', 'HOME', 'TMPDIR', 'LANG', 'DEVELOPER_DIR', 'CEF_PATH', 'CARGO_HOME', 'RUSTUP_HOME', 'RUST_MIN_STACK'].filter(key => process.env[key]).map(key => [key, process.env[key]]));
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
  await run('cargo', ['build', '-p', 'nomifun-desktop', '--example', 'browser_cef_smoke', '--no-default-features']);
  await run('cargo', ['build', '-p', 'nomifun-browser-macos', '--bin', 'nomifun-browser-cef-helper']);
  const helper = join(root, 'target/debug/nomifun-browser-cef-helper');
  const runtime = await run(helper, ['--print-runtime-path'], true);
  const stage = await mkdtemp(join(output, 'run-'));
  const app = join(stage, 'NomiCEFSmoke.app');
  const contents = join(app, 'Contents');
  const framework = join(contents, 'Frameworks/Chromium Embedded Framework.framework');
  const helperNames = ['', ' (GPU)', ' (Renderer)', ' (Plugin)', ' (Alerts)'].map(suffix => `NomiCEFSmoke Helper${suffix}`);
  const executable = join(contents, 'MacOS/browser_cef_smoke');
  await mkdir(join(contents, 'MacOS'), { recursive: true });
  // Copy into a fresh bundle. Do not sign hard-linked build-cache executables.
  await cp(join(root, 'target/debug/examples/browser_cef_smoke'), executable);
  await cp(join(runtime, 'Chromium Embedded Framework.framework'), framework, { recursive: true, dereference: false, verbatimSymlinks: true });
  const common = { CFBundlePackageType: 'APPL', CFBundleVersion: '1', CFBundleShortVersionString: '1.0', LSMinimumSystemVersion: '14.0', NSHighResolutionCapable: true };
  // The Tauri build embeds these two keys in the Mach-O __info_plist section.
  // macOS process-requirement validation rejects the signed process when the
  // external bundle plist disagrees with the embedded values (OSStatus -67030).
  await writeFile(join(contents, 'Info.plist'), plist({
    ...common,
    CFBundleIdentifier: 'com.nomifun.cef-native-smoke',
    CFBundleName: 'NomiFun',
    CFBundleExecutable: 'browser_cef_smoke',
    NSMicrophoneUsageDescription: 'NomiFun uses the microphone to record voice input and convert it to text with your selected speech recognition model.',
  }));
  for (const name of helperNames) {
    const helperApp = join(contents, 'Frameworks', `${name}.app`);
    await mkdir(join(helperApp, 'Contents/MacOS'), { recursive: true });
    await cp(helper, join(helperApp, 'Contents/MacOS', name));
    const suffix = name.replace('NomiCEFSmoke Helper', '').replace(/[^A-Za-z]/g, '').toLowerCase();
    await writeFile(join(helperApp, 'Contents/Info.plist'), plist({ ...common, CFBundleIdentifier: `com.nomifun.cef-native-smoke.helper${suffix ? `.${suffix}` : ''}`, CFBundleName: name, CFBundleExecutable: name, LSUIElement: true }));
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
  const receipt = { scope: 'tauri-native-cef-input', productAcceptance: false, app, report, executableSha256: await hash(executable), helpers, architecture: await run('lipo', ['-archs', executable], true) };
  await writeFile(join(stage, 'artifact.json'), JSON.stringify(receipt, null, 2));
  const launch = spawn('open', ['-W', '-n', '--env', `NOMIFUN_CEF_REPORT=${report}`, '-o', join(stage, 'stdout.log'), '--stderr', join(stage, 'stderr.log'), app], { cwd: root, env: environment, stdio: 'inherit' });
  const exited = new Promise((resolve, reject) => { launch.once('error', reject); launch.once('exit', resolve); });
  let timer;
  try {
    await Promise.race([exited, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('Native CEF fixture timed out')), 260_000); })]);
    const result = JSON.parse(await readFile(report, 'utf8'));
    if (!result.checks || !Object.keys(result.checks).length || result.passed !== true || result.shutdown_complete !== true) throw new Error(`Native CEF conformance failed; inspect ${report}`);
    console.log(JSON.stringify({ ...receipt, result }, null, 2));
  } finally {
    clearTimeout(timer);
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
