// A disposable real Mach-O signing regression. This proves signatures and
// entitlements, not TCC grants or administrator-service approval.
import { spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { signMacosAppBundle } from '../lib/macos-app-bundle.mjs';
import { macosDevelopmentSealArguments } from '../lib/macos-dev-signing.mjs';

export async function verifyMacosHostSigningFixture() {
  if (process.platform !== 'darwin') throw new Error('This real Mach-O signing fixture requires macOS');
  const root = await mkdtemp(join(tmpdir(), 'nomifun-host-signing-'));
  const run = (program, args) => {
    const result = spawnSync(program, args, { encoding: 'utf8', timeout: 15_000 });
    if (result.status !== 0) throw new Error(`${program} failed (${result.status ?? result.signal}): ${result.stderr || result.stdout || result.error}`);
    return result.stdout;
  };
  let passed = false;
  try {
    const app = join(root, 'Fixture.app');
    const program = join(app, 'Contents/MacOS/nomifun-desktop');
    await mkdir(join(app, 'Contents/MacOS'), { recursive: true });
    const source = join(root, 'fixture.c');
    await writeFile(source, 'int main(void) { return 0; }\n');
    const architecture = process.arch === 'arm64' ? 'arm64' : 'x86_64';
    run('/usr/bin/clang', ['-arch', architecture, '-o', program, source]);
    await writeFile(join(app, 'Contents/Info.plist'), '<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.nomifun.host-signing-fixture</string><key>CFBundleExecutable</key><string>nomifun-desktop</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>');
    await signMacosAppBundle({ appPath: app, target: architecture === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin' });
    const inspect = () => {
      run('/usr/bin/codesign', ['--verify', '--deep', '--strict', app]);
      const dump = join(root, 'entitlements.plist');
      run('/usr/bin/codesign', ['--display', '--entitlements', dump, '--xml', app]);
      const entitlements = JSON.parse(run('/usr/bin/plutil', ['-convert', 'json', '-o', '-', dump]));
      for (const key of ['com.apple.security.device.audio-input', 'com.apple.security.device.camera', 'com.apple.security.automation.apple-events']) {
        if (entitlements[key] !== true) throw new Error(`Actual Mach-O signature lacks ${key}`);
      }
      return entitlements;
    };
    const initial = inspect();
    run('/usr/bin/codesign', macosDevelopmentSealArguments(app, '-'));
    const resealed = inspect();
    passed = true;
    return { passed: true, architecture, initial, resealed };
  } catch (error) {
    throw new Error(`${error.message}; failed native fixture retained at ${root}`, { cause: error });
  } finally {
    if (passed) await rm(root, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const args = process.argv.slice(2);
    if (args.length && (args.length !== 2 || args[0] !== '--report')) throw new Error('Expected optional --report /ABS/NEW/report.json');
    const report = JSON.stringify(await verifyMacosHostSigningFixture());
    if (args.length) await writeFile(resolve(args[1]), `${report}\n`, { flag: 'wx' });
    console.log(report);
  }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
