#!/usr/bin/env bun
/**
 * build-docker-prebuilt -- build the runtime Docker image from local artifacts.
 *
 * This is the fast path for machines that already have:
 *   - ui/dist from `bun run build:ui`
 *   - target/release/nomifun-web from `cargo build --release --locked -p nomifun-web`
 *
 * The full Dockerfile remains the reproducible-from-source path. This script
 * stages only the paired WebUI bundle and Linux release binary into a small
 * build context, then uses Dockerfile.prebuilt to assemble the runtime image.
 */
import { spawnSync } from 'node:child_process';
import {
  chmodSync,
  closeSync,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  rmSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const PKG = join(ROOT, 'package.json');
const UI_DIST = join(ROOT, 'ui', 'dist');
const UI_MANIFEST = join(UI_DIST, 'nomifun-build.json');
const CONTEXT = join(ROOT, 'dist', 'docker-prebuilt-context');
const DOCKERFILE = join(ROOT, 'Dockerfile.prebuilt');
const TAG = '[docker:prebuilt]';
const DEFAULT_REPOSITORY = 'nomifun/nomifun-web';
const DEFAULT_BUN_IMAGE = 'oven/bun:1';
const DEFAULT_NODE_IMAGE = 'node:22-bookworm-slim';
const DEFAULT_RUNTIME_IMAGE = 'ubuntu:26.04';

const args = parseArgs(process.argv.slice(2).filter((arg) => arg !== '--'));
const pkg = readJson(PKG, 'package.json');
const version = pkg.version;

if (!version) fail('package.json has no version.');

if (args.help) {
  printHelp(version);
  process.exit(0);
}

const requestedTags = args.tags.length
  ? args.tags
  : [`${args.repository}:v${version}`];

if (args.buildMissing) {
  ensureUiDistForBuild();
}

const manifest = validateUiDist(version);
const binary = resolveBinary(args.binary);

if (!binary && args.buildMissing) {
  run('cargo', ['build', '--release', '--locked', '-p', 'nomifun-web']);
}

const finalBinary = resolveBinary(args.binary);
if (!finalBinary) {
  const debugHint = existsSync(join(ROOT, 'target', 'debug', 'nomifun-web'))
    ? ' A debug binary exists, but it cannot serve the static WebUI because it does not embed the frontend build id.'
    : '';
  const fixHint = args.buildMissing ? '' : `\nFix: ${buildMissingSuggestion()}`;
  fail(
    `missing release binary. Run \`cargo build --release --locked -p nomifun-web\` after \`bun run build:ui\`, or rerun this script with \`--build-missing\`.${debugHint}${fixHint}`
  );
}

validateLinuxBinary(finalBinary);
validateKnownRuntimeGlibc(finalBinary);
validateBinaryMatchesUi(finalBinary, manifest.frontend_build_id);
stageContext(finalBinary);

const tags = requestedTags;
const dockerArgs = [
  'build',
  '-f',
  DOCKERFILE,
  '--build-arg',
  `BUN_IMAGE=${args.bunImage}`,
  '--build-arg',
  `NODE_IMAGE=${args.nodeImage}`,
  '--build-arg',
  `RUNTIME_IMAGE=${args.runtimeImage}`,
  '--build-arg',
  `NOMIFUN_VERSION=${version}`,
];
if (args.aptMirror) dockerArgs.push('--build-arg', `APT_MIRROR=${args.aptMirror}`);
for (const tag of tags) dockerArgs.push('-t', tag);
dockerArgs.push(CONTEXT);

log(`staged ${relative(finalBinary)} + ui/dist for ${manifest.app_version} (${manifest.frontend_build_id})`);
log(`image tag${tags.length === 1 ? '' : 's'}: ${tags.join(', ')}`);

if (args.dryRun) {
  log(`dry run; would execute: ${dockerCommandForDisplay(dockerArgs)}`);
  process.exit(0);
}

if (args.sudo) run('sudo', [args.dockerBin, ...dockerArgs]);
else run(args.dockerBin, dockerArgs);

function parseArgs(argv) {
  const parsed = {
    aptMirror: process.env.APT_MIRROR || '',
    binary: '',
    buildMissing: false,
    bunImage: process.env.BUN_IMAGE || DEFAULT_BUN_IMAGE,
    dockerBin: process.env.DOCKER || 'docker',
    dryRun: false,
    help: false,
    nodeImage: process.env.NODE_IMAGE || DEFAULT_NODE_IMAGE,
    repository: process.env.DOCKER_REPOSITORY || DEFAULT_REPOSITORY,
    runtimeImage: process.env.RUNTIME_IMAGE || DEFAULT_RUNTIME_IMAGE,
    sudo: false,
    tags: [],
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const [name, inlineValue] = arg.includes('=') ? arg.split(/=(.*)/s, 2) : [arg, undefined];
    const value = () => {
      if (inlineValue !== undefined) return inlineValue;
      i += 1;
      if (i >= argv.length) fail(`${name} requires a value.`);
      return argv[i];
    };

    switch (name) {
      case '--help':
      case '-h':
        parsed.help = true;
        break;
      case '--tag':
      case '-t':
        parsed.tags.push(value());
        break;
      case '--binary':
        parsed.binary = value();
        break;
      case '--runtime-image':
        parsed.runtimeImage = value();
        break;
      case '--bun-image':
        parsed.bunImage = value();
        break;
      case '--node-image':
        parsed.nodeImage = value();
        break;
      case '--apt-mirror':
        parsed.aptMirror = value();
        break;
      case '--docker':
        parsed.dockerBin = value();
        break;
      case '--repository':
        parsed.repository = value();
        break;
      case '--build-missing':
        parsed.buildMissing = true;
        break;
      case '--dry-run':
        parsed.dryRun = true;
        break;
      case '--sudo':
        parsed.sudo = true;
        break;
      default:
        fail(`unknown argument: ${arg}`);
    }
  }

  return parsed;
}

function printHelp(currentVersion) {
  console.log(`Usage:
  bun run docker:prebuilt
  bun run docker:prebuilt -- --build-missing --sudo
  bun run docker:prebuilt -- --tag nomifun/nomifun-web:v${currentVersion} --build-missing --sudo

Options:
  -t, --tag <tag>          Docker image tag. Repeatable.
      --repository <repo>  Repository for the default tag. Default: ${DEFAULT_REPOSITORY}
      --binary <path>      Linux release nomifun-web binary to copy.
      --build-missing      Build missing/outdated local artifacts first; no-op when they already match.
      --dry-run            Stage artifacts and print the docker command only.
      --bun-image <image>  Bun image for copying /usr/local/bin/bun.
      --node-image <image> Node image for copying Node.js plus npm/npx.
      --runtime-image <i>  Runtime image for the prebuilt Linux binary.
      --apt-mirror <url>   Optional apt mirror for Debian/Ubuntu runtime images.
      --docker <command>   Docker-compatible CLI command.
      --sudo               Run the final docker build through sudo.
`);
}

function readJson(path, label) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch (error) {
    fail(`failed to read ${label}: ${error.message}`);
  }
}

