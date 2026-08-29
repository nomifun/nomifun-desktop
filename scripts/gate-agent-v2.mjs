import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const gateName = args[0];

if (gateName !== 'contract-closure') {
  console.error('usage: bun run gate:agent-v2 -- contract-closure');
  process.exit(2);
}

const requiredFiles = [
  'crates/backend/nomifun-agent-contracts/Cargo.toml',
  'crates/backend/nomifun-agent-contracts/src/digest.rs',
  'crates/backend/nomifun-agent-contracts/src/package.rs',
  'crates/backend/nomifun-agent-contracts/src/preset.rs',
  'crates/backend/nomifun-agent-contracts/src/remote.rs',
  'crates/backend/nomifun-agent-contracts/src/runtime.rs',
  'crates/backend/nomifun-agent-contracts/src/session.rs',
  'crates/backend/nomifun-agent-contracts/src/event.rs',
  'crates/backend/nomifun-agent-contracts/src/deletion.rs',
  'crates/backend/nomifun-agent-contracts/src/validation.rs',
  'crates/backend/nomifun-agent-contracts/src/manifest.rs',
  'crates/backend/nomifun-agent-contracts/src/bin/agent-v2-contract.rs',
  'crates/backend/nomifun-agent-contracts/src/schema.rs',
  'crates/backend/nomifun-agent-contracts/schema/0001_fresh_v4.sql',
  'crates/backend/nomifun-agent-contracts/contracts/generated/schemas.json',
  'crates/backend/nomifun-agent-contracts/contracts/generated/canonical-v4-schema-manifest.envelope.json',
  'crates/backend/nomifun-agent-contracts/contracts/generated/contract-digest-ledger.envelope.json',
  'crates/backend/nomifun-agent-contracts/contracts/validation/d025-compatibility-fixture-reference.envelope.json',
  'docs/specs/2026-08-28-agent-capability-platform-v2/C0-WRITE-MANIFESTS.json',
  'docs/specs/2026-08-28-agent-capability-platform-v2/IMPLEMENTATION-STATUS.zh.md',
];

const commands = [];
const failures = [];

function run(command, commandArgs) {
  const startedAt = new Date().toISOString();
  const result = spawnSync(command, commandArgs, {
    cwd: repoRoot,
    encoding: 'utf8',
    shell: process.platform === 'win32',
    stdio: 'pipe',
  });
  commands.push({
    command: [command, ...commandArgs].join(' '),
    started_at: startedAt,
    exit_code: result.status ?? 1,
    stdout: result.stdout,
    stderr: result.stderr,
  });
  if (result.status !== 0) {
    failures.push(`${command} ${commandArgs.join(' ')}`);
  }
}

function collectFiles(root, suffix) {
  const output = [];
  if (!statSafe(root)?.isDirectory()) {
    return output;
  }
  for (const entry of readdirSync(root)) {
    const path = join(root, entry);
    const stat = statSync(path);
    if (stat.isDirectory()) {
      output.push(...collectFiles(path, suffix));
    } else if (path.endsWith(suffix)) {
      output.push(path);
    }
  }
  return output;
}

function statSafe(path) {
  try {
    return statSync(path);
  } catch {
    return null;
  }
}

for (const file of requiredFiles) {
  if (!statSafe(join(repoRoot, file))?.isFile()) {
    failures.push(`missing required artifact: ${file}`);
  }
}

const contractRoot = join(repoRoot, 'crates/backend/nomifun-agent-contracts/contracts');
const jsonFiles = collectFiles(contractRoot, '.json');
if (jsonFiles.length === 0) {
  failures.push('no contract JSON payloads found');
}

