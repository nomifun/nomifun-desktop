#!/usr/bin/env bun
/**
 * Start Tauri development with the native toolchain environment it needs.
 *
 * Rust's Windows MSVC target needs both the Visual C++ and Windows SDK library
 * paths. Ordinary PowerShell terminals often omit them even when Build Tools
 * are installed, which makes rust-lld fail at the final desktop link. Keep the
 * environment process-local: discover vcvars64, capture its environment, and
 * pass it only to the Tauri child. Other platforms retain the original path.
 */

import { existsSync, mkdirSync, readdirSync, mkdtempSync, rmSync } from 'node:fs';
import { createServer } from 'node:net';
import { dirname, join, resolve } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const WINDOWS_TOOLCHAIN_COMPONENT =
  'Microsoft.VisualStudio.Component.VC.Tools.x86.x64';

function getEnvironmentValue(environment, name) {
  const expected = name.toLowerCase();
  for (const [key, value] of Object.entries(environment)) {
    if (key.toLowerCase() === expected) {
      return typeof value === 'string' ? value : '';
    }
  }
  return '';
}

export function splitEnvironmentPath(value) {
  if (typeof value !== 'string') return [];
  return value
    .split(';')
    .map((entry) => entry.trim())
    .filter(Boolean);
}

function normalizeWindowsPath(value) {
  return value
    .trim()
    .replace(/^"+|"+$/g, '')
    .replace(/\//g, '\\')
    .replace(/\\{2,}/g, '\\')
    .replace(/\\+$/g, '')
    .toLowerCase();
}

function pathMatches(entries, predicate) {
  return entries.some((entry) => predicate(normalizeWindowsPath(entry)));
}

const MSVC_HOST_X64_PATH = /(?:^|\\)hostx64\\x64(?:\\|$)/;
const MSVC_LIBRARY_X64_PATH =
  /(?:^|\\)(?:vc\\tools\\)?msvc\\[^\\]+\\lib\\x64(?:\\|$)/;
const WINDOWS_SDK_UCRT_X64_PATH =
  /(?:^|\\)windows kits\\(?:10|11)\\lib\\[^\\]+\\ucrt\\x64(?:\\|$)/;
const WINDOWS_SDK_UM_X64_PATH =
  /(?:^|\\)windows kits\\(?:10|11)\\lib\\[^\\]+\\um\\x64(?:\\|$)/;
const WINDOWS_SDK_BIN_X64_PATH =
  /(?:^|\\)windows kits\\(?:10|11)\\bin\\(?:[^\\]+\\)?x64(?:\\|$)/;

function extractWindowsSdkVersion(environment) {
  for (const name of ['WindowsSDKVersion', 'WindowsSdkVerBinPath']) {
    const value = getEnvironmentValue(environment, name);
    const match = value.match(/\b\d+\.\d+\.\d+\.\d+\b/);
    if (match) return match[0].toLowerCase();
  }
  return null;
}

function pathMatchesSelectedSdkVersion(entry, sdkVersion) {
  return !sdkVersion || normalizeWindowsPath(entry).includes(sdkVersion);
}

function selectedSdkPathMatches(entries, predicate, sdkVersion) {
  return entries.some(
    (entry) =>
      predicate(normalizeWindowsPath(entry)) &&
      pathMatchesSelectedSdkVersion(entry, sdkVersion),
  );
}

export function parseCommandEnvironment(output, baseEnvironment = {}) {
  const environment = { ...baseEnvironment };
  for (const line of output.split(/\r?\n/)) {
    const separator = line.indexOf('=');
    if (separator <= 0) continue;
    const key = line.slice(0, separator);
    const value = line.slice(separator + 1);
    // cmd.exe exposes drive-current-directory pseudo variables such as
    // `=C:=C:\path`; they cannot be passed through Node's env object.
    if (key.startsWith('=')) continue;
    const previousKey = Object.keys(environment).find(
      (existingKey) => existingKey.toLowerCase() === key.toLowerCase(),
    );
    if (previousKey && previousKey !== key) {
      delete environment[previousKey];
    }
    environment[key] = value;
  }
  return environment;
}

export function validateWindowsLinkEnvironment(environment = {}) {
  const pathEntries = splitEnvironmentPath(getEnvironmentValue(environment, 'PATH'));
  const libraryEntries = splitEnvironmentPath(getEnvironmentValue(environment, 'LIB'));
  const libraryPathEntries = splitEnvironmentPath(
    getEnvironmentValue(environment, 'LIBPATH'),
  );
  const sdkVersion = extractWindowsSdkVersion(environment);

  const checks = {
    msvcHostX64Path: pathMatches(pathEntries, (entry) =>
      MSVC_HOST_X64_PATH.test(entry),
    ),
    msvcLibraryX64Path: pathMatches(libraryEntries, (entry) =>
      MSVC_LIBRARY_X64_PATH.test(entry),
    ),
    msvcLibraryPathX64Path: pathMatches(libraryPathEntries, (entry) =>
      MSVC_LIBRARY_X64_PATH.test(entry),
    ),
    windowsSdkBinX64Path: selectedSdkPathMatches(
      pathEntries,
      (entry) => WINDOWS_SDK_BIN_X64_PATH.test(entry),
      sdkVersion,
    ),
    windowsSdkUcrtX64Path: selectedSdkPathMatches(
      libraryEntries,
      (entry) => WINDOWS_SDK_UCRT_X64_PATH.test(entry),
      sdkVersion,
    ),
    windowsSdkUmX64Path: selectedSdkPathMatches(
      libraryEntries,
      (entry) => WINDOWS_SDK_UM_X64_PATH.test(entry),
      sdkVersion,
    ),
  };

  const missing = [];
  if (!checks.msvcHostX64Path) {
    missing.push({
      variable: 'PATH',
      expected: 'MSVC bin\\Hostx64\\x64 compiler/linker directory',
    });
  }
  if (!checks.msvcLibraryX64Path) {
    missing.push({
      variable: 'LIB',
      expected: 'MSVC lib\\x64 directory',
    });
  }
  if (!checks.msvcLibraryPathX64Path) {
    missing.push({
      variable: 'LIBPATH',
      expected: 'MSVC lib\\x64 directory',
    });
  }
  if (!checks.windowsSdkBinX64Path) {
    missing.push({
      variable: 'PATH',
      expected: sdkVersion
        ? `Windows SDK ${sdkVersion} bin\\x64 directory`
        : 'Windows SDK bin\\x64 directory',
    });
  }
  if (!checks.windowsSdkUcrtX64Path) {
    missing.push({
      variable: 'LIB',
      expected: sdkVersion
        ? `Windows SDK ${sdkVersion} ucrt\\x64 directory`
        : 'Windows SDK ucrt\\x64 directory',
    });
  }
  if (!checks.windowsSdkUmX64Path) {
    missing.push({
      variable: 'LIB',
      expected: sdkVersion
        ? `Windows SDK ${sdkVersion} um\\x64 directory`
        : 'Windows SDK um\\x64 directory',
    });
  }

  return {
    ok: missing.length === 0,
    checks,
    missing,
    sdkVersion,
  };
}

export function hasWindowsLinkEnvironmentShape(environment) {
  return validateWindowsLinkEnvironment(environment).ok;
}

export function formatWindowsLinkEnvironmentError(validation) {
  const missing = validation.missing
    .map(({ variable, expected }) => `${variable} must contain ${expected}`)
    .join('; ');
  const suffix = validation.sdkVersion
    ? ` Selected Windows SDK: ${validation.sdkVersion}.`
    : '';
  return `Windows x64 MSVC/Windows SDK environment is incomplete. ${missing}.${suffix} Run from an x64 Native Tools prompt or allow run-dev to initialize vcvars64.bat.`;
}

function pushCandidate(candidates, candidate) {
  if (!candidate) return;
  const absolute = resolve(candidate);
  if (!candidates.includes(absolute)) candidates.push(absolute);
}

function visualStudioInstallations(environment) {
  const installations = [];
  const vswhere = join(
    environment['ProgramFiles(x86)'] ?? 'C:\\Program Files (x86)',
    'Microsoft Visual Studio',
    'Installer',
    'vswhere.exe',
  );
  if (existsSync(vswhere)) {
    const result = spawnSync(
      vswhere,
      [
        '-latest',
        '-products',
        '*',
        '-requires',
        WINDOWS_TOOLCHAIN_COMPONENT,
        '-property',
        'installationPath',
      ],
      {
        encoding: 'utf8',
        windowsHide: true,
        env: environment,
      },
    );
    if (!result.error && result.status === 0) {
      for (const line of result.stdout.split(/\r?\n/)) {
        const installation = line.trim();
        if (installation) pushCandidate(installations, installation);
      }
    }
  }

  for (const programFiles of [
    environment['ProgramFiles(x86)'],
    environment.ProgramFiles,
    'C:\\Program Files (x86)',
    'C:\\Program Files',
  ]) {
    if (!programFiles) continue;
    for (const year of ['2022', '2019']) {
      const root = join(programFiles, 'Microsoft Visual Studio', year);
      let editions;
      try {
        editions = readdirSync(root, { withFileTypes: true });
      } catch {
        continue;
      }
      for (const edition of editions) {
        if (edition.isDirectory()) {
          pushCandidate(installations, join(root, edition.name));
        }
      }
    }
  }
  return installations;
}

export function findVcvars64(environment = process.env) {
  const candidates = [];
  if (environment.VCToolsInstallDir) {
    pushCandidate(
      candidates,
      resolve(
        environment.VCToolsInstallDir,
        '..',
        '..',
        '..',
        'Auxiliary',
        'Build',
        'vcvars64.bat',
      ),
    );
  }
  if (environment.VCINSTALLDIR) {
    pushCandidate(
      candidates,
      join(environment.VCINSTALLDIR, 'Auxiliary', 'Build', 'vcvars64.bat'),
    );
  }
  for (const installation of visualStudioInstallations(environment)) {
    pushCandidate(
      candidates,
      join(installation, 'VC', 'Auxiliary', 'Build', 'vcvars64.bat'),
    );
  }
  return candidates.find((candidate) => existsSync(candidate)) ?? null;
}

export function loadWindowsToolchainEnvironment(
  environment = process.env,
  platform = process.platform,
) {
  if (platform !== 'win32') {
    return { ...environment };
  }

  const existingValidation = validateWindowsLinkEnvironment(environment);
  if (existingValidation.ok) return { ...environment };

  const vcvars64 = findVcvars64(environment);
  if (!vcvars64) {
    throw new Error(
      `${formatWindowsLinkEnvironmentError(existingValidation)} Visual Studio Build Tools with the Desktop development with C++ workload and vcvars64.bat were not found.`,
    );
  }
  const command = `call "${vcvars64}" >nul && set`;
  const result = spawnSync(environment.ComSpec ?? 'cmd.exe', ['/d', '/s', '/c', command], {
    encoding: 'utf8',
    windowsHide: true,
    windowsVerbatimArguments: true,
    env: environment,
  });
  if (result.error || result.status !== 0) {
    const reason = result.error?.message ?? `cmd.exe exited with status ${result.status}`;
    throw new Error(`Visual Studio native toolchain environment initialization failed: ${reason}`);
  }

  const initialized = parseCommandEnvironment(result.stdout ?? '', environment);
  const initializedValidation = validateWindowsLinkEnvironment(initialized);
  if (!initializedValidation.ok) {
    throw new Error(formatWindowsLinkEnvironmentError(initializedValidation));
  }
  return initialized;
}

// The Plugin clean-start refactor intentionally replaced the historical product
// migrations. Reusing NomiFun-dev[-nomi-core] cannot upgrade that database.
// Give Windows dev a stable generation-specific root without deleting or
// rewriting old data. Explicit data roots remain the caller's responsibility.
// TODO(platform): validate the same clean-start dev policy on macOS/Linux.
export function developmentEnvironment(environment, platform = process.platform) {
  const result = { ...environment, NOMI_CHANNEL: 'dev' };
  if (platform !== 'win32') return result;
  const explicitKey = Object.keys(environment).find(
    (key) => key.toUpperCase() === 'NOMIFUN_DATA_DIR',
  );
  if (explicitKey) {
    if (!environment[explicitKey]?.trim()) {
      throw new Error('NOMIFUN_DATA_DIR must not be empty; unset it to use the isolated development data directory');
    }
    return result;
  }
  const localAppData = getEnvironmentValue(environment, 'LOCALAPPDATA');
  if (!localAppData) {
    throw new Error('LOCALAPPDATA is unavailable; set NOMIFUN_DATA_DIR to an explicit development data directory');
  }
  result.NOMIFUN_DATA_DIR = join(localAppData, 'NomiFun-dev-plugin-v1');
  return result;
}

/**
 * Materialize only the generated Windows development root. Explicit roots are
 * still owned by their caller, matching the clean-start contract above.
 */
export function ensureGeneratedDevelopmentDataDirectory(
  environment,
  sourceEnvironment = process.env,
  platform = process.platform,
  createDirectory = mkdirSync,
) {
  if (platform !== 'win32') return null;
  const explicitKey = Object.keys(sourceEnvironment).find(
    (key) => key.toUpperCase() === 'NOMIFUN_DATA_DIR',
  );
  if (explicitKey) return null;

  const localAppData = getEnvironmentValue(sourceEnvironment, 'LOCALAPPDATA');
  const target = getEnvironmentValue(environment, 'NOMIFUN_DATA_DIR');
  const expected = localAppData
    ? join(localAppData, 'NomiFun-dev-plugin-v1')
    : '';
  if (!target || target !== expected) {
    throw new Error('generated Windows development data directory does not match LOCALAPPDATA');
  }
  createDirectory(target, { recursive: true });
  return target;
}

// A per-run socket ties the macOS dev app to this runner without signalling
// unrelated processes or trusting stale PIDs. Sending a stop byte requests
// the app's ordinary ExitCoordinator shutdown; its connection closes only when
// the app exits. The server also disappears if the runner unexpectedly dies.
export async function createMacosDevLifetime() {
  // macOS Unix socket paths have a small length limit; $TMPDIR can be too long.
  const directory = mkdtempSync('/tmp/nomifun-dev-');
  const socketPath = join(directory, 'lifetime.sock');
  const connections = new Set();
  let stopping = false;
  let finishShutdown = () => {};
  const server = createServer((socket) => {
    connections.add(socket);
    socket.resume();
    socket.on('error', () => socket.destroy());
    socket.on('end', () => socket.destroy());
    socket.on('close', () => {
      connections.delete(socket);
      finishShutdown();
    });
    if (stopping) socket.write('q');
  });
  try {
    await new Promise((resolve, reject) => {
      server.once('error', reject);
      server.listen(socketPath, resolve);
    });
  } catch (error) {
    rmSync(directory, { recursive: true, force: true });
    throw error;
  }
  let shutdown;
  return {
    socketPath,
    stop() {
      if (shutdown) return shutdown;
      stopping = true;
      shutdown = new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
          reject(new Error('macOS desktop did not finish graceful shutdown within 120 seconds'));
        }, 120_000);
        // Stop new connections. A desktop still starting will fail to connect
        // and take the same graceful exit path instead of becoming an orphan.
        let serverClosed = false;
        finishShutdown = () => {
          if (!serverClosed || connections.size !== 0) return;
          clearTimeout(timeout);
          rmSync(directory, { recursive: true, force: true });
          resolve();
        };
        server.close(() => {
          serverClosed = true;
          finishShutdown();
        });
        for (const socket of connections) socket.write('q');
      });
      return shutdown;
    },
  };
}