function ensureUiDistForBuild() {
  if (validUiManifest(version)) return;
  run('bun', ['run', 'build:ui']);
}

function validUiManifest(expectedVersion) {
  if (!existsSync(UI_MANIFEST) || !existsSync(join(UI_DIST, 'index.html'))) return false;
  try {
    const manifest = JSON.parse(readFileSync(UI_MANIFEST, 'utf8'));
    return manifest.app_version === expectedVersion && Boolean(manifest.frontend_build_id);
  } catch {
    return false;
  }
}

function validateUiDist(expectedVersion) {
  const fixHint = args.buildMissing ? '' : `\nFix: ${buildMissingSuggestion()}`;
  if (!existsSync(UI_DIST)) fail(`ui/dist is missing. Run \`bun run build:ui\` first, or pass \`--build-missing\`.${fixHint}`);
  if (!existsSync(join(UI_DIST, 'index.html'))) fail(`ui/dist/index.html is missing. Run \`bun run build:ui\` first.${fixHint}`);
  if (!existsSync(UI_MANIFEST)) fail(`ui/dist/nomifun-build.json is missing. Run \`bun run build:ui\` first.${fixHint}`);

  const manifest = readJson(UI_MANIFEST, 'ui/dist/nomifun-build.json');
  if (manifest.schema !== 1) fail('ui/dist/nomifun-build.json has an unsupported schema.');
  if (manifest.app_version !== expectedVersion) {
    fail(`ui/dist app_version is ${manifest.app_version}, but package.json is ${expectedVersion}. Run \`bun run build:ui\`.`);
  }
  if (!manifest.frontend_build_id || typeof manifest.frontend_build_id !== 'string') {
    fail('ui/dist/nomifun-build.json has no frontend_build_id. Run `bun run build:ui`.');
  }
  return manifest;
}

