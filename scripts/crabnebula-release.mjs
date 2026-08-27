#!/usr/bin/env bun
/**
 * CrabNebula Cloud release helper for NomiFun's non-standard Tauri workspace.
 *
 * NomiFun builds on separate platform machines and keeps Tauri configuration in
 * apps/desktop/. This helper therefore reads the merged latest.json and uploads
 * local artifacts with explicit CrabNebula platform names instead of relying on
 * the CLI's framework auto-discovery.
 *
 * Typical multi-machine release:
 *
 *   bun run release:cloud -- draft --notes-file notes.md
 *   bun run release:cloud -- upload --release-id <id>   # each build machine
 *   bun run release:cloud -- publish --release-id <id>  # after all platforms
 *   bun run release:cloud -- verify
 *
 * CN_API_KEY is read from the environment or from the gitignored
 * apps/desktop/signing/.env.release file. Never commit a real key.
 */
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const DEFAULT_APP = 'nomifun/nomifun-desktop';
const DEFAULT_MANIFEST = join(ROOT, 'apps/desktop/updater/latest.json');
const RELEASE_ID_FILE = join(ROOT, 'dist/desktop/crabnebula-release-id.txt');
const RELEASE_ENV_FILE = join(ROOT, 'apps/desktop/signing/.env.release');
const ULID_PATTERN = /\b[0-9A-HJKMNP-TV-Z]{26}\b/g;
const EXACT_ULID_PATTERN = /^[0-9A-HJKMNP-TV-Z]{26}$/;

function fail(message) {
  console.error(`ERROR: ${message}`);
  process.exit(1);
}

function rel(path) {
  return relative(ROOT, path) || '.';
}

function readWorkspaceVersion() {
  const lines = readFileSync(join(ROOT, 'Cargo.toml'), 'utf8').split(/\r?\n/);
  let inSection = false;
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith('[')) {
      inSection = trimmed === '[workspace.package]';
      continue;
    }
    if (inSection) {
      const match = line.match(/^\s*version\s*=\s*"([^"]+)"/);
      if (match) return match[1];
    }
  }
  fail('cannot read [workspace.package].version from Cargo.toml');
}

function loadReleaseEnvironment() {
  if (!existsSync(RELEASE_ENV_FILE)) return;
  for (const rawLine of readFileSync(RELEASE_ENV_FILE, 'utf8').split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith('#')) continue;
    const match = line.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/);
    if (!match || process.env[match[1]]) continue;
    let value = match[2].trim();
    if (
      value.length >= 2 &&
      ((value.startsWith('"') && value.endsWith('"')) ||
        (value.startsWith("'") && value.endsWith("'")))
    ) {
      value = value.slice(1, -1);
    }
    process.env[match[1]] = value;
  }
}

function parseArgs(argv) {
  const args = argv.filter((arg) => arg !== '--');
  const command = args.shift();
  const values = new Map();
  const switches = new Set();
  const repeated = new Map();

  while (args.length) {
    const token = args.shift();
    if (!token?.startsWith('--')) fail(`unknown argument: ${token}`);
    const equal = token.indexOf('=');
    const name = equal === -1 ? token.slice(2) : token.slice(2, equal);
    const inlineValue = equal === -1 ? undefined : token.slice(equal + 1);
    if (['dry-run', 'no-manual'].includes(name)) {
      switches.add(name);
      continue;
    }
    const value = inlineValue ?? args.shift();
    if (value == null || value.startsWith('--')) fail(`--${name} requires a value`);
    if (name === 'platform') {
      const current = repeated.get(name) ?? [];
      current.push(value);
      repeated.set(name, current);
    } else {
      values.set(name, value);
    }
  }
  return { command, values, switches, repeated };
}

function option(parsed, name, fallback) {
  return parsed.values.has(name) ? parsed.values.get(name) : fallback;
}

function readStoredReleaseId() {
  if (!existsSync(RELEASE_ID_FILE)) return null;
  const value = readFileSync(RELEASE_ID_FILE, 'utf8').trim();
  return value.match(ULID_PATTERN)?.[0] ?? null;
}

function resolveReleaseId(parsed) {
  return option(parsed, 'release-id') || process.env.CN_RELEASE_ID || readStoredReleaseId();
}

function releaseIdFromCliOutput(stdout, stderr) {
  const output = `${stdout ?? ''}`.trim();
  if (output) {
    try {
      const payload = JSON.parse(output);
      if (typeof payload?.id === 'string' && EXACT_ULID_PATTERN.test(payload.id)) {
        return payload.id;
      }
    } catch {
      // Older CLI versions may print human-readable output instead of JSON.
    }
  }

  const ids = [...new Set(`${stdout ?? ''}\n${stderr ?? ''}`.match(ULID_PATTERN) ?? [])];
  return ids.length === 1 ? ids[0] : null;
}