async function main() {
  let environment;
  try {
    environment = developmentEnvironment(loadWindowsToolchainEnvironment(process.env));
    ensureGeneratedDevelopmentDataDirectory(environment, process.env);
  } catch (error) {
    console.error(`[dev] ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
    return;
  }

  const tauri = join(
    ROOT,
    'node_modules',
    '.bin',
    process.platform === 'win32' ? 'tauri.exe' : 'tauri',
  );
  if (!existsSync(tauri)) {
    console.error('[dev] Tauri CLI is not installed; run bun install first');
    process.exitCode = 1;
    return;
  }

  if (process.platform === 'win32') {
    console.log(`[dev] data directory: ${getEnvironmentValue(environment, 'NOMIFUN_DATA_DIR')} (existing historical directories are preserved)`);
  }

  const lifetime = process.platform === 'darwin'
    ? await createMacosDevLifetime()
    : null;
  if (lifetime) environment.NOMIFUN_DEV_LIFETIME_SOCKET = lifetime.socketPath;

  const child = spawn(
    tauri,
    [
      'dev',
      '--config',
      'apps/desktop/tauri.conf.json',
      '--config',
      'apps/desktop/tauri.dev.conf.json',
      ...process.argv.slice(2),
    ],
    {
      cwd: ROOT,
      env: environment,
      stdio: 'inherit',
      windowsHide: false,
      // Keep terminal Ctrl-C on the runner until the app has cleaned up its
      // managed tools; signalling the whole child group would race cleanup.
      detached: process.platform === 'darwin',
    },
  );
  let stopRequest;
  const requestStop = () => {
    if (stopRequest || !lifetime) return;
    stopRequest = lifetime.stop().then(() => {
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGINT');
    }).catch((error) => {
      console.error(`[dev] ${error.message}; no forced termination was attempted`);
      process.exitCode = 1;
    });
  };
  if (lifetime) {
    process.on('SIGINT', requestStop);
    process.on('SIGTERM', requestStop);
  }
  child.on('error', (error) => {
    console.error(`[dev] failed to start Tauri: ${error.message}`);
  });

  const result = await new Promise((complete) => {
    child.once('exit', (code, signal) => complete({ code, signal }));
    child.once('error', () => complete({ code: 1, signal: null }));
  });
  if (lifetime) {
    try {
      await lifetime.stop();
    } catch (error) {
      console.error(`[dev] ${error.message}; no forced termination was attempted`);
      process.exitCode = 1;
    }
    process.off('SIGINT', requestStop);
    process.off('SIGTERM', requestStop);
  }
  process.exitCode ||= result.code ?? (result.signal ? 1 : 0);
}

if (import.meta.main) {
  await main();
}