function resolveBinary(explicitPath) {
  const candidates = explicitPath
    ? [resolve(ROOT, explicitPath)]
    : [
        join(ROOT, 'target', 'release', 'nomifun-web'),
        join(ROOT, 'target', 'x86_64-unknown-linux-gnu', 'release', 'nomifun-web'),
        join(ROOT, 'target', 'aarch64-unknown-linux-gnu', 'release', 'nomifun-web'),
      ];
  return candidates.find((candidate) => existsSync(candidate)) || '';
}

function validateLinuxBinary(path) {
  let fd;
  const header = Buffer.alloc(4);
  try {
    fd = openSync(path, 'r');
    readSync(fd, header, 0, header.length, 0);
  } catch (error) {
    fail(`failed to read ${relative(path)}: ${error.message}`);
  } finally {
    if (fd !== undefined) closeSync(fd);
  }
  if (!(header[0] === 0x7f && header[1] === 0x45 && header[2] === 0x4c && header[3] === 0x46)) {
    fail(`${relative(path)} is not a Linux ELF binary. Use the full Dockerfile on non-Linux hosts.`);
  }
}

function validateKnownRuntimeGlibc(path) {
  const required = maxGlibcRequirement(path);
  if (!required) return;

  const provided = knownRuntimeGlibc(args.runtimeImage);
  if (!provided) {
    log(`binary requires up to GLIBC_${required}; ensure ${args.runtimeImage} provides that version or newer`);
    return;
  }

  if (compareVersions(required, provided) > 0) {
    fail(
      `${relative(path)} requires GLIBC_${required}, but ${args.runtimeImage} is known to provide about GLIBC_${provided}. Use \`--runtime-image ubuntu:26.04\`, build the binary inside the full Dockerfile, or choose a newer runtime image.`
    );
  }
}

function maxGlibcRequirement(path) {
  const fd = openSync(path, 'r');
  const chunk = Buffer.alloc(1024 * 1024);
  let carry = '';
  let max = '';
  try {
    while (true) {
      const bytes = readSync(fd, chunk, 0, chunk.length, null);
      if (bytes === 0) break;
      const text = carry + chunk.subarray(0, bytes).toString('latin1');
      const matches = text.matchAll(/GLIBC_(\d+\.\d+)/g);
      for (const match of matches) {
        const version = match[1];
        if (!max || compareVersions(version, max) > 0) max = version;
      }
      carry = text.slice(-32);
    }
  } finally {
    closeSync(fd);
  }
  return max;
}

function knownRuntimeGlibc(image) {
  const normalized = image.toLowerCase();
  if (/^ubuntu:(26\.04|resolute)(?:$|[-@])/.test(normalized)) return '2.43';
  if (/^ubuntu:(26\.10|rolling)(?:$|[-@])/.test(normalized)) return '2.43';
  if (/^ubuntu:(24\.04|noble)(?:$|[-@])/.test(normalized)) return '2.39';
  if (/^ubuntu:(22\.04|jammy)(?:$|[-@])/.test(normalized)) return '2.35';
  if (/^debian:(bookworm|12)(?:$|[-@])/.test(normalized)) return '2.36';
  if (/^debian:(bullseye|11)(?:$|[-@])/.test(normalized)) return '2.31';
  if (/^debian:(trixie|13)(?:$|[-@])/.test(normalized)) return '2.41';
  return '';
}

function compareVersions(a, b) {
  const [aMajor, aMinor] = a.split('.').map((part) => Number(part));
  const [bMajor, bMinor] = b.split('.').map((part) => Number(part));
  if (aMajor !== bMajor) return aMajor - bMajor;
  return aMinor - bMinor;
}