function resolveCli(parsed, dryRun) {
  const cli = option(parsed, 'cli', process.env.CN_CLI || 'cn');
  if (dryRun) return cli;
  const probe = spawnSync(cli, ['--version'], {
    cwd: ROOT,
    encoding: 'utf8',
    env: process.env,
    windowsHide: true,
  });
  if (probe.error || probe.status !== 0) {
    fail(
      `CrabNebula CLI '${cli}' is unavailable. Install it from the official ` +
        `Cloud CLI page, or set CN_CLI to its absolute path.`,
    );
  }
  return cli;
}

function printableCommand(cli, args) {
  const quote = (value) =>
    /[\s"]/u.test(value) ? `"${value.replaceAll('"', '\\"')}"` : value;
  return [cli, ...args].map(quote).join(' ');
}

function runCli(cli, args, { dryRun = false, capture = false } = {}) {
  console.log(`> ${printableCommand(cli, args)}`);
  if (dryRun) return { stdout: '', stderr: '', status: 0 };
  const result = spawnSync(cli, args, {
    cwd: ROOT,
    encoding: 'utf8',
    env: process.env,
    windowsHide: true,
    stdio: capture ? 'pipe' : 'inherit',
  });
  if (capture) {
    if (result.stdout) process.stdout.write(result.stdout);
    if (result.stderr) process.stderr.write(result.stderr);
  }
  if (result.error) fail(`failed to start CrabNebula CLI: ${result.error.message}`);
  if (result.status !== 0) fail(`CrabNebula CLI exited with code ${result.status}`);
  return result;
}

function walkFiles(root) {
  if (!existsSync(root)) return [];
  const found = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir)) {
      const full = join(dir, entry);
      const stat = statSync(full);
      if (stat.isDirectory()) walk(full);
      else if (stat.isFile()) found.push(full);
    }
  };
  walk(root);
  return found;
}

function artifactRoots() {
  const target = join(ROOT, 'target');
  const roots = [join(target, 'release', 'bundle'), join(ROOT, 'dist/desktop')];
  if (existsSync(target)) {
    for (const entry of readdirSync(target)) {
      const nested = join(target, entry, 'release', 'bundle');
      if (existsSync(nested)) roots.push(nested);
    }
  }
  return roots;
}

function allArtifactFiles() {
  return [...new Set(artifactRoots().flatMap(walkFiles))];
}

function signatureMatches(artifact, expectedSignature) {
  const signatureFile = `${artifact}.sig`;
  return (
    existsSync(signatureFile) &&
    readFileSync(signatureFile, 'utf8').trim() === expectedSignature.trim()
  );
}

function platformArch(platform) {
  if (platform.endsWith('-x86_64')) return 'x86_64';
  if (platform.endsWith('-aarch64')) return 'aarch64';
  if (platform.endsWith('-i686')) return 'i686';
  if (platform.endsWith('-armv7')) return 'armv7';
  return null;
}

function preferredPathScore(path, platform) {
  const normalized = path.replaceAll('\\', '/').toLowerCase();
  let score = 0;
  if (platform.startsWith('darwin-') && normalized.includes('universal-apple-darwin')) score += 20;
  const arch = platformArch(platform);
  if (arch && normalized.includes(arch)) score += 10;
  if (normalized.includes('/dist/desktop/')) score -= 1;
  return score;
}

function findLocalUpdaterArtifact(files, platform, entry) {
  let assetName;
  try {
    assetName = basename(new URL(entry.url).pathname);
  } catch {
    fail(`invalid updater URL for ${platform}: ${entry.url}`);
  }
  return (
    files
      .filter((path) => basename(path) === assetName)
      .filter((path) => signatureMatches(path, entry.signature))
      .sort((a, b) => preferredPathScore(b, platform) - preferredPathScore(a, platform))[0] ?? null
  );
}

function publicPlatformForUpdater(platform, artifact) {
  const arch = platformArch(platform);
  if (!arch) return null;
  const lower = artifact.toLowerCase();
  if (platform.startsWith('windows-')) {
    if (lower.endsWith('.msi')) return `wix-${arch}`;
    if (lower.endsWith('.exe')) return `nsis-${arch}`;
  }
  if (platform.startsWith('linux-') && lower.endsWith('.appimage')) {
    return `appimage-${arch}`;
  }
  return null;
}

