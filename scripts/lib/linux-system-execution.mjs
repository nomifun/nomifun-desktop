// Linux administrator execution always uses the installed, root-owned runner.
// Building only stages bytes; installation and native authentication are separate.
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmod, copyFile, lstat, mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const LINUX_SYSTEM_RUNNER = '/usr/libexec/nomifun-system-runner';
export const LINUX_SYSTEM_POLICY = '/usr/share/polkit-1/actions/com.nomifun.system-execution.policy';
const ASSET_NAMES = ['com.nomifun.system-execution.policy', 'install.sh', 'README.md'];
// Wayland Computer consumes the desktop portal's PipeWire stream through
// pipewiresrc -> videoconvert -> videoflip -> appsink. Plugins are runtime-loaded,
// so ELF dependency scanning alone cannot discover their package dependencies.
const DESKTOP_DEB_DEPENDENCIES = [
  'pkexec | policykit-1', 'polkitd | policykit-1', 'xdg-desktop-portal', 'util-linux', 'coreutils', 'libasound2t64 | libasound2', 'xwayland',
  'libgstreamer1.0-0 (>= 1.14)', 'libgstreamer-plugins-base1.0-0 (>= 1.14)',
  'gstreamer1.0-pipewire', 'gstreamer1.0-plugins-base', 'gstreamer1.0-plugins-good',
];
const DESKTOP_RPM_DEPENDENCIES = [
  'polkit', 'xdg-desktop-portal', 'util-linux', 'coreutils', 'alsa-lib', 'xorg-x11-server-Xwayland', 'gstreamer1', 'gstreamer1-plugins-base',
  'gstreamer1-plugins-good', 'pipewire-gstreamer',
];

export function resolveLinuxSystemTarget(target, architecture = process.arch) {
  const resolved = target ?? { x64: 'x86_64-unknown-linux-gnu', arm64: 'aarch64-unknown-linux-gnu' }[architecture];
  if (!['x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu'].includes(resolved)) {
    throw new Error(`unsupported Linux system execution target: ${resolved ?? architecture}`);
  }
  return resolved;
}

export function linuxSystemExecutionBuildPlan({ target, profile = 'release' }) {
  target = resolveLinuxSystemTarget(target);
  if (!['debug', 'release'].includes(profile)) throw new Error(`unsupported Linux system execution profile: ${profile}`);
  return ['build', '--locked', '-p', 'nomi-system-execution', '--bin', 'nomifun-system-runner',
    '--target', target, '--profile', profile === 'debug' ? 'dev' : profile, '--message-format=json-render-diagnostics'];
}

export function linuxSystemExecutionBundleConfig(directory) {
  const runner = join(directory, 'nomifun-system-runner');
  const policy = join(directory, ASSET_NAMES[0]);
  const files = { [LINUX_SYSTEM_RUNNER]: runner, [LINUX_SYSTEM_POLICY]: policy };
  return {
    bundle: {
      linux: {
        deb: { depends: DESKTOP_DEB_DEPENDENCIES, files },
        rpm: { depends: DESKTOP_RPM_DEPENDENCIES, files },
        // An AppImage cannot install files in the host's /usr. Carry a complete,
        // explicit installer alongside its exact runner instead of claiming it did.
        appimage: { files: Object.fromEntries(['nomifun-system-runner', ...ASSET_NAMES]
          .map(name => [`/usr/lib/nomifun-system-execution/${name}`, join(directory, name)])) },
      },
    },
  };
}

const run = (program, args, { root = ROOT, environment = process.env } = {}) => new Promise((accept, reject) => {
  const child = spawn(program, args, { cwd: root, env: environment, stdio: ['ignore', 'pipe', 'inherit'] });
  let output = '';
  child.stdout.on('data', bytes => { output += bytes; });
  child.once('error', reject);
  child.once('close', code => code === 0 ? accept(output) : reject(new Error(`${program} exited ${code}`)));
});

export async function prepareLinuxSystemExecution({ root = ROOT, target, profile = 'release', environment = process.env } = {}, { execute = run } = {}) {
  if (process.platform !== 'linux') throw new Error('Linux system execution requires a Linux build host');
  target = resolveLinuxSystemTarget(target ?? environment.CARGO_BUILD_TARGET);
  const reports = await execute('cargo', linuxSystemExecutionBuildPlan({ target, profile }), { root, environment });
  let executable;
  for (const line of reports.split('\n')) {
    let report; try { report = JSON.parse(line); } catch { continue; }
    if (report.reason === 'compiler-artifact' && report.target?.name === 'nomifun-system-runner'
      && report.target?.kind?.includes('bin') && report.executable) executable = report.executable;
  }
  if (!executable || !(await lstat(executable)).isFile()) throw new Error('Cargo did not identify the exact Linux system runner executable');
  const bytes = await readFile(executable);
  const machine = target.startsWith('x86_64') ? 62 : 183;
  if (bytes.length < 64 || !bytes.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))
    || bytes[4] !== 2 || bytes[5] !== 1 || bytes.readUInt16LE(18) !== machine) {
    throw new Error(`Linux system runner is not an ELF executable for ${target}`);
  }
  const destination = join(root, 'target/linux-system-execution', target, profile);
  await mkdir(destination, { recursive: true });
  // Freeze this generation so later Cargo builds cannot change package inputs.
  const directory = await mkdtemp(join(destination, 'run-'));
  await writeFile(join(directory, 'nomifun-system-runner'), bytes, { mode: 0o755 });
  await chmod(join(directory, 'nomifun-system-runner'), 0o755);
  for (const name of ASSET_NAMES) {
    await copyFile(join(root, 'apps/desktop/linux-system-execution', name), join(directory, name));
    await chmod(join(directory, name), name === 'install.sh' ? 0o755 : 0o644);
  }
  const config = join(directory, 'tauri.system-execution.conf.json');
  await writeFile(config, `${JSON.stringify(linuxSystemExecutionBundleConfig(directory), null, 2)}\n`);
  const sha256 = createHash('sha256').update(bytes).digest('hex');
  await writeFile(join(directory, 'build-identity.json'), `${JSON.stringify({ target, profile, sha256 }, null, 2)}\n`);
  return { directory, config, sha256 };
}
