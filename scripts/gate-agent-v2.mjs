import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const gateName = args[0];

if (!['contract-closure', 'c1-fullauto'].includes(gateName)) {
  console.error('usage: bun run gate:agent-v2 -- <contract-closure|c1-fullauto>');
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

if (gateName === 'c1-fullauto') {
  runC1Gate();
  writeGateReport('c1-fullauto');
  finishGate('c1-fullauto');
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

function runC1Gate() {
  const manifestPath =
    'docs/specs/2026-08-28-agent-capability-platform-v2/C1-WRITE-MANIFESTS.json';
  if (!statSafe(join(repoRoot, manifestPath))?.isFile()) {
    failures.push(`missing C1 write manifest: ${manifestPath}`);
    return;
  }

  const productionRoots = [
    'crates/agent/nomi-protocol/src',
    'crates/agent/nomi-protocol/tests',
    'crates/agent/nomi-cli/src',
    'crates/agent/nomi-agent/src',
    'crates/agent/nomi-agent/tests',
    'crates/agent/nomi-browser/src',
    'crates/agent/nomi-browser/tests',
    'crates/agent/nomi-config/src',
    'crates/agent/nomi-mcp/src',
    'crates/agent/nomi-skills/src',
    'crates/agent/nomi-tools/src',
    'crates/agent/nomi-types/src',
    'crates/backend/nomifun-common/src',
    'crates/backend/nomifun-api-types/src',
    'crates/backend/nomifun-agent-execution/src',
    'crates/backend/nomifun-agent-execution/tests',
    'crates/backend/nomifun-ai-agent/src',
    'crates/backend/nomifun-ai-agent/tests',
    'crates/backend/nomifun-conversation/src',
    'crates/backend/nomifun-cron/src',
    'crates/backend/nomifun-cron/tests',
    'crates/backend/nomifun-channel/src',
    'crates/backend/nomifun-channel/tests',
    'crates/backend/nomifun-db/src',
    'crates/backend/nomifun-db/tests',
    'crates/backend/nomifun-idmm/src',
    'crates/backend/nomifun-gateway/src',
    'crates/backend/nomifun-public/src',
    'crates/backend/nomifun-requirement/src',
    'crates/backend/nomifun-requirement/tests',
    'crates/backend/nomifun-app/src',
    'crates/backend/nomifun-app/tests',
    'crates/backend/nomifun-companion/src',
    'crates/backend/nomifun-robot/src',
    'ui/src',
  ];
  const forbiddenPatterns = [
    [/\bSessionMode\b/, 'legacy execution mode type'],
    [/\bApprovalScope\b/, 'agent tool approval scope'],
    [/\bToolApprovalManager\b/, 'agent tool approval manager'],
    [/\bToolConfirmer\b/, 'agent tool confirmer'],
    [/session\/set_mode\b/, 'runtime mode command'],
    [/\bneeds_confirmation\b/, 'agent confirmation response field'],
    [/\bawaiting_approval\b/, 'agent approval lifecycle state'],
    [/\bwaiting_confirmation\b/, 'agent confirmation lifecycle state'],
    [/\brequire_approval\b/, 'plan approval policy'],
    [/\bagentMode(?!l)/, 'agent mode product surface'],
    [/\bpermission-review\b/, 'agent permission review surface'],
    [/\bConfirmRequest\b/, 'agent confirmation request DTO'],
    [/AgentStreamEvent::Permission\b/, 'agent permission stream event'],
    [/\bPermissionConfirm\b/, 'IDMM permission confirmation payload'],
    [/WakeAction::Confirm\b/, 'IDMM confirmation action'],
    [/DecisionSource::Permission\b/, 'IDMM permission decision source'],
    [/\bauto_approve_invocation\b/, 'per-invocation approval bypass'],
    [/\bauto_approve\b/, 'global agent approval bypass'],
  ];
  for (const root of productionRoots) {
    const files = collectFiles(join(repoRoot, root), '');
    for (const file of files) {
      if (!/\.(rs|ts|tsx|json|toml)$/.test(file)) continue;
      const normalized = relative(repoRoot, file).replaceAll('\\', '/');
      const source = readFileSync(file, 'utf8');
      for (const [pattern, label] of forbiddenPatterns) {
        if (!pattern.test(source)) continue;
        failures.push(`${normalized}: ${label} residual (${pattern.source})`);
      }
    }
  }

  const deletedRoutes = [
    '/confirmations',
    '/approvals/check',
    '\\bsession_mode\\b',
    'auto_edit',
    'permissionDefault',
    'permissionFullAuto',
  ];
  for (const route of deletedRoutes) {
    const result = spawnSync('rg', ['-n', '--hidden', '-g', '!target/**', '-g', '!node_modules/**', route, ...productionRoots], {
      cwd: repoRoot,
      encoding: 'utf8',
      shell: process.platform === 'win32',
      stdio: 'pipe',
    });
    if (result.status === 0 && result.stdout.trim()) {
      failures.push(`C1 deleted surface remains reachable: ${route}`);
    }
  }

  run('cargo', [
    'check',
    '-p',
    'nomi-protocol',
    '-p',
    'nomi-agent',
    '-p',
    'nomi-browser',
    '-p',
    'nomi-cli',
    '-p',
    'nomi-config',
    '-p',
    'nomi-mcp',
    '-p',
    'nomi-skills',
    '-p',
    'nomi-tools',
    '-p',
    'nomi-types',
    '-p',
    'nomifun-common',
    '-p',
    'nomifun-api-types',
    '-p',
    'nomifun-agent-execution',
    '-p',
    'nomifun-ai-agent',
    '-p',
    'nomifun-conversation',
    '-p',
    'nomifun-cron',
    '-p',
    'nomifun-channel',
    '-p',
    'nomifun-idmm',
    '-p',
    'nomifun-gateway',
    '-p',
    'nomifun-public',
    '-p',
    'nomifun-requirement',
    '-p',
    'nomifun-app',
  ]);
  run('bun', [
    'test',
    'ui/src/common/adapter/ipcBridge.idmm-intervention-wire.test.ts',
    'ui/src/renderer/components/settings/SettingsModal/contents/BrowserUseSettingsContent.test.ts',
    'ui/src/renderer/pages/conversation/Messages/turnProcessState.test.ts',
    'ui/src/renderer/pages/conversation/execution/readOnlyConversation.structure.test.ts',
    'ui/src/renderer/pages/conversation/platforms/nomi/NomiSendBoxLayout.structure.test.ts',
    'ui/src/renderer/pages/conversation/platforms/nomi/useNomiMessage.test.ts',
    'ui/src/renderer/pages/conversation/components/IdmmControl.structure.test.ts',
  ]);
  run('bun', ['run', 'check:i18n']);
  run('git', ['diff', '--check']);
}

function writeGateReport(name) {
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
    name
  );
  mkdirSync(reportDir, { recursive: true });
  const report = {
    schema_version: '1.0.0',
    gate_name: name,
    source_sha: sourceSha,
    evidence_kind: 'informational',
    status: failures.length === 0 ? 'pass' : 'fail',
    commands,
    failures,
  };
  writeFileSync(join(reportDir, 'summary.json'), `${JSON.stringify(report, null, 2)}\n`);
}

function finishGate(name) {
  if (failures.length > 0) {
    console.error(`agent-v2 ${name} failed (${failures.length} issue(s))`);
    for (const failure of failures) {
      console.error(`- ${failure}`);
    }
    process.exit(1);
  }
  console.log(`agent-v2 ${name} passed`);
  process.exit(0);
}