function manualPlatformActions(files, version, availablePlatforms) {
  const distPrefix = join(ROOT, 'dist', 'desktop').toLowerCase();
  const actions = [];
  for (const file of files.filter((path) => path.toLowerCase().startsWith(distPrefix))) {
    const name = basename(file);
    const lower = name.toLowerCase();
    if (!lower.includes(version.toLowerCase())) continue;

    if (lower.endsWith('.dmg')) {
      const platforms = lower.includes('universal')
        ? ['darwin-x86_64', 'darwin-aarch64']
        : lower.includes('aarch64') || lower.includes('arm64')
          ? ['darwin-aarch64']
          : lower.includes('x86_64') || lower.includes('x64') || lower.includes('intel')
            ? ['darwin-x86_64']
            : [];
      for (const platform of platforms) {
        if (!availablePlatforms.has(platform)) continue;
        actions.push({
          file,
          publicPlatform: `dmg-${platformArch(platform)}`,
          updatePlatform: null,
          signature: null,
        });
      }
    } else if (lower.endsWith('.deb')) {
      const arch = lower.includes('arm64') || lower.includes('aarch64') ? 'aarch64' : 'x86_64';
      actions.push({
        file,
        publicPlatform: `deb-${arch}`,
        updatePlatform: null,
        signature: null,
      });
    } else if (lower.endsWith('.rpm')) {
      const arch = lower.includes('arm64') || lower.includes('aarch64') ? 'aarch64' : 'x86_64';
      actions.push({
        file,
        publicPlatform: `rpm-${arch}`,
        updatePlatform: null,
        signature: null,
      });
    }
  }
  return actions;
}

function actionKey(action) {
  return [resolve(action.file), action.publicPlatform ?? '', action.updatePlatform ?? ''].join('|');
}

function discoverUploadActions(manifestPath, requestedPlatforms, includeManual, expectedVersion) {
  if (!existsSync(manifestPath)) fail(`updater manifest not found: ${rel(manifestPath)}`);
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  if (!manifest.version || !manifest.platforms) fail(`invalid updater manifest: ${rel(manifestPath)}`);
  if (expectedVersion && manifest.version !== expectedVersion) {
    fail(
      `manifest version ${manifest.version} does not match requested release ` +
        `version ${expectedVersion}; run bun run make:latest after bumping`,
    );
  }

  const files = allArtifactFiles();
  const selectedPlatforms = new Set(
    requestedPlatforms.length ? requestedPlatforms : Object.keys(manifest.platforms),
  );
  const actions = [];
  const missing = [];

  for (const platform of selectedPlatforms) {
    const entry = manifest.platforms[platform];
    if (!entry) {
      missing.push(`${platform} (not present in latest.json)`);
      continue;
    }
    const artifact = findLocalUpdaterArtifact(files, platform, entry);
    if (!artifact) {
      missing.push(`${platform} (matching local artifact + .sig not found)`);
      continue;
    }
    actions.push({
      file: artifact,
      publicPlatform: publicPlatformForUpdater(platform, artifact),
      updatePlatform: platform,
      signature: `${artifact}.sig`,
    });
  }

  if (requestedPlatforms.length && missing.length) {
    fail(`requested platform assets are unavailable:\n  - ${missing.join('\n  - ')}`);
  }
  if (includeManual) {
    actions.push(
      ...manualPlatformActions(
        files,
        manifest.version,
        new Set(Object.keys(manifest.platforms)),
      ),
    );
  }

  const unique = [...new Map(actions.map((action) => [actionKey(action), action])).values()];
  if (!unique.length) {
    const detail = missing.length ? `\nSkipped:\n  - ${missing.join('\n  - ')}` : '';
    fail(`no uploadable CrabNebula assets were found on this machine.${detail}`);
  }
  return { manifest, actions: unique, missing };
}

function uploadArgs(app, releaseId, action, channel) {
  const args = ['release', 'upload', app, releaseId];
  if (action.publicPlatform) args.push('--public-platform', action.publicPlatform);
  if (action.updatePlatform) args.push('--update-platform', action.updatePlatform);
  args.push('--file', action.file);
  if (action.signature) args.push('--signature', action.signature);
  if (channel) args.push('--channel', channel);
  return args;
}

async function verifyRelease(app, manifestPath, platforms, fromVersion, channel) {
  if (!existsSync(manifestPath)) fail(`updater manifest not found: ${rel(manifestPath)}`);
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  const selected = platforms.length ? platforms : Object.keys(manifest.platforms ?? {});
  if (!selected.length) fail('latest.json contains no platforms to verify');

  let failed = false;
  for (const platform of selected) {
    const expected = manifest.platforms?.[platform];
    if (!expected) {
      console.error(`FAIL ${platform}: not present in ${rel(manifestPath)}`);
      failed = true;
      continue;
    }
    const query = channel ? `?channel=${encodeURIComponent(channel)}` : '';
    const endpoint =
      `https://cdn.crabnebula.app/update/${app}/${platform}/` +
      `${encodeURIComponent(fromVersion)}${query}`;
    try {
      const response = await fetch(endpoint, {
        headers: { Accept: 'application/json' },
        redirect: 'follow',
        signal: AbortSignal.timeout(15_000),
      });
      if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
      const release = await response.json();
      if (release.version !== manifest.version) {
        throw new Error(`version ${release.version} != ${manifest.version}`);
      }
      if (release.signature !== expected.signature) {
        throw new Error('signature differs from latest.json');
      }
      console.log(`OK   ${platform}: ${release.version} -> ${release.url}`);
    } catch (error) {
      console.error(`FAIL ${platform}: ${error instanceof Error ? error.message : String(error)}`);
      failed = true;
    }
  }
  if (failed) process.exit(1);
}