function validateBinaryMatchesUi(path, buildId) {
  if (!fileContainsUtf8(path, buildId)) {
    if (args.binary) {
      fail(
        `${relative(path)} does not embed the current ui/dist frontend_build_id. Remove \`--binary\` and pass \`--build-missing\`, or point \`--binary\` at the matching release binary.`
      );
    }
    if (args.buildMissing) {
      run('cargo', ['build', '--release', '--locked', '-p', 'nomifun-web']);
      if (fileContainsUtf8(path, buildId)) return;
    }
    fail(
      `${relative(path)} does not embed the current ui/dist frontend_build_id. Run \`cargo build --release --locked -p nomifun-web\` after \`bun run build:ui\`, or pass \`--build-missing\`.\nFix: ${buildMissingSuggestion()}`
    );
  }
}

function fileContainsUtf8(path, text) {
  const needle = Buffer.from(text, 'utf8');
  const chunk = Buffer.alloc(1024 * 1024);
  let carry = Buffer.alloc(0);
  let fd;
  try {
    fd = openSync(path, 'r');
    while (true) {
      const bytes = readSync(fd, chunk, 0, chunk.length, null);
      if (bytes === 0) return false;
      const haystack = Buffer.concat([carry, chunk.subarray(0, bytes)]);
      if (haystack.indexOf(needle) >= 0) return true;
      carry = haystack.subarray(Math.max(0, haystack.length - needle.length + 1));
    }
  } catch {
    return false;
  } finally {
    if (fd !== undefined) closeSync(fd);
  }
}

function stageContext(binaryPath) {
  rmSync(CONTEXT, { recursive: true, force: true });
  mkdirSync(CONTEXT, { recursive: true });
  copyFileSync(binaryPath, join(CONTEXT, 'nomifun-web'));
  chmodSync(join(CONTEXT, 'nomifun-web'), 0o755);
  cpSync(UI_DIST, join(CONTEXT, 'web'), {
    dereference: true,
    errorOnExist: false,
    force: true,
    recursive: true,
  });
}

function run(command, commandArgs) {
  log(`${command} ${commandArgs.map(shellQuote).join(' ')}`);
  const result = spawnSync(command, commandArgs, { cwd: ROOT, stdio: 'inherit' });
  if (result.error) fail(`failed to run ${command}: ${result.error.message}`);
  if (result.status !== 0) process.exit(result.status ?? 1);
}

function shellQuote(value) {
  if (/^[A-Za-z0-9_./:@%+=,-]+$/.test(value)) return value;
  return `'${value.replaceAll("'", "'\\''")}'`;
}

function dockerCommandForDisplay(commandArgs) {
  const parts = args.sudo ? ['sudo', args.dockerBin, ...commandArgs] : [args.dockerBin, ...commandArgs];
  return parts.map(shellQuote).join(' ');
}

function buildMissingSuggestion() {
  const commandArgs = ['run', 'docker:prebuilt', '--'];
  if (args.tags.length) {
    for (const tag of args.tags) commandArgs.push('--tag', tag);
  } else if (args.repository !== DEFAULT_REPOSITORY) {
    commandArgs.push('--repository', args.repository);
  }
  if (args.bunImage !== DEFAULT_BUN_IMAGE) commandArgs.push('--bun-image', args.bunImage);
  if (args.nodeImage !== DEFAULT_NODE_IMAGE) commandArgs.push('--node-image', args.nodeImage);
  if (args.runtimeImage !== DEFAULT_RUNTIME_IMAGE) commandArgs.push('--runtime-image', args.runtimeImage);
  if (args.aptMirror) commandArgs.push('--apt-mirror', args.aptMirror);
  if (args.dockerBin !== 'docker') commandArgs.push('--docker', args.dockerBin);
  commandArgs.push('--build-missing');
  if (args.sudo) commandArgs.push('--sudo');
  return ['bun', ...commandArgs].map(shellQuote).join(' ');
}

function relative(path) {
  return path.startsWith(ROOT) ? path.slice(ROOT.length + 1) : path;
}

function log(message) {
  console.log(`${TAG} ${message}`);
}

function fail(message) {
  console.error(`${TAG} ERROR: ${message}`);
  process.exit(1);
}