const absoluteWindowsPath = /(?:^|["'\s])[A-Za-z]:[\\/]/m;
const localFileUri = /file:\/\//i;
for (const file of jsonFiles) {
  const source = readFileSync(file, 'utf8');
  try {
    JSON.parse(source);
  } catch (error) {
    failures.push(`${relative(repoRoot, file)}: invalid JSON: ${error.message}`);
    continue;
  }
  if (absoluteWindowsPath.test(source) || localFileUri.test(source)) {
    failures.push(`${relative(repoRoot, file)}: contains a machine-local path`);
  }
}

const canonicalPayloadFiles = [
  'crates/backend/nomifun-agent-contracts/contracts/closure/contract-closure.v1.json',
  'crates/backend/nomifun-agent-contracts/contracts/target-packages/target-first-party-contributions.v1.json',
  'crates/backend/nomifun-agent-contracts/contracts/runtime/coding-runtime-feature-inventory.payload.json',
  'crates/backend/nomifun-agent-contracts/contracts/presets/official-preset-seed-manifest.payload.json',
  'crates/backend/nomifun-agent-contracts/contracts/events/session-event-registry.json',
  'crates/backend/nomifun-agent-contracts/contracts/events/error-registry.json',
  'crates/backend/nomifun-agent-contracts/contracts/validation/platform-validation-manifest.payload.json',
  'crates/backend/nomifun-agent-contracts/contracts/generated/canonical-v4-schema-manifest.envelope.json',
  'crates/backend/nomifun-agent-contracts/contracts/generated/contract-digest-ledger.envelope.json',
];
const obviousPlaceholderDigest = /"([0-9a-f])\1{63}"/i;
for (const file of canonicalPayloadFiles) {
  const source = readFileSync(join(repoRoot, file), 'utf8');
  if (obviousPlaceholderDigest.test(source)) {
    failures.push(`${file}: contains an obvious placeholder digest`);
  }
}

const manifestSource = readFileSync(
  join(repoRoot, 'crates/backend/nomifun-agent-contracts/Cargo.toml'),
  'utf8'
);
for (const forbiddenDependency of [
  'nomi-agent',
  'nomifun-ai-agent',
  'nomifun-app',
  'nomifun-conversation',
  'nomifun-db',
  'nomifun-gateway',
]) {
  if (manifestSource.includes(forbiddenDependency)) {
    failures.push(`canonical contract crate depends on legacy/product crate ${forbiddenDependency}`);
  }
}

run('cargo', [
  'run',
  '-p',
  'nomifun-agent-contracts',
  '--bin',
  'agent-v2-contract',
  '--',
  'check',
]);
run('cargo', ['test', '-p', 'nomifun-agent-contracts']);
run('git', ['diff', '--check']);

const closurePayload = JSON.parse(
  readFileSync(
    join(
      repoRoot,
      'crates/backend/nomifun-agent-contracts/contracts/closure/contract-closure.v1.json'
    ),
    'utf8'
  )
);
if (
  closurePayload.decisions?.length !== 28 ||
  closurePayload.unresolved_decisions?.length !== 0 ||
  closurePayload.production_behavior_included !== false
) {
  failures.push('Contract Closure payload is not a fully confirmed, G0-only input');
}

const sourceOnlyPaths = [
  'crates/backend/nomifun-agent-contracts/contracts/runtime/codex-runtime-release-input.json',
  'crates/backend/nomifun-agent-contracts/contracts/validation/platform-validation-manifest.payload.json',
];
for (const file of sourceOnlyPaths) {
  const source = readFileSync(join(repoRoot, file), 'utf8');
  if (/\b(status|evidence|logs?|summary)\b\s*:/i.test(source)) {
    failures.push(`${file}: pre-run input contains runtime output fields`);
  }
}

const shaResult = spawnSync('git', ['rev-parse', 'HEAD'], {
  cwd: repoRoot,
  encoding: 'utf8',
  shell: process.platform === 'win32',
});
const sourceSha = shaResult.status === 0 ? shaResult.stdout.trim() : 'unknown-source';
const reportDir = join(
  repoRoot,
  'build.noindex/agent-capability-v2',
  sourceSha,
  'contract-closure'
);
mkdirSync(reportDir, { recursive: true });

const report = {
  schema_version: '1.0.0',
  gate_name: 'contract-closure',
  source_sha: sourceSha,
  evidence_kind: 'informational',
  status: failures.length === 0 ? 'pass' : 'fail',
  contract_json_files: jsonFiles.map((file) => relative(repoRoot, file).replaceAll('\\', '/')),
  required_files: requiredFiles,
  commands,
  failures,
};
writeFileSync(join(reportDir, 'summary.json'), `${JSON.stringify(report, null, 2)}\n`);

if (failures.length > 0) {
  console.error(`agent-v2 contract closure failed (${failures.length} issue(s))`);
  for (const failure of failures) {
    console.error(`- ${failure}`);
  }
  process.exit(1);
}

console.log(`agent-v2 contract closure passed (${jsonFiles.length} JSON payloads)`);