function printHelp() {
  console.log(`
CrabNebula Cloud release helper

  bun run release:cloud -- draft [--version <semver>] [--notes-file <md>]
  bun run release:cloud -- upload --release-id <id> [--platform <key>] [--no-manual]
  bun run release:cloud -- publish --release-id <id>
  bun run release:cloud -- verify [--platform <key>] [--from-version <semver>]

Common options:
  --app <org/app>       Default: CN_APP or ${DEFAULT_APP}
  --channel <name>      Optional CrabNebula release channel
  --manifest <path>     Default: ${rel(DEFAULT_MANIFEST)}
  --cli <path>          Default: CN_CLI or cn
  --dry-run             Print cn commands without executing them

Release ID resolution order:
  --release-id, CN_RELEASE_ID, ${rel(RELEASE_ID_FILE)}
`);
}

loadReleaseEnvironment();
const parsed = parseArgs(process.argv.slice(2));
if (!parsed.command || ['help', '-h'].includes(parsed.command)) {
  printHelp();
  process.exit(0);
}

const dryRun = parsed.switches.has('dry-run');
const app = option(parsed, 'app', process.env.CN_APP || DEFAULT_APP);
const channel = option(parsed, 'channel', process.env.CN_CHANNEL);
const manifestPath = resolve(ROOT, option(parsed, 'manifest', DEFAULT_MANIFEST));
const version = option(parsed, 'version', readWorkspaceVersion());
const platforms = parsed.repeated.get('platform') ?? [];

if (parsed.command === 'verify') {
  await verifyRelease(
    app,
    manifestPath,
    platforms,
    option(parsed, 'from-version', '0.0.0'),
    channel,
  );
  process.exit(0);
}

const cli = resolveCli(parsed, dryRun);
if (!dryRun && !process.env.CN_API_KEY) {
  fail(
    `CN_API_KEY is required. Set it in the environment or in the gitignored ` +
      `${rel(RELEASE_ENV_FILE)} file.`,
  );
}

if (parsed.command === 'draft') {
  const args = ['release', 'draft', app, version];
  const notesFile = option(parsed, 'notes-file');
  const notes = option(parsed, 'notes');
  if (notesFile) args.push('--notes-file', resolve(ROOT, notesFile));
  else if (notes) args.push('--notes', notes);
  if (channel) args.push('--channel', channel);
  const result = runCli(cli, args, { dryRun, capture: !dryRun });
  if (!dryRun) {
    const releaseId = releaseIdFromCliOutput(result.stdout, result.stderr);
    if (releaseId) {
      mkdirSync(dirname(RELEASE_ID_FILE), { recursive: true });
      writeFileSync(RELEASE_ID_FILE, `${releaseId}\n`);
      console.log(`Saved release ID ${releaseId} to ${rel(RELEASE_ID_FILE)}`);
    } else {
      console.warn(
        `Could not identify the release ID in CLI output. Pass it explicitly with ` +
          `--release-id or CN_RELEASE_ID for upload/publish.`,
      );
    }
  }
  process.exit(0);
}

const releaseId = resolveReleaseId(parsed);
if (!releaseId) {
  fail(
    `release ID is required. Pass --release-id, set CN_RELEASE_ID, or run the ` +
      `draft command first on this machine.`,
  );
}

if (parsed.command === 'upload') {
  const { manifest, actions, missing } = discoverUploadActions(
    manifestPath,
    platforms,
    !parsed.switches.has('no-manual'),
    version,
  );
  console.log(`CrabNebula upload plan: ${app} ${manifest.version} (${releaseId})`);
  for (const action of actions) {
    console.log(
      `  ${rel(action.file)} -> ` +
        [
          action.publicPlatform && `public=${action.publicPlatform}`,
          action.updatePlatform && `update=${action.updatePlatform}`,
        ]
          .filter(Boolean)
          .join(', '),
    );
  }
  if (missing.length && !platforms.length) {
    console.log(`  Local machine does not contain: ${missing.join('; ')}`);
  }
  for (const action of actions) {
    runCli(cli, uploadArgs(app, releaseId, action, channel), { dryRun });
  }
  process.exit(0);
}

if (parsed.command === 'publish') {
  const args = ['release', 'publish', app, releaseId];
  if (channel) args.push('--channel', channel);
  runCli(cli, args, { dryRun });
  process.exit(0);
}

fail(`unknown command: ${parsed.command}`);
