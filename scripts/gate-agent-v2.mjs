// Only contract generation/integrity remains from the historical stage gate.
// Wrapper-era C1-C9/AP-7 evidence does not certify the in-process multi-Engine
// product. Retired commands fail before spawning tools or writing reports.
import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
if (args.length !== 1 || args[0] !== 'contract-closure') {
  console.error(
    'Only contract-closure remains supported. Historical AP-7/C1-C9 and --self-test ' +
    'were retired with the external Wrapper host; they cannot produce CAR acceptance. ' +
    'See docs/specs/2026-09-06-coding-agent-runtime-internalization/' +
    '07-validation-release-and-cutover.zh.md for the current, separately required evidence.'
  );
  process.exit(2);
}

const commands = [];
const failures = [];

const requiredFiles = [
  'crates/backend/nomifun-agent-contracts/Cargo.toml',
  'crates/backend/nomifun-agent-contracts/src/digest.rs',
  'crates/backend/nomifun-agent-contracts/src/package.rs',
  'crates/backend/nomifun-agent-contracts/src/preset.rs',
  'crates/backend/nomifun-agent-contracts/src/remote.rs',
  'crates/backend/nomifun-agent-contracts/src/runtime.rs',
  'crates/backend/nomifun-agent-contracts/src/engine_features.rs',
  'crates/backend/nomifun-agent-contracts/contracts/inventory/current-composition.json',
  'crates/backend/nomifun-agent-contracts/contracts/historical/agent-v2/README.md',
  'crates/backend/nomifun-agent-contracts/src/session.rs',
  'crates/backend/nomifun-agent-contracts/src/event.rs',
  'crates/backend/nomifun-agent-contracts/src/deletion.rs',
  'crates/backend/nomifun-agent-contracts/src/validation.rs',
  'crates/backend/nomifun-agent-contracts/src/manifest.rs',
  'crates/backend/nomifun-agent-contracts/src/bin/agent-v2-contract.rs',
  'crates/backend/nomifun-agent-contracts/src/schema.rs',
  'crates/backend/nomifun-agent-contracts/schema/0001_agent_store.sql',
  'crates/backend/nomifun-agent-contracts/contracts/generated/schemas.json',
  'crates/backend/nomifun-agent-contracts/contracts/generated/canonical-agent-store-schema-manifest.envelope.json',
  'crates/backend/nomifun-agent-contracts/contracts/generated/contract-digest-ledger.envelope.json',
  'crates/backend/nomifun-agent-contracts/contracts/validation/d025-compatibility-fixture-reference.envelope.json',
  'docs/specs/2026-08-28-agent-capability-platform-v2/C0-WRITE-MANIFESTS.json',
  'docs/specs/2026-08-28-agent-capability-platform-v2/README.zh.md',
  'docs/specs/2026-08-28-agent-capability-platform-v2/05-system-capability-replacement-foundation.zh.md',
  'docs/specs/2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md',
];

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
  'crates/backend/nomifun-agent-contracts/contracts/target-packages/first-party-agent-modules.v1.json',
  'crates/backend/nomifun-agent-contracts/contracts/engine/platform-feature-inventory.payload.json',
  'crates/backend/nomifun-agent-contracts/contracts/presets/official-agent-seed-manifest.payload.json',
  'crates/backend/nomifun-agent-contracts/contracts/events/session-event-registry.json',
  'crates/backend/nomifun-agent-contracts/contracts/events/error-registry.json',
  'crates/backend/nomifun-agent-contracts/contracts/validation/platform-validation-manifest.payload.json',
  'crates/backend/nomifun-agent-contracts/contracts/generated/canonical-agent-store-schema-manifest.envelope.json',
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
  !Array.isArray(closurePayload.decisions) ||
  closurePayload.decisions.length === 0 ||
  closurePayload.decisions.some((decision) => decision.status !== 'confirmed') ||
  closurePayload.unresolved_decisions?.length !== 0 ||
  closurePayload.production_behavior_included !== false
) {
  failures.push('Contract Closure payload is not a fully confirmed, G0-only input');
}

const sourceOnlyPaths = [
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
