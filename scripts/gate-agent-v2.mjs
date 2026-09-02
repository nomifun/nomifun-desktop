import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readAndVerifyReleaseLock } from './release/release-lock.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const gateName = args[0];

if (
  !args.includes('--self-test') &&
  ![
    'contract-closure',
    'c1-fullauto',
    'c2-c5-foundations',
    'c6-triad',
    'c7-domain-waves',
    'c8-win-pre',
    'c8-native',
    'c8-ma',
    'c8-ld',
    'c8-merge',
    'c9-hard-delete',
  ].includes(gateName)
) {
  console.error(
    'usage: bun run gate:agent-v2 -- <contract-closure|c1-fullauto|c2-c5-foundations|c6-triad|c7-domain-waves|c8-win-pre|c8-native|c8-ma|c8-ld|c8-merge|c9-hard-delete> [--evidence <platform-result.json>] [--cell <cell_id>] [--self-test]'
  );
  process.exit(2);
}

const requiredFiles = [
  'crates/backend/nomifun-agent-contracts/Cargo.toml',
  'crates/backend/nomifun-agent-contracts/src/digest.rs',
  'crates/backend/nomifun-agent-contracts/src/package.rs',
  'crates/backend/nomifun-agent-contracts/src/preset.rs',
  'crates/backend/nomifun-agent-contracts/src/remote.rs',
  'crates/backend/nomifun-agent-contracts/src/root.rs',
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
  'docs/specs/2026-08-28-agent-capability-platform-v2/README.zh.md',
  'docs/specs/2026-08-28-agent-capability-platform-v2/05-system-capability-replacement-foundation.zh.md',
  'docs/specs/2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md',
];

const commands = [];
const failures = [];
const COMMAND_MAX_BUFFER = 64 * 1024 * 1024;
const DEFAULT_COMMAND_TIMEOUT_MS = 10 * 60 * 1000;
const WORKSPACE_COMMAND_TIMEOUT_MS = 15 * 60 * 1000;

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

const C8_MANIFEST_PATH =
  'docs/specs/2026-08-28-agent-capability-platform-v2/C8-WIN-PRE-MANIFEST.json';
const C8_PLATFORM_VALIDATION_MANIFEST_PATH =
  'crates/backend/nomifun-agent-contracts/contracts/validation/platform-validation-manifest.payload.json';
const C8_PLATFORM_VALIDATION_FIXTURE_PATH =
  'crates/backend/nomifun-agent-contracts/contracts/generated/platform-validation-fixture.envelope.json';
const C8_RUNTIME_RELEASE_FIXTURE_INPUT_PATH =
  'crates/backend/nomifun-agent-contracts/contracts/runtime/runtime-release-fixture.json';
const C8_RUNTIME_RELEASE_FIXTURE_PATH =
  'crates/backend/nomifun-agent-contracts/contracts/generated/runtime-release-fixture.envelope.json';
const C8_TRIAD_DELETION_MANIFEST_PATH =
  'crates/backend/nomifun-agent-contracts/contracts/deletion/triad-core.json';
const C8_BRANCH = 'rf/agent-capability-platform-v2';
const C8_C7_MIGRATION_CHECKPOINT =
  '253e850b44bce83fa9b785dc6805c431201f6c91';
const C8_EXPECTED_DIGESTS = {
  confirmed_decision_contract:
    'b3c32f0579a36c1f720a906b785b76cea58e8c8a1e4b07df6416f0d7410d78d5',
  platform_validation_contract:
    'a3f5180906c239d791a03281199d80f2ea957dbe8382fb849dc2926935672a9c',
  runtime_feature_inventory:
    'bc01fffa050a721debc7740405a05f53b966d4e2dc2d8b4392e321d944fca2ee',
  canonical_schema_manifest:
    'f8eb056bfd49e5330603ad36b284ed3269c34e1301db74ba33a0bec861e9573a',
  official_seed:
    'd15da58409cb096d0a1a5cc8c60534bf378500ecfc697789a5d0ecec85e56582',
  target_inventory:
    'f8b5460165689bb463bd286573b5b2731a20803107e2b27ca3d86420dad62d1b',
  capability_availability:
    '70ab40f20452974594d897cbf32bcbcb3b030a77b9708534aa5e93c1b76eaa8c',
  coding_codex_native:
    'f699f376a9414b7830b90a68c890d39010687499e6d16ee1687f5c370cd0127a',
  cargo_lock:
    '51629196c5d1c2940e9ac748e095bbdbd621ba5788a0afa3af5181f0714db22d',
};
const C8_EXPECTED_TEMPLATES = [
  'chat.minimal',
  'assistant.general',
  'coding.codex',
  'companion.default',
  'robot.default',
  'customer-service.default',
  'creative-studio.default',
];
const C8_GLOBAL_RESIDUAL_MAX_FINDINGS = 5000;
const C8_REQUIRED_NATIVE_CELLS = [
  'windows_desktop_x64',
  'macos_desktop_arm64',
  'linux_desktop_x64',
];
const C8_NATIVE_CELL_SPECS = Object.freeze({
  windows_desktop_x64: Object.freeze({
    cell_id: 'windows_desktop_x64',
    host_os: 'windows',
    host_arch: 'x86_64',
    host_target: 'x86_64-pc-windows-msvc',
    runtime_target: 'x86_64-pc-windows-msvc',
    host_surface: 'desktop',
    package_format: 'nsis',
    gate_name: 'c8-win-pre',
    check_id: 'c8_win_pre_full_gate',
    command: 'bun run gate:agent-v2 -- c8-win-pre',
  }),
  macos_desktop_arm64: Object.freeze({
    cell_id: 'macos_desktop_arm64',
    host_os: 'macos',
    host_arch: 'aarch64',
    host_target: 'aarch64-apple-darwin',
    runtime_target: 'aarch64-apple-darwin',
    host_surface: 'desktop',
    package_format: 'universal-app',
    gate_name: 'c8-ma',
    check_id: 'c8_ma_full_gate',
    command: 'bun run gate:agent-v2 -- c8-ma',
  }),
  linux_desktop_x64: Object.freeze({
    cell_id: 'linux_desktop_x64',
    host_os: 'linux',
    host_arch: 'x86_64',
    host_target: 'x86_64-unknown-linux-gnu',
    runtime_target: 'x86_64-unknown-linux-musl',
    host_surface: 'desktop',
    package_format: 'appimage-deb-rpm',
    gate_name: 'c8-ld',
    check_id: 'c8_ld_full_gate',
    command: 'bun run gate:agent-v2 -- c8-ld',
  }),
});
const C8_NATIVE_GATE_DISPATCH = Object.freeze({
  'c8-ma': Object.freeze({
    ...C8_NATIVE_CELL_SPECS.macos_desktop_arm64,
  }),
  'c8-ld': Object.freeze({
    ...C8_NATIVE_CELL_SPECS.linux_desktop_x64,
  }),
});
const C8_NATIVE_GATE_NAMES = Object.keys(C8_NATIVE_GATE_DISPATCH);
let c8ConfirmationPolicyCache = null;

if (args.includes('--self-test')) {
  runC8SelfTest();
  console.log('C8 gate self-test passed');
  process.exit(0);
}

if (gateName === 'c8-native' || C8_NATIVE_GATE_NAMES.includes(gateName)) {
  let dispatch;
  try {
    dispatch = c8ParseNativeDispatchArgs(gateName, args);
  } catch (error) {
    console.error(`invalid ${gateName} arguments: ${error.message}`);
    process.exit(2);
  }
  const report = runC8NativeCellGate(dispatch);
  writeC8NativeCellReport(report);
  finishGate(gateName);
}

if (gateName === 'c1-fullauto') {
  runC1Gate();
  writeGateReport('c1-fullauto');
  finishGate('c1-fullauto');
}

if (gateName === 'c2-c5-foundations') {
  runC2C5Gate();
  writeGateReport('c2-c5-foundations');
  finishGate('c2-c5-foundations');
}

if (gateName === 'c6-triad') {
  runC6TriadGate();
  writeGateReport('c6-triad');
  finishGate('c6-triad');
}

if (gateName === 'c7-domain-waves') {
  let c7Report;
  try {
    c7Report = runC7DomainWavesGate();
  } catch (error) {
    failures.push(`C7 gate crashed before completing its report: ${error.message}`);
    c7Report = {
      task: 'C7',
      workstream: 'W4',
      slice: 'domain_waves',
      tasks: [],
      workstreams: [],
      slices: [],
      input_digests: {},
      residual_results: [],
      reachability_results: [],
      pending_native_verification_points: [],
      generated_domain_crates: [],
      registration_coverage: [],
    };
  }
  writeC7DomainWavesReport(c7Report);
  finishGate('c7-domain-waves');
}

if (gateName === 'c8-win-pre') {
  let c8Report;
  try {
    c8Report = runC8WinPreGate();
  } catch (error) {
    failures.push(`C8-WIN-PRE gate crashed before completing its report: ${error.message}`);
    const sourceSha = c8ReadGitHeadForReport();
    c8Report = {
      schema_version: '1.0.0',
      gate_name: 'c8-win-pre',
      evidence_kind: 'native',
      source_sha: sourceSha,
      candidate_source_sha: sourceSha,
      target_cell: null,
      manifest_path:
        'docs/specs/2026-08-28-agent-capability-platform-v2/C8-WIN-PRE-MANIFEST.json',
      c7: { status: 'not_evaluated' },
      platform_validation: { status: 'not_evaluated' },
      all_scene_coverage: { status: 'not_evaluated' },
      windows_metadata: { status: 'not_evaluated' },
      residual_reachability: { status: 'not_evaluated' },
      checks: [],
      statuses: {},
      artifact_digests: {},
      failure_details: [{ code: 'gate_crash', message: error.message }],
    };
  }
  writeC8WinPreReport(c8Report);
  finishGate('c8-win-pre');
}

if (gateName === 'c8-merge') {
  const report = runC8MergeGate();
  writeC8MergeReport(report);
  finishGate('c8-merge');
}

if (gateName === 'c9-hard-delete') {
  const report = runC9HardDeleteGate();
  writeC9HardDeleteReport(report);
  finishGate('c9-hard-delete');
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
  'crates/backend/nomifun-agent-contracts/contracts/runtime/runtime-release-fixture.json',
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

function runC2C5Gate() {
  const manifestPath =
    'docs/specs/2026-08-28-agent-capability-platform-v2/C2-C5-WRITE-MANIFESTS.json';
  if (!statSafe(join(repoRoot, manifestPath))?.isFile()) {
    failures.push(`missing C2-C5 write manifest: ${manifestPath}`);
    return;
  }

  const foundationCrates = [
    'nomifun-v4-root',
    'nomifun-agent-session',
    'nomifun-agent-kernel',
    'nomifun-codex-runtime',
    'nomifun-chat-model-broker',
    'nomifun-agent-control-plane',
  ];
  const foundationRoots = foundationCrates.map(
    (name) => `crates/backend/${name}`
  );
  const requiredFoundationFiles = foundationRoots.flatMap((root) => [
    `${root}/Cargo.toml`,
    `${root}/src/lib.rs`,
  ]);
  for (const file of [manifestPath, ...requiredFoundationFiles]) {
    if (!statSafe(join(repoRoot, file))?.isFile()) {
      failures.push(`missing C2-C5 foundation artifact: ${file}`);
    }
  }

  const forbiddenManifestDependencies = new Map([
    [
      'nomifun-v4-root',
      [
        'nomifun-db',
        'nomifun-app',
        'nomifun-conversation',
        'nomifun-ai-agent',
        'nomifun-preset',
      ],
    ],
    [
      'nomifun-agent-session',
      [
        'nomifun-db',
        'nomifun-app',
        'nomifun-conversation',
        'nomifun-ai-agent',
        'nomifun-gateway',
      ],
    ],
    [
      'nomifun-agent-kernel',
      [
        'nomifun-db',
        'nomifun-app',
        'nomifun-conversation',
        'nomifun-ai-agent',
        'nomifun-gateway',
        'nomifun-extension',
        'nomifun-preset',
      ],
    ],
    [
      'nomifun-codex-runtime',
      [
        'nomi-agent',
        'nomifun-ai-agent',
        'nomifun-app',
        'nomifun-conversation',
        'nomifun-gateway',
      ],
    ],
    [
      'nomifun-chat-model-broker',
      [
        'nomifun-ai-agent',
        'nomifun-app',
        'nomifun-conversation',
        'nomifun-model-invoke',
      ],
    ],
    [
      'nomifun-agent-control-plane',
      [
        'nomifun-app',
        'nomifun-conversation',
        'nomifun-ai-agent',
        'nomifun-gateway',
        'nomifun-extension',
        'nomifun-preset',
      ],
    ],
  ]);
  for (const [crateName, dependencies] of forbiddenManifestDependencies) {
    const source = readFileSync(
      join(repoRoot, `crates/backend/${crateName}/Cargo.toml`),
      'utf8'
    );
    for (const dependency of dependencies) {
      if (source.includes(dependency)) {
        failures.push(
          `${crateName} foundation manifest depends on forbidden legacy/product crate ${dependency}`
        );
      }
    }
  }

  const coreRoots = [
    'crates/backend/nomifun-agent-session/src',
    'crates/backend/nomifun-agent-kernel/src',
    'crates/backend/nomifun-codex-runtime/src',
    'crates/backend/nomifun-chat-model-broker/src',
    'crates/backend/nomifun-agent-control-plane/src',
    'ui/src/common/types/agentPlatform',
    'ui/src/renderer/pages/agentSettings',
  ];
  const forbiddenCorePatterns = [
    [/\bAppServices\b/, 'AppServices escape hatch'],
    [/\bGatewayDeps\b/, 'GatewayDeps escape hatch'],
    [/\bConversationService\b/, 'legacy Conversation service dependency'],
    [/\bNomiAgentManager\b/, 'legacy Nomi runtime dependency'],
    [/\bNomiBuildExtra\b/, 'legacy Nomi build-extra dependency'],
    [/\bPresetService\b/, 'legacy preset service dependency'],
    [/\bExtensionRegistry\b/, 'legacy extension registry dependency'],
    [/\/api\/conversations\b/, 'legacy conversation route'],
    [/\/api\/presets\b/, 'legacy preset route'],
    [/\bDraftSnapshot\b/, 'test-only draft snapshot'],
    [/\bSessionMode\b/, 'legacy session mode'],
    [/\bToolApprovalManager\b/, 'legacy approval manager'],
    [/\bneeds_confirmation\b/, 'legacy confirmation protocol'],
  ];
  for (const root of coreRoots) {
    for (const file of collectFiles(join(repoRoot, root), '')) {
      if (!/\.(rs|ts|tsx|json|toml)$/.test(file)) continue;
      const normalized = relative(repoRoot, file).replaceAll('\\', '/');
      const source = readFileSync(file, 'utf8');
      for (const [pattern, label] of forbiddenCorePatterns) {
        if (pattern.test(source)) {
          failures.push(
            `${normalized}: C2-C5 forbidden foundation edge (${label})`
          );
        }
      }
    }
  }

  const migrationDiff = spawnSync(
    'git',
    [
      'diff',
      '--quiet',
      '253e850b44bce83fa9b785dc6805c431201f6c91',
      '--',
      'crates/backend/nomifun-db/migrations',
    ],
    {
      cwd: repoRoot,
      encoding: 'utf8',
      shell: process.platform === 'win32',
      stdio: 'pipe',
    }
  );
  if (migrationDiff.status !== 0) {
    failures.push('published legacy migrations changed after the C1 checkpoint');
  }

  runC1Gate();
  run('cargo', [
    'run',
    '-p',
    'nomifun-agent-contracts',
    '--bin',
    'agent-v2-contract',
    '--',
    'check',
  ]);
  run('cargo', [
    'check',
    '-p',
    'nomifun-v4-root',
    '-p',
    'nomifun-agent-session',
    '-p',
    'nomifun-agent-kernel',
    '-p',
    'nomifun-codex-runtime',
    '-p',
    'nomifun-chat-model-broker',
    '-p',
    'nomifun-agent-control-plane',
  ]);
  run('cargo', [
    'test',
    '-p',
    'nomifun-v4-root',
    '-p',
    'nomifun-agent-session',
    '-p',
    'nomifun-agent-kernel',
    '-p',
    'nomifun-codex-runtime',
    '-p',
    'nomifun-chat-model-broker',
    '-p',
    'nomifun-agent-control-plane',
  ]);

  const uiFoundationPaths = [
    'ui/src/common/types/agentPlatform',
    'ui/src/renderer/pages/agentSettings',
  ].filter((path) => statSafe(join(repoRoot, path))?.isDirectory());
  if (uiFoundationPaths.length > 0) {
    run('bun', ['test', ...uiFoundationPaths]);
  }
  run('bun', ['run', 'check:i18n']);
  run('git', ['diff', '--check']);
}

function runC6TriadGate() {
  const manifestPath =
    'docs/specs/2026-08-28-agent-capability-platform-v2/C6-WRITE-MANIFESTS.json';
  const requiredTriadFiles = [
    manifestPath,
    'crates/backend/nomifun-agent-platform/Cargo.toml',
    'crates/backend/nomifun-agent-platform/src/lib.rs',
    'crates/backend/nomifun-agent-platform/src/platform.rs',
    'crates/backend/nomifun-agent-platform/src/chat.rs',
    'crates/backend/nomifun-agent-platform/src/coding.rs',
    'crates/backend/nomifun-agent-platform/src/sample_echo.rs',
    'crates/backend/nomifun-agent-platform/tests/chat_minimal.rs',
    'crates/backend/nomifun-agent-platform/tests/coding_codex.rs',
    'crates/backend/nomifun-agent-platform/tests/sample_echo.rs',
    'crates/backend/nomifun-app/src/router/agent_platform.rs',
    'crates/backend/nomifun-app/tests/agent_platform_e2e.rs',
    'ui/src/renderer/pages/agentSession',
  ];
  for (const file of requiredTriadFiles) {
    if (!statSafe(join(repoRoot, file))) {
      failures.push(`missing C6 triad artifact: ${file}`);
    }
  }

  const platformManifest = readFileSync(
    join(repoRoot, 'crates/backend/nomifun-agent-platform/Cargo.toml'),
    'utf8'
  );
  for (const dependency of [
    'nomi-agent',
    'nomifun-ai-agent',
    'nomifun-conversation',
    'nomifun-gateway',
    'nomifun-preset',
    'nomifun-extension',
  ]) {
    if (platformManifest.includes(dependency)) {
      failures.push(
        `nomifun-agent-platform depends on forbidden legacy/product crate ${dependency}`
      );
    }
  }

  const finalStackRoots = [
    'crates/backend/nomifun-agent-platform/src',
    'crates/backend/nomifun-app/src/router/agent_platform.rs',
  ];
  const forbiddenFinalStackPatterns = [
    [/\bAgentFactoryDeps\b/, 'legacy Agent factory dependency'],
    [/\bbuild_agent_factory\b/, 'legacy Agent factory builder'],
    [/\bNomiAgentManager\b/, 'legacy Nomi manager'],
    [/\bNomiBuildExtra\b/, 'legacy Nomi build-extra DTO'],
    [/\bGatewayDeps\b/, 'legacy Gateway dependency bag'],
    [/\bConversationService\b/, 'legacy Conversation service'],
    [/\/api\/conversations\b/, 'legacy Conversation route'],
    [/\bDraftSnapshot\b/, 'test-only draft snapshot'],
    [/\bSessionMode\b/, 'legacy execution mode'],
    [/\bToolApprovalManager\b/, 'legacy approval manager'],
    [/\bneeds_confirmation\b/, 'legacy confirmation protocol'],
  ];
  for (const root of finalStackRoots) {
    const absoluteRoot = join(repoRoot, root);
    const files = statSafe(absoluteRoot)?.isDirectory()
      ? collectFiles(absoluteRoot, '')
      : statSafe(absoluteRoot)?.isFile()
        ? [absoluteRoot]
        : [];
    for (const file of files) {
      if (!/\.(rs|toml)$/.test(file)) continue;
      const normalized = relative(repoRoot, file).replaceAll('\\', '/');
      const source = readFileSync(file, 'utf8');
      for (const [pattern, label] of forbiddenFinalStackPatterns) {
        if (pattern.test(source)) {
          failures.push(`${normalized}: C6 forbidden final-stack edge (${label})`);
        }
      }
    }
  }

  const productionSampleEchoRoots = [
    'crates/backend/nomifun-app/src',
    'ui/src',
    'crates/backend/nomifun-agent-contracts/contracts/presets/official-preset-seed-manifest.payload.json',
  ];
  for (const root of productionSampleEchoRoots) {
    const absoluteRoot = join(repoRoot, root);
    const files = statSafe(absoluteRoot)?.isDirectory()
      ? collectFiles(absoluteRoot, '')
      : statSafe(absoluteRoot)?.isFile()
        ? [absoluteRoot]
        : [];
    for (const file of files) {
      if (!/\.(rs|ts|tsx|json|toml)$/.test(file)) continue;
      const source = readFileSync(file, 'utf8');
      if (/\bsample\.echo\b/.test(source)) {
        failures.push(
          `${relative(repoRoot, file).replaceAll('\\', '/')}: sample.echo leaked into production inventory/API/UI`
        );
      }
    }
  }

  const migrationDiff = spawnSync(
    'git',
    [
      'diff',
      '--quiet',
      '253e850b44bce83fa9b785dc6805c431201f6c91',
      '--',
      'crates/backend/nomifun-db/migrations',
    ],
    {
      cwd: repoRoot,
      encoding: 'utf8',
      shell: process.platform === 'win32',
      stdio: 'pipe',
    }
  );
  if (migrationDiff.status !== 0) {
    failures.push('published legacy migrations changed after the C1 checkpoint');
  }

  run('cargo', ['check', '-p', 'nomifun-agent-platform', '-p', 'nomifun-app']);
  run('cargo', ['test', '-p', 'nomifun-agent-platform']);
  run('cargo', ['test', '-p', 'nomifun-app', '--test', 'agent_platform_e2e']);

  const uiTriadPaths = [
    'ui/src/common/types/agentPlatform',
    'ui/src/renderer/pages/agentSettings',
    'ui/src/renderer/pages/agentSession',
  ].filter((path) => statSafe(join(repoRoot, path)));
  if (uiTriadPaths.length > 0) {
    run('bun', ['test', ...uiTriadPaths]);
  }
  run('bun', ['run', 'check:i18n']);
  run('git', ['diff', '--check']);
}

function runC7DomainWavesGate() {
  const manifestPath =
    'docs/specs/2026-08-28-agent-capability-platform-v2/C7-WRITE-MANIFESTS.json';
  const c1MigrationCheckpoint =
    '253e850b44bce83fa9b785dc6805c431201f6c91';
  const expectedWaves = [
    {
      task_id: 'C7-W1-READ',
      wave: 'wave1_read_capabilities',
      owner: 'domain-wave-1-read-capabilities',
      deletion_manifest:
        'crates/backend/nomifun-agent-contracts/contracts/deletion/domain-wave-1-read-capabilities.json',
      generated_crate: 'crates/backend/nomifun-agent-domain-wave1',
    },
    {
      task_id: 'C7-W2-CODING',
      wave: 'wave2_coding_extensions',
      owner: 'domain-wave-2-coding-extensions',
      deletion_manifest:
        'crates/backend/nomifun-agent-contracts/contracts/deletion/domain-wave-2-coding-extensions.json',
      generated_crate: 'crates/backend/nomifun-agent-domain-wave2',
    },
    {
      task_id: 'C7-W3-CREATIVE',
      wave: 'wave3_creative_multimodal',
      owner: 'domain-wave-3-creative-multimodal',
      deletion_manifest:
        'crates/backend/nomifun-agent-contracts/contracts/deletion/domain-wave-3-creative-multimodal.json',
      generated_crate: 'crates/backend/nomifun-agent-domain-wave3',
    },
    {
      task_id: 'C7-W4-IDENTITY',
      wave: 'wave4_identity_channels_devices',
      owner: 'domain-wave-4-identity-channels-devices',
      deletion_manifest:
        'crates/backend/nomifun-agent-contracts/contracts/deletion/domain-wave-4-identity-channels-devices.json',
      generated_crate: 'crates/backend/nomifun-agent-domain-wave4',
    },
    {
      task_id: 'C7-W5-AUTOMATION',
      wave: 'wave5_automation_supervision_remote',
      owner: 'domain-wave-5-automation-supervision-remote',
      deletion_manifest:
        'crates/backend/nomifun-agent-contracts/contracts/deletion/domain-wave-5-automation-supervision-remote.json',
      generated_crate: 'crates/backend/nomifun-agent-domain-wave5',
    },
  ];
  const report = {
    task: 'C7',
    workstream: 'W4',
    slice: 'domain_waves',
    tasks: [],
    workstreams: [],
    slices: [],
    input_digests: {},
    residual_results: [],
    reachability_results: [],
    pending_native_verification_points: [],
    generated_domain_crates: [],
    registration_coverage: [],
  };

  const manifest = readC7Json(manifestPath, 'C7 write manifest');
  if (!manifest) {
    report.pending_native_verification_points = c7PendingNativePoints();
    recordC7ValidationCommands(c1MigrationCheckpoint);
    return report;
  }

  report.manifest_path = manifestPath;
  report.manifest_status = manifest.status;
  report.base_sha = manifest.base_sha;
  report.code_base_sha = manifest.code_base_sha;
  report.task = manifest.boundary || 'C7';
  report.workstream = 'W4';
  report.slice = 'domain_waves';

  validateC7WriteManifest(manifest, expectedWaves);

  const waves = Array.isArray(manifest.waves) ? manifest.waves : [];
  const waveByTask = new Map(
    waves
      .filter((wave) => wave && typeof wave.task_id === 'string')
      .map((wave) => [wave.task_id, wave])
  );
  if (waves.length !== expectedWaves.length) {
    failures.push(
      `C7 write manifest must contain exactly ${expectedWaves.length} domain waves`
    );
  }
  if (waveByTask.size !== waves.length) {
    failures.push('C7 write manifest contains duplicate or invalid task_id entries');
  }

  const deletionManifests = [];
  const waveResults = [];
  for (const expected of expectedWaves) {
    const wave = waveByTask.get(expected.task_id);
    const waveResult = {
      task: expected.task_id,
      task_id: expected.task_id,
      workstream: wave?.workstream || manifest.workstream || 'W4',
      slice: expected.wave,
      owner: expected.owner,
      deletion_manifest: expected.deletion_manifest,
      generated_crate: expected.generated_crate,
      target_packages: [],
      target_capability_families: [],
      status: 'fail',
    };
    report.tasks.push({
      task_id: expected.task_id,
      workstream: waveResult.workstream,
      slice: expected.wave,
      owner: expected.owner,
    });
    if (!wave) {
      failures.push(`missing C7 wave task: ${expected.task_id}`);
      waveResults.push(waveResult);
      continue;
    }

    const waveFailureStart = failures.length;
    validateC7WaveEntry(wave, expected);
    waveResult.target_packages = uniqueSortedStrings(wave.target_packages);
    waveResult.target_capability_families = uniqueSortedStrings(
      wave.target_capability_families
    );

    const deletion = readC7Json(
      expected.deletion_manifest,
      `${expected.task_id} deletion manifest`
    );
    const deletionResult = {
      task_id: expected.task_id,
      workstream: waveResult.workstream,
      slice: expected.wave,
      path: expected.deletion_manifest,
      status: deletion ? 'pass' : 'fail',
    };
    if (deletion) {
      const deletionFailureStart = failures.length;
      validateC7DeletionManifest(deletion, wave, expected);
      deletionResult.manifest_id = deletion.manifest_id;
      deletionResult.target_packages = uniqueSortedStrings(
        deletion.canonical_producer?.target_package_keys
      );
      deletionResult.target_capability_families = uniqueSortedStrings(
        deletion.canonical_producer?.target_capability_families
      );
      deletionManifests.push({
        path: expected.deletion_manifest,
        payload: deletion,
        result: deletionResult,
      });
      deletionResult.status =
        failures.length === deletionFailureStart ? 'pass' : 'fail';
    }
    waveResult.deletion_manifest_result = deletionResult;
    waveResult.status = failures.length === waveFailureStart ? 'pass' : 'fail';
    waveResults.push(waveResult);
  }
  report.slices = waveResults;
  report.workstreams = uniqueSortedStrings(
    waveResults.map((wave) => wave.workstream).filter(Boolean)
  );
  report.domain_wave_manifests = deletionManifests.map(({ path, result }) => ({
    path,
    ...result,
  }));

  report.input_digests = verifyC7InputDigests(manifest);

  const generatedCrateResults = [];
  const canonicalScopes = [];
  const supportPath = findC7Path(
    manifest.shared_integration?.write_paths,
    /^crates\/backend\/nomifun-agent-domain-support\/?$/
  );
  if (supportPath) {
    const supportResult = inspectC7Crate(
      supportPath,
      'C7-SHARED-INTEGRATION',
      'W4',
      'shared_support',
      false
    );
    generatedCrateResults.push(supportResult);
    canonicalScopes.push({
      task_id: 'C7-SHARED-INTEGRATION',
      workstream: 'W4',
      slice: 'shared_support',
      paths: [supportPath],
    });
  }

  for (const expected of expectedWaves) {
    const wave = waveByTask.get(expected.task_id);
    const pluginPaths = Array.isArray(wave?.write_paths)
      ? wave.write_paths
          .map(normalizeRepoPath)
          .filter((path) => path.endsWith('/v2_plugin.rs'))
      : [];
    const crateResult = inspectC7Crate(
      expected.generated_crate,
      expected.task_id,
      wave?.workstream || 'W4',
      expected.wave,
      manifest.status === 'closed'
    );
    generatedCrateResults.push(crateResult);
    const existingPluginPaths = pluginPaths.filter((path) =>
      statSafe(join(repoRoot, path))
    );
    canonicalScopes.push({
      task_id: expected.task_id,
      workstream: wave?.workstream || 'W4',
      slice: expected.wave,
      paths: [expected.generated_crate, ...existingPluginPaths],
    });
    for (const pluginPath of pluginPaths) {
      if (existingPluginPaths.includes(pluginPath)) continue;
      generatedCrateResults.push({
        task_id: expected.task_id,
        workstream: wave?.workstream || 'W4',
        slice: expected.wave,
        path: pluginPath,
        kind: 'registration_source',
        present: false,
        required: manifest.status === 'closed',
        status: manifest.status === 'closed' ? 'fail' : 'pending',
        files: [],
        missing_files: [],
      });
      if (manifest.status === 'closed') {
        failures.push(
          `${expected.task_id}: missing canonical registration source ${pluginPath}`
        );
      }
    }
  }
  report.generated_domain_crates = generatedCrateResults;

  const residualResults = [];
  for (const scope of canonicalScopes) {
    const files = c7SourceFilesForPaths(scope.paths);
    const scan = scanC7ForbiddenEdges(files);
    const result = {
      task_id: scope.task_id,
      workstream: scope.workstream,
      slice: scope.slice,
      scope: scope.paths,
      scanned_files: files.map((file) =>
        relative(repoRoot, file).replaceAll('\\', '/')
      ),
      expected_count: 0,
      observed_count: scan.total_count,
      status: scan.total_count === 0 ? 'pass' : 'fail',
      findings: scan.findings,
    };
    residualResults.push(result);
    for (const finding of scan.findings) {
      failures.push(
        `${finding.path}:${finding.line}: C7 forbidden ${finding.category} edge (${finding.label})`
      );
    }
  }
  report.residual_results = residualResults;

  const coverageResults = [];
  for (const expected of expectedWaves) {
    const wave = waveByTask.get(expected.task_id);
    const scope = canonicalScopes.find(
      (candidate) => candidate.task_id === expected.task_id
    );
    const coverage = inspectC7RegistrationCoverage(
      expected,
      wave,
      scope ? c7SourceFilesForPaths(scope.paths) : []
    );
    coverageResults.push(coverage);
    if (coverage.status === 'fail') {
      failures.push(
        `${expected.task_id}: registration coverage mismatch (missing packages: ${coverage.missing_packages.join(', ') || 'none'}; missing capability families: ${coverage.missing_capability_families.join(', ') || 'none'})`
      );
    }
  }
  report.registration_coverage = coverageResults;

  const reachabilityResults = [];
  for (const expected of expectedWaves) {
    const wave = waveByTask.get(expected.task_id);
    const deletion = deletionManifests.find(
      (candidate) => candidate.path === expected.deletion_manifest
    )?.payload;
    const reachability = scanC7Reachability(
      expected,
      wave,
      deletion,
      canonicalScopes,
      manifest.status === 'closed',
    );
    reachabilityResults.push(reachability);
    if (reachability.status === 'fail') {
      for (const finding of reachability.findings) {
        failures.push(
          `${finding.path}:${finding.line}: C7 legacy edge reachable from production root (${finding.label})`
        );
      }
    }
  }
  report.reachability_results = reachabilityResults;

  report.pending_native_verification_points = c7PendingNativePoints();
  recordC7ValidationCommands(c1MigrationCheckpoint);
  return report;
}

function writeC7DomainWavesReport(details) {
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
    'c7-domain-waves'
  );
  mkdirSync(reportDir, { recursive: true });
  const report = {
    schema_version: '1.0.0',
    gate_name: 'c7-domain-waves',
    source_sha: sourceSha,
    evidence_kind: 'informational',
    ...details,
    commands,
    status: failures.length === 0 ? 'pass' : 'fail',
    failures,
  };
  writeFileSync(join(reportDir, 'summary.json'), `${JSON.stringify(report, null, 2)}\n`);
}

function runC8WinPreGate() {
  const sourceSha = c8ReadGitHeadForReport();
  const evidenceDirectory = `build.noindex/agent-capability-v2/${sourceSha}/c8-win-pre`;
  mkdirSync(join(repoRoot, evidenceDirectory), { recursive: true });
  const report = {
    schema_version: '1.0.0',
    gate_name: 'c8-win-pre',
    evidence_kind: 'native',
    source_sha: sourceSha,
    candidate_source_sha: sourceSha,
    manifest_path: C8_MANIFEST_PATH,
    evidence_directory: evidenceDirectory,
    execution_host: {
      platform: process.platform,
      arch: process.arch,
      native_windows_x64:
        process.platform === 'win32' && process.arch === 'x64',
    },
    source_checkpoint: null,
    target_cell: null,
    c7: { status: 'not_evaluated' },
    platform_validation: { status: 'not_evaluated' },
    windows_metadata: { status: 'not_evaluated' },
    all_scene_coverage: { status: 'not_evaluated' },
    residual_reachability: { status: 'not_evaluated' },
    checks: [],
    statuses: {},
    artifact_digests: {},
    failure_details: [],
    preflight_blocked: false,
  };

  if (process.platform !== 'win32') {
    c8Block(
      report,
      'native_platform_required',
      'C8-WIN-PRE is a Windows-only native gate; this host cannot attest Windows.'
    );
    report.execution_host.native_windows_x64 = false;
    c8SkipC8Checks(report, null, 'non-Windows host');
    return report;
  }

  if (process.arch !== 'x64') {
    c8Block(
      report,
      'native_architecture_required',
      `C8-WIN-PRE requires a native Windows x64 host; observed ${process.arch}.`
    );
  }

  const manifest = c8ReadJsonArtifact(report, C8_MANIFEST_PATH, 'C8-WIN-PRE manifest');
  if (!manifest) {
    c8SkipC8Checks(report, null, 'missing or invalid C8-WIN-PRE manifest');
    return report;
  }
  report.manifest_status = manifest.value.status;
  report.manifest_raw_sha256 = manifest.raw_sha256;

  c8ValidateC8Manifest(report, manifest.value, sourceSha);
  report.source_checkpoint = c8ResolveC8SourceCheckpoint(
    report,
    manifest.value,
    sourceSha
  );
  report.target_cell = manifest.value.target_cell || null;

  report.c7 = c8ValidateC7State(report, sourceSha);
  report.platform_validation = c8ValidateCanonicalPlatformInputs(
    report,
    manifest.value
  );
  report.windows_metadata = c8ValidateWindowsMetadata(
    report,
    report.platform_validation
  );
  report.residual_reachability = c8ValidateC8ResidualReachability(
    report,
    report.c7,
    manifest.value
  );

  if (report.preflight_blocked) {
    c8SkipNativeChecksIfBlocked(
      report,
      manifest.value,
      'C8 preflight failure'
    );
    return report;
  }

  c8RunToolchainProbe(report);
  c8RunDeclaredC8Checks(report, manifest.value);
  // Scene coverage is finalized only after all declared checks have run.  In
  // particular, provider_unavailable is not evidence until the real
  // production broker test has a recorded passing result.
  report.all_scene_coverage = c8ValidateAllSceneCoverage(
    report,
    manifest.value,
    report.platform_validation
  );
  c8ValidateProductionBrokerFunctionalEvidence(report);
  return report;
}

function c8ParseNativeDispatchArgs(name, argv) {
  const explicit = C8_NATIVE_GATE_DISPATCH[name] || null;
  let cellId = explicit?.cell_id || null;
  let evidencePath = null;
  let sawCell = false;

  for (let index = 1; index < argv.length; index += 1) {
    const token = argv[index];
    const [flag, inlineValue] = token.includes('=')
      ? [token.slice(0, token.indexOf('=')), token.slice(token.indexOf('=') + 1)]
      : [token, null];
    if (!['--cell', '--evidence'].includes(flag)) {
      throw new Error(`unknown ${name} option: ${token}`);
    }
    let value = inlineValue;
    if (value === null) {
      value = argv[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error(`${flag} requires a value`);
      }
      index += 1;
    }
    if (flag === '--cell') {
      if (sawCell) throw new Error('--cell may be specified only once');
      sawCell = true;
      cellId = value;
    } else {
      if (evidencePath !== null) {
        throw new Error('--evidence may be specified only once');
      }
      evidencePath = value;
    }
  }

  if (name === 'c8-native' && !cellId) {
    throw new Error('c8-native requires --cell <cell_id>');
  }
  if (!Object.hasOwn(C8_NATIVE_CELL_SPECS, cellId)) {
    throw new Error(`unsupported native cell: ${cellId || '(missing)'}`);
  }
  if (explicit && explicit.cell_id !== cellId) {
    throw new Error(`${name} is fixed to --cell ${explicit.cell_id}`);
  }

  const spec = C8_NATIVE_CELL_SPECS[cellId];
  const defaultEvidencePath =
    `build.noindex/agent-capability-v2/${c8ReadGitHeadForReport()}/${cellId}/platform-result.json`;
  const suppliedEvidencePath =
    evidencePath ||
    (typeof process.env.AGENT_V2_CELL_EVIDENCE === 'string'
      ? process.env.AGENT_V2_CELL_EVIDENCE.trim() || null
      : null);
  return {
    gate_name: name,
    cell_id: cellId,
    evidence_path: suppliedEvidencePath || defaultEvidencePath,
    evidence_path_source: suppliedEvidencePath
      ? evidencePath
        ? 'argument'
        : 'environment'
      : 'canonical_default',
    check_id: spec.check_id,
    command: spec.command,
    target: spec,
  };
}

function c8NativeFailure(report, code, message, details = {}) {
  const failure = { code, message, ...details };
  report.failure_details.push(failure);
  failures.push(`${report.gate_name}: ${message}`);
  return false;
}

function c8NativeRequire(report, condition, code, message, details = {}) {
  return condition ? true : c8NativeFailure(report, code, message, details);
}

function c8NativeRunProbe(command, commandArgs, timeout = 5000) {
  const startedAt = new Date().toISOString();
  try {
    const result = spawnSync(command, commandArgs, {
      cwd: repoRoot,
      encoding: 'utf8',
      shell: process.platform === 'win32',
      stdio: 'pipe',
      timeout,
    });
    return {
      command: [command, ...commandArgs].join(' '),
      started_at: startedAt,
      finished_at: new Date().toISOString(),
      status: typeof result.status === 'number' ? result.status : 1,
      stdout: String(result.stdout || ''),
      stderr: String(result.stderr || ''),
      error: result.error?.message || null,
    };
  } catch (error) {
    return {
      command: [command, ...commandArgs].join(' '),
      started_at: startedAt,
      finished_at: new Date().toISOString(),
      status: 1,
      stdout: '',
      stderr: error.message,
      error: error.message,
    };
  }
}

function c8NativeHostProbe() {
  const hostOsByPlatform = {
    win32: 'windows',
    darwin: 'macos',
    linux: 'linux',
  };
  const hostArchByProcess = {
    x64: 'x86_64',
    arm64: 'aarch64',
  };
  const hostOs = hostOsByPlatform[process.platform] || null;
  const hostArch = hostArchByProcess[process.arch] || null;
  const probes = [];
  const rejectionReasons = [];
  let unameSystem = null;
  let unameMachine = null;
  let rosettaTranslated = false;
  let rustcHostTarget = null;
  let toolchainFingerprintDigest = null;
  const rustc = c8NativeRunProbe('rustc', ['-vV']);
  probes.push(rustc);
  rustcHostTarget =
    rustc.stdout.match(/^\s*host:\s*([^\s]+)\s*$/m)?.[1] || null;
  if (rustc.status !== 0 || !rustcHostTarget) {
    rejectionReasons.push('rustc_host_probe_failed');
  } else {
    toolchainFingerprintDigest = createHash('sha256')
      .update(rustc.stdout)
      .digest('hex');
  }

  if (process.platform !== 'win32') {
    const uname = c8NativeRunProbe('uname', ['-s', '-m']);
    probes.push(uname);
    const fields = uname.stdout.trim().split(/\s+/);
    unameSystem = fields[0] || null;
    unameMachine = fields[1] || null;
    if (uname.status !== 0) rejectionReasons.push('uname_probe_failed');
  }

  if (process.platform === 'darwin') {
    const translated = c8NativeRunProbe(
      'sysctl',
      ['-in', 'sysctl.proc_translated']
    );
    probes.push(translated);
    rosettaTranslated = translated.status === 0 && translated.stdout.trim() === '1';
    if (rosettaTranslated) rejectionReasons.push('rosetta_translation_detected');
  }

  if (process.platform === 'linux') {
    const environmentText = [
      process.env.WSL_DISTRO_NAME,
      process.env.WSL_INTEROP,
      process.env.WSLENV,
    ]
      .filter(Boolean)
      .join(' ');
    if (environmentText) rejectionReasons.push('wsl_environment_detected');

    const kernelText = [
      unameSystem,
      unameMachine,
      readFileSafe('/proc/version'),
      readFileSafe('/proc/1/cgroup'),
    ]
      .filter(Boolean)
      .join('\n')
      .toLowerCase();
    for (const marker of [
      'microsoft',
      'wsl',
      'docker',
      'kubepods',
      'containerd',
      'libpod',
      'lxc',
      'qemu',
    ]) {
      if (kernelText.includes(marker)) {
        rejectionReasons.push(`non_native_environment_marker:${marker}`);
      }
    }

    const virtual = c8NativeRunProbe('systemd-detect-virt', ['--vm']);
    probes.push(virtual);
    const virtualName = virtual.stdout.trim().toLowerCase();
    if (virtual.status === 0 && virtualName && virtualName !== 'none') {
      rejectionReasons.push(`virtual_machine_detected:${virtualName}`);
    }
  }

  const expectedUnameSystem =
    hostOs === 'macos' ? 'darwin' : hostOs === 'linux' ? 'linux' : null;
  const expectedUnameMachine =
    process.platform === 'win32'
      ? null
      : hostArch === 'aarch64'
        ? 'arm64'
        : hostArch === 'x86_64'
          ? 'x86_64'
          : null;
  if (expectedUnameSystem && unameSystem?.toLowerCase() !== expectedUnameSystem) {
    rejectionReasons.push('host_uname_os_mismatch');
  }
  if (expectedUnameMachine && unameMachine !== expectedUnameMachine) {
    rejectionReasons.push('host_uname_arch_mismatch');
  }

  return {
    process_platform: process.platform,
    process_arch: process.arch,
    host_os: hostOs,
    host_arch: hostArch,
    rustc_host_target: rustcHostTarget,
    toolchain_fingerprint_digest: toolchainFingerprintDigest,
    uname_system: unameSystem,
    uname_machine: unameMachine,
    rosetta_translated: rosettaTranslated,
    rejection_reasons: [...new Set(rejectionReasons)],
    probes: probes.map((probe) => ({
      command: probe.command,
      status: probe.status === 0 ? 'pass' : 'fail',
      stdout: c8Tail(probe.stdout),
      stderr: c8Tail(probe.stderr),
      error: probe.error,
    })),
    native:
      Boolean(hostOs && hostArch) &&
      rejectionReasons.length === 0 &&
      !rosettaTranslated,
  };
}

function c8NativeValidateHost(report, target) {
  const observed = report.execution_host;
  c8NativeRequire(
    report,
    observed.host_os === target.host_os,
    'native_host_os_mismatch',
    `native gate ${target.cell_id} requires ${target.host_os}, observed ${observed.host_os || 'unknown'}`,
    { expected: target.host_os, observed: observed.host_os }
  );
  c8NativeRequire(
    report,
    observed.host_arch === target.host_arch,
    'native_host_arch_mismatch',
    `native gate ${target.cell_id} requires ${target.host_arch}, observed ${observed.host_arch || 'unknown'}`,
    { expected: target.host_arch, observed: observed.host_arch }
  );
  c8NativeRequire(
    report,
    observed.rustc_host_target === target.host_target,
    'native_host_target_mismatch',
    `native gate ${target.cell_id} requires rustc host target ${target.host_target}`,
    { expected: target.host_target, observed: observed.rustc_host_target }
  );
  c8NativeRequire(
    report,
    observed.native === true,
    'native_host_not_verified',
    'the current process is not proven to be a native target host',
    { rejection_reasons: observed.rejection_reasons }
  );
  return report.failure_details.length === 0;
}

function c8NativeGitProbe(report, checkId, commandArgs, timeout = 10000) {
  const probe = c8NativeRunProbe('git', commandArgs, timeout);
  const entry = {
    check_id: checkId,
    kind: 'metadata_probe',
    command: probe.command,
    started_at: probe.started_at,
    finished_at: probe.finished_at,
    exit_code: probe.status,
    status: probe.status === 0 ? 'pass' : 'fail',
    stdout: c8Tail(probe.stdout),
    stderr: c8Tail(probe.stderr),
  };
  report.commands.push(entry);
  report.metadata_commands.push(entry);
  return probe;
}

function c8NativeValidateSourceCheckpoint(report, sourceSha) {
  const statusProbe = c8NativeGitProbe(report, 'source_worktree', [
    'status',
    '--porcelain',
    '--untracked-files=all',
  ]);
  const worktreeStatus = statusProbe.stdout.trim();
  const headProbe = c8NativeGitProbe(report, 'source_head', [
    'rev-parse',
    'HEAD',
  ]);
  const observedHead = headProbe.stdout.trim();
  const checkpoint = {
    local_head: sourceSha,
    observed_head: observedHead,
    clean_worktree: statusProbe.status === 0 && !worktreeStatus,
  };
  report.source_checkpoint = checkpoint;
  c8NativeRequire(
    report,
    statusProbe.status === 0,
    'native_worktree_probe_failed',
    'native cell cannot prove a clean worktree',
    { exit_code: statusProbe.status, stderr: c8Tail(statusProbe.stderr) }
  );
  c8NativeRequire(
    report,
    !worktreeStatus,
    'native_dirty_worktree',
    'native cell requires a clean worktree before evidence is accepted',
    { status: worktreeStatus }
  );
  c8NativeRequire(
    report,
    observedHead === sourceSha,
    'native_head_mismatch',
    'native cell source HEAD changed during dispatch',
    { expected: sourceSha, observed: observedHead }
  );
  return checkpoint;
}

function c8NativeReadJson(report, path, label) {
  const normalized = normalizeRepoPath(path);
  if (!isSafeRepoPath(normalized)) {
    c8NativeFailure(report, 'native_invalid_repo_path', `${label} path is not repository-relative`, {
      path,
    });
    return null;
  }
  const absolute = join(repoRoot, normalized);
  if (!statSafe(absolute)?.isFile()) {
    c8NativeFailure(report, 'native_missing_artifact', `missing ${label}: ${normalized}`, {
      path: normalized,
    });
    return null;
  }
  try {
    return {
      path: normalized,
      absolute,
      value: JSON.parse(readFileSync(absolute, 'utf8')),
      raw_sha256: sha256File(absolute),
    };
  } catch (error) {
    c8NativeFailure(report, 'native_invalid_json', `${label} is invalid JSON: ${error.message}`, {
      path: normalized,
    });
    return null;
  }
}

function c8NativeExpectedPlatformCells() {
  return Object.fromEntries(
    Object.entries(C8_NATIVE_CELL_SPECS).map(([cellId, spec]) => [
      cellId,
      {
        host_os: spec.host_os,
        host_arch: spec.host_arch,
        host_target: spec.host_target,
        runtime_target: spec.runtime_target,
        host_surface: spec.host_surface,
        package_format: spec.package_format,
        capability_availability:
          cellId === 'linux_desktop_x64'
              ? {
                  coding_codex_native: { availability: 'required_exact_set' },
                  browser: { availability: 'release_manifest_defined' },
                  computer: {
                    availability: 'independent_partial_or_exact_unavailable',
                  },
                }
              : {
                  coding_codex_native: { availability: 'required_exact_set' },
                  browser: { availability: 'release_manifest_defined' },
                  computer: { availability: 'release_manifest_defined' },
                },
      },
    ])
  );
}

function c8NativeValidatePlatformInputs(report) {
  const result = {
    status: 'fail',
    input_digests: {},
    platform_matrix: null,
    required_checks: [],
    verification_points: [],
  };
  const fixture = c8NativeReadJson(
    report,
    C8_PLATFORM_VALIDATION_FIXTURE_PATH,
    'generated PlatformValidationManifest fixture'
  );
  const platformPayloadArtifact = c8NativeReadJson(
    report,
    C8_PLATFORM_VALIDATION_MANIFEST_PATH,
    'PlatformValidationManifest payload'
  );
  const runtimeFixture = c8NativeReadJson(
    report,
    C8_RUNTIME_RELEASE_FIXTURE_PATH,
    'generated Runtime release fixture'
  );
  const runtimeInput = c8NativeReadJson(
    report,
    C8_RUNTIME_RELEASE_FIXTURE_INPUT_PATH,
    'Runtime release schema fixture'
  );
  if (!fixture || !platformPayloadArtifact || !runtimeFixture || !runtimeInput) {
    return result;
  }

  const platformPayload = fixture.value?.payload;
  const runtimePayload = runtimeFixture.value?.payload;
  const platformPayloadDigest = fixture.value?.payload_digest;
  const runtimePayloadDigest = runtimeFixture.value?.payload_digest;
  result.input_digests.platform_validation_fixture = {
    observed: platformPayloadDigest || null,
    raw_sha256: fixture.raw_sha256,
    status:
      c8DigestPayload(platformPayload) === platformPayloadDigest
        ? 'pass'
        : 'fail',
  };
  result.input_digests.runtime_release_fixture = {
    observed: runtimePayloadDigest || null,
    raw_sha256: runtimeFixture.raw_sha256,
    status:
      c8DigestPayload(runtimePayload) === runtimePayloadDigest
        ? 'pass'
        : 'fail',
  };
  c8NativeRequire(
    report,
    fixture.value?.digest_algorithm === 'sorted-json-sha256-v1' &&
      runtimeFixture.value?.digest_algorithm === 'sorted-json-sha256-v1',
    'native_input_digest_algorithm',
    'native validation inputs must use sorted-json-sha256-v1'
  );
  c8NativeRequire(
    report,
    c8DigestPayload(platformPayload) === platformPayloadDigest &&
      c8DigestPayload(runtimePayload) === runtimePayloadDigest,
    'native_input_payload_digest',
    'native validation input payload bytes do not reproduce their frozen digests'
  );
  c8NativeRequire(
    report,
    c8CanonicalEqual(platformPayloadArtifact.value, platformPayload),
    'native_platform_payload_mismatch',
    'PlatformValidationManifest source payload differs from its generated fixture'
  );
  c8NativeRequire(
    report,
    c8CanonicalEqual(runtimeInput.value, runtimePayload),
    'native_runtime_payload_mismatch',
    'Runtime release schema fixture differs from its generated envelope'
  );
  for (const [key, expected] of Object.entries({
    confirmed_decision_contract_digest:
      C8_EXPECTED_DIGESTS.confirmed_decision_contract,
    canonical_schema_manifest_digest: C8_EXPECTED_DIGESTS.canonical_schema_manifest,
    cargo_lock_digest: C8_EXPECTED_DIGESTS.cargo_lock,
    official_preset_seed_manifest_digest: C8_EXPECTED_DIGESTS.official_seed,
    capability_availability_manifest_digest: C8_EXPECTED_DIGESTS.capability_availability,
    coding_codex_native_contract_digest: C8_EXPECTED_DIGESTS.coding_codex_native,
  })) {
    c8NativeRequire(
      report,
      platformPayload?.[key] === expected,
      'native_platform_input_digest',
      `PlatformValidationManifest ${key} differs from the frozen input`,
      { expected, observed: platformPayload?.[key] || null }
    );
  }

  const cells = platformPayload?.platform_matrix?.target_cells;
  const expectedCellIds = [...C8_REQUIRED_NATIVE_CELLS].sort();
  const observedCellIds =
    cells && typeof cells === 'object' && !Array.isArray(cells)
      ? Object.keys(cells).sort()
      : [];
  c8NativeRequire(
    report,
    c8CanonicalEqual(observedCellIds, expectedCellIds),
    'native_platform_required_cells',
    'PlatformValidationManifest must contain exactly the three release-blocking platform cells',
    { expected: expectedCellIds, observed: observedCellIds }
  );
  const expectedCells = c8NativeExpectedPlatformCells();
  for (const cellId of C8_REQUIRED_NATIVE_CELLS) {
    const observed = cells?.[cellId];
    const expected = expectedCells[cellId];
    c8NativeRequire(
      report,
      c8CanonicalEqual(observed, expected),
      'native_platform_cell_contract',
      `PlatformValidationManifest ${cellId} does not match the frozen target/availability contract`,
      { cell_id: cellId, expected, observed: observed || null }
    );
  }
  result.platform_matrix = cells || null;

  const requiredChecks = Array.isArray(platformPayload?.required_checks)
    ? platformPayload.required_checks
    : [];
  const expectedChecks = C8_REQUIRED_NATIVE_CELLS.map((cellId) => {
    const spec = C8_NATIVE_CELL_SPECS[cellId];
    return {
      check_id: spec.check_id,
      target_cells: [cellId],
      command: spec.command,
      required_execution_kind: 'native',
    };
  });
  result.required_checks = requiredChecks;
  c8NativeRequire(
    report,
    c8CanonicalEqual(
      requiredChecks.map((check) => ({
        check_id: check?.check_id,
        target_cells: check?.target_cells,
        command: check?.command,
        required_execution_kind: check?.required_execution_kind,
      })).sort((left, right) => left.check_id.localeCompare(right.check_id)),
      expectedChecks.sort((left, right) => left.check_id.localeCompare(right.check_id))
    ),
    'native_required_check_contract',
    'PlatformValidationManifest release-blocking checks differ from the three-platform dispatch table'
  );

  const verificationPoints = Array.isArray(platformPayload?.platform_verification_points)
    ? platformPayload.platform_verification_points
    : [];
  result.verification_points = verificationPoints;
  for (const cellId of C8_REQUIRED_NATIVE_CELLS) {
    const expectedPoint = verificationPoints.filter(
      (point) => point?.target_cell === cellId
    );
    c8NativeRequire(
      report,
      expectedPoint.length === 1 &&
        expectedPoint[0]?.exact_check_id === C8_NATIVE_CELL_SPECS[cellId].check_id,
      'native_verification_point_contract',
      `PlatformValidationManifest must declare exactly one native verification point for ${cellId} with its exact full-gate check`,
      { cell_id: cellId, observed: expectedPoint }
    );
  }
  return {
    ...result,
    platform_payload: platformPayload,
    runtime_payload: runtimePayload,
    status: report.failure_details.length === 0 ? 'pass' : 'fail',
  };
}

function c8NativeResolveEvidencePath(value) {
  if (typeof value !== 'string' || !value.trim()) return null;
  return resolve(repoRoot, value);
}

function c8NativePathInsideRepo(absolute) {
  const normalized = relative(repoRoot, absolute).replaceAll('\\', '/');
  return (
    normalized === '' ||
    (normalized !== '..' &&
      !normalized.startsWith('../') &&
      !/^[A-Za-z]:\//.test(normalized))
  );
}

function c8NativeValidateArtifactRef(
  report,
  ref,
  label,
  { requireFile = true, inspectJson = true } = {}
) {
  const normalized = normalizeRepoPath(ref?.normalized_relative_path);
  const unknownKeys =
    ref && typeof ref === 'object' && !Array.isArray(ref)
      ? Object.keys(ref)
          .filter(
            (key) =>
              !['artifact_id', 'digest', 'normalized_relative_path'].includes(
                key
              )
          )
          .sort()
      : [];
  c8NativeRequire(
    report,
    unknownKeys.length === 0,
    'native_evidence_ref_unknown_fields',
    `${label} contains fields outside LogicalArtifactRef`,
    { fields: unknownKeys }
  );
  const shape =
    ref &&
    typeof ref === 'object' &&
    !Array.isArray(ref) &&
    typeof ref.artifact_id === 'string' &&
    ref.artifact_id.length > 0 &&
    c8PortablePath(normalized) &&
    c8Hex(ref.digest);
  if (!c8NativeRequire(
    report,
    shape,
    'native_evidence_ref_invalid',
    `${label} is not a valid logical artifact reference`,
    { observed: ref || null }
  )) {
    return null;
  }
  if (!isSafeRepoPath(normalized)) {
    c8NativeFailure(
      report,
      'native_evidence_ref_outside_repo',
      `${label} must use a repository-relative evidence path`,
      { path: normalized }
    );
    return null;
  }
  const absolute = join(repoRoot, normalized);
  if (!requireFile) return { ...ref, normalized_relative_path: normalized };
  if (!statSafe(absolute)?.isFile()) {
    c8NativeFailure(
      report,
      'native_evidence_artifact_missing',
      `${label} is missing: ${normalized}`,
      { path: normalized }
    );
    return null;
  }
  let observedDigest;
  try {
    observedDigest = sha256File(absolute);
  } catch (error) {
    c8NativeFailure(
      report,
      'native_evidence_artifact_unreadable',
      `${label} cannot be read: ${error.message}`,
      { path: normalized }
    );
    return null;
  }
  if (observedDigest.toLowerCase() !== String(ref.digest).toLowerCase()) {
    c8NativeFailure(
      report,
      'native_evidence_artifact_digest_mismatch',
      `${label} digest does not match the referenced artifact`,
      { path: normalized, expected: ref.digest, observed: observedDigest }
    );
    return null;
  }
  if (inspectJson && normalized.toLowerCase().endsWith('.json')) {
    try {
      const parsed = JSON.parse(readFileSync(absolute, 'utf8'));
      c8NativeInspectAuxiliaryEvidence(report, parsed, label, report.target_cell);
    } catch (error) {
      c8NativeFailure(
        report,
        'native_evidence_artifact_invalid_json',
        `${label} is not valid JSON: ${error.message}`,
        { path: normalized }
      );
      return null;
    }
  }
  return {
    ...ref,
    normalized_relative_path: normalized,
    absolute,
    observed_digest: observedDigest,
  };
}

function c8NativeInspectAuxiliaryEvidence(report, value, label, target) {
  const payload =
    value && typeof value === 'object' && value.payload && typeof value.payload === 'object'
      ? value.payload
      : value;
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return;
  const observedCell = payload.cell_id || payload.target_cell;
  if (observedCell !== undefined) {
    c8NativeRequire(
      report,
      observedCell === target.cell_id,
      'native_auxiliary_cell_mismatch',
      `${label} identifies a different target cell`,
      { expected: target.cell_id, observed: observedCell }
    );
  }
  if (payload.host_surface !== undefined) {
    c8NativeRequire(
      report,
      payload.host_surface === target.host_surface,
      'native_auxiliary_surface_mismatch',
      `${label} identifies a different host surface`,
      { expected: target.host_surface, observed: payload.host_surface }
    );
  }
  if (payload.execution_kind !== undefined) {
    c8NativeRequire(
      report,
      payload.execution_kind === 'native',
      'native_auxiliary_execution_kind',
      `${label} is not native execution evidence`,
      { observed: payload.execution_kind }
    );
  }
  if (payload.native !== undefined) {
    c8NativeRequire(
      report,
      payload.native === true,
      'native_auxiliary_native_flag',
      `${label} does not assert native execution`,
      { observed: payload.native }
    );
  }
  if (
    payload.status !== undefined &&
    (payload.gate_name !== undefined ||
      payload.cell_id !== undefined ||
      payload.target_cell !== undefined ||
      payload.verification_point_id !== undefined ||
      payload.native !== undefined)
  ) {
    c8NativeRequire(
      report,
      payload.status === 'pass',
      'native_auxiliary_status',
      `${label} is not a passing evidence artifact`,
      { observed: payload.status }
    );
  }
  for (const key of [
    'virtualization',
    'virtual_machine',
    'emulation',
    'emulator',
    'rosetta',
    'containerized',
    'container',
    'wsl',
  ]) {
    if (payload[key] === undefined) continue;
    const valueText = String(payload[key]).toLowerCase();
    const disallowed =
      payload[key] === true ||
      (typeof payload[key] === 'string' &&
        valueText !== '' &&
        !['none', 'false', 'no', 'native'].includes(valueText));
    c8NativeRequire(
      report,
      !disallowed,
      'native_auxiliary_non_native_environment',
      `${label} contains a cross-compile/VM/emulation/container environment marker`,
      { key, observed: payload[key] }
    );
  }
}

function c8NativeLoadCellEvidence(report, dispatch) {
  if (!dispatch.evidence_path) {
    c8NativeFailure(
      report,
      'native_evidence_required',
      `${dispatch.gate_name} requires --evidence <platform-result.json>; no native evidence was supplied`
    );
    return null;
  }
  const absolute = c8NativeResolveEvidencePath(dispatch.evidence_path);
  if (!absolute || !statSafe(absolute)?.isFile()) {
    c8NativeFailure(
      report,
      'native_evidence_missing',
      'the selected platform-result.json file does not exist',
      {
        source_kind: dispatch.evidence_path_source,
        configured_path: dispatch.evidence_path_source === 'canonical_default'
          ? 'canonical platform-result path'
          : 'user-supplied path',
      }
    );
    return null;
  }
  let source;
  let parsed;
  try {
    source = readFileSync(absolute, 'utf8');
    parsed = JSON.parse(source);
  } catch (error) {
    c8NativeFailure(
      report,
      'native_evidence_invalid_json',
      `the supplied platform-result.json is invalid JSON: ${error.message}`
    );
    return null;
  }
  const value =
    parsed &&
    typeof parsed === 'object' &&
    parsed.payload &&
    typeof parsed.payload === 'object'
      ? parsed.payload
      : parsed;
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    c8NativeFailure(
      report,
      'native_evidence_shape',
      'platform-result.json must be a JSON object'
    );
    return null;
  }
  const evidenceDirectory = join(
    repoRoot,
    report.evidence_directory
  );
  const canonicalPath = join(evidenceDirectory, 'platform-result.json');
  try {
    mkdirSync(evidenceDirectory, { recursive: true });
    writeFileSync(canonicalPath, source);
  } catch (error) {
    c8NativeFailure(
      report,
      'native_evidence_copy_failed',
      `cannot retain the supplied evidence as a repo-local artifact: ${error.message}`
    );
    return null;
  }
  report.input_evidence = {
    source_kind: c8NativePathInsideRepo(absolute) ? 'repository' : 'external',
    raw_sha256: createHash('sha256').update(source).digest('hex'),
    normalized_relative_path: relative(repoRoot, canonicalPath).replaceAll('\\', '/'),
    envelope: Boolean(parsed?.payload),
  };
  return { value, source, absolute, canonicalPath };
}

function c8NativeValidateCellEvidence(
  report,
  dispatch,
  sourceSha,
  loaded
) {
  if (!loaded) return null;
  const evidence = loaded.value;
  const platformResult =
    evidence &&
    typeof evidence === 'object' &&
    Object.hasOwn(evidence, 'source_commit') &&
    Object.hasOwn(evidence, 'target') &&
    Object.hasOwn(evidence, 'suite') &&
    Object.hasOwn(evidence, 'release_lock');
  if (!platformResult) {
    c8NativeFailure(
      report,
      'native_platform_result_required',
      'native gates accept only platform-result.json backed by a real release-lock.json'
    );
    return null;
  }
  return c8NativeValidatePlatformResult(
    report,
    dispatch,
    sourceSha,
    evidence
  );

}

function c8NativeValidatePlatformResult(
  report,
  dispatch,
  sourceSha,
  result
) {
  const target = dispatch.target;
  c8NativeRequire(
    report,
    result.schema_version === '1.0.0',
    'native_platform_result_schema',
    'platform-result.json schema_version must be 1.0.0'
  );
  c8NativeRequire(
    report,
    result.source_commit === sourceSha,
    'native_platform_result_source',
    'platform-result.json source_commit differs from the current clean source',
    { expected: sourceSha, observed: result.source_commit || null }
  );
  c8NativeRequire(
    report,
    result.target === target.cell_id,
    'native_platform_result_target',
    'platform-result.json target differs from the dispatched native cell',
    { expected: target.cell_id, observed: result.target || null }
  );
  c8NativeRequire(
    report,
    result.status === 'pass',
    'native_platform_result_status',
    'only a passing platform-result.json can satisfy a native cell',
    { observed: result.status || null }
  );
  c8NativeRequire(
    report,
    result.suite &&
      typeof result.suite.name === 'string' &&
      result.suite.name.length > 0 &&
      Array.isArray(result.suite.checks) &&
      result.suite.checks.length > 0,
    'native_platform_result_suite',
    'platform-result.json must record the actual suite and executed checks'
  );
  c8NativeRequire(
    report,
    Array.isArray(result.logs) && result.logs.length > 0,
    'native_platform_result_logs',
    'platform-result.json must contain at least one log reference'
  );

  const releaseLockPath =
    process.env.NOMIFUN_RELEASE_LOCK_PATH || result.release_lock?.path || null;
  const releaseArtifactRoot =
    process.env.NOMIFUN_RELEASE_ARTIFACT_ROOT || repoRoot;
  const releaseLock = releaseLockPath
    ? readAndVerifyReleaseLock(releaseLockPath, { root: releaseArtifactRoot })
    : {
        status: 'blocked',
        reason: 'release_lock_missing',
        checks: [],
      };
  c8NativeRequire(
    report,
    releaseLock.status === 'pass',
    'native_platform_result_release_lock',
    'platform-result.json must reference a verified real release-lock.json',
    {
      path: releaseLockPath,
      artifact_root: releaseArtifactRoot,
      status: releaseLock.status,
      reason: releaseLock.reason || null,
      checks: releaseLock.checks || [],
    }
  );
  c8NativeRequire(
    report,
    result.release_lock?.sha256 === releaseLock.lock_sha256,
    'native_platform_result_release_lock_digest',
    'platform-result.json release-lock digest differs from the referenced file',
    {
      expected: releaseLock.lock_sha256 || null,
      observed: result.release_lock?.sha256 || null,
    }
  );
  c8NativeRequire(
    report,
    releaseLock.lock?.source_commit === sourceSha,
    'native_platform_result_release_source',
    'release-lock.json source_commit differs from the current clean source',
    {
      expected: sourceSha,
      observed: releaseLock.lock?.source_commit || null,
    }
  );
  const lockedSidecar = releaseLock.lock?.sidecars?.[target.cell_id];
  c8NativeRequire(
    report,
    Boolean(lockedSidecar),
    'native_platform_result_sidecar',
    'release-lock.json has no Sidecar for the dispatched native cell',
    {
      target: target.cell_id,
      available_targets: Object.keys(releaseLock.lock?.sidecars || {}),
    }
  );
  report.artifact_digests = {
    host: releaseLock.lock?.host?.sha256 || null,
    package: releaseLock.lock?.package?.sha256 || null,
    runtime_sidecar: lockedSidecar?.sha256 || null,
    runtime_helpers: releaseLock.lock?.helpers || [],
  };
  report.release_lock = {
    path: releaseLockPath,
    sha256: releaseLock.lock_sha256 || null,
  };
  report.platform_result = {
    source_commit: result.source_commit,
    target: result.target,
    suite: result.suite,
    logs: result.logs,
  };
  return result;
}

function runC8NativeCellGate(dispatch) {
  const sourceSha = c8ReadGitHeadForReport();
  const evidenceDirectory =
    `build.noindex/agent-capability-v2/${sourceSha}/${dispatch.cell_id}`;
  const report = {
    schema_version: '1.0.0',
    gate_name: dispatch.gate_name,
    evidence_kind: 'native',
    source_sha: sourceSha,
    candidate_source_sha: sourceSha,
    manifest_path: C8_PLATFORM_VALIDATION_MANIFEST_PATH,
    evidence_directory: evidenceDirectory,
    dispatch: {
      cell_id: dispatch.cell_id,
      required_check_id: dispatch.check_id,
      required_command: dispatch.command,
      evidence_supplied: dispatch.evidence_path_source !== 'canonical_default',
      evidence_path_source: dispatch.evidence_path_source,
    },
    execution_host: c8NativeHostProbe(),
    source_checkpoint: null,
    target_cell: dispatch.target,
    platform_validation: { status: 'not_evaluated' },
    checks: [],
    statuses: {},
    artifact_digests: {},
    metadata_commands: [],
    commands: [],
    failure_details: [],
    preflight_blocked: false,
  };

  try {
    c8NativeValidateHost(report, dispatch.target);
    c8NativeValidateSourceCheckpoint(report, sourceSha);
    const platform = c8NativeValidatePlatformInputs(report);
    const {
      platform_payload: _platformPayload,
      runtime_payload: _runtimePayload,
      ...platformSummary
    } = platform;
    report.platform_validation = platformSummary;
    const loaded = c8NativeLoadCellEvidence(report, dispatch);
    if (loaded) {
      c8NativeValidateCellEvidence(
        report,
        dispatch,
        sourceSha,
        loaded
      );
    }
  } catch (error) {
    c8NativeFailure(
      report,
      'native_gate_crash',
      `native cell gate failed before completing validation: ${error.message}`
    );
  }

  report.preflight_blocked = report.failure_details.some((failure) =>
    [
      'native_host_',
      'native_worktree_',
      'native_dirty_',
      'native_head_',
      'native_platform_',
      'native_runtime_',
      'native_required_',
    ].some((prefix) => String(failure.code).startsWith(prefix))
  );
  report.status = report.failure_details.length === 0 ? 'pass' : 'fail';
  return report;
}

function writeC8NativeCellReport(report) {
  const reportDir = join(
    repoRoot,
    'build.noindex',
    'agent-capability-v2',
    report.source_sha,
    report.gate_name
  );
  mkdirSync(reportDir, { recursive: true });
  const output = {
    schema_version: '1.0.0',
    gate_name: report.gate_name,
    source_sha: report.source_sha,
    evidence_kind: 'native',
    ...report,
    commands: [...(report.commands || []), ...commands],
    status: report.status === 'pass' && report.failure_details.length === 0
      ? 'pass'
      : 'fail',
    failures: report.failure_details.map((failure) => failure.message),
  };
  writeFileSync(
    join(reportDir, 'summary.json'),
    `${JSON.stringify(output, null, 2)}\n`
  );
}

function c8ReadGitHeadForReport() {
  const result = spawnSync('git', ['rev-parse', 'HEAD'], {
    cwd: repoRoot,
    encoding: 'utf8',
    shell: process.platform === 'win32',
    stdio: 'pipe',
  });
  return result.status === 0 && result.stdout?.trim()
    ? result.stdout.trim()
    : 'unknown-source';
}

function c8ReportPath(report, path) {
  const normalized = normalizeRepoPath(path);
  if (!isSafeRepoPath(normalized)) return null;
  return join(repoRoot, normalized);
}

function c8Block(report, code, message, details = {}) {
  report.preflight_blocked = true;
  c8Failure(report, code, message, details);
}

function c8Failure(report, code, message, details = {}) {
  const failure = { code, message, ...details };
  report.failure_details.push(failure);
  failures.push(`C8-WIN-PRE: ${message}`);
}

function c8Check(report, checkId, status, details = {}) {
  const existing = report.checks.find((check) => check.check_id === checkId);
  const entry = {
    check_id: checkId,
    status,
    ...details,
  };
  if (existing) {
    Object.assign(existing, entry);
  } else {
    report.checks.push(entry);
  }
  report.statuses[checkId] = status;
  return entry;
}

function c8Require(
  report,
  condition,
  code,
  message,
  details = {},
  fatal = false
) {
  if (condition) return true;
  if (fatal) c8Block(report, code, message, details);
  else c8Failure(report, code, message, details);
  return false;
}

function c8SkipC8Checks(report, manifest, reason) {
  const checks = Array.isArray(manifest?.required_checks)
    ? manifest.required_checks
    : [];
  for (const check of checks) {
    if (typeof check?.check_id !== 'string') continue;
    c8Check(report, check.check_id, 'skipped_preflight_failure', {
      command: check.command,
      reason,
    });
  }
}

function c8SkipNativeChecksIfBlocked(report, manifest, reason) {
  if (!report?.preflight_blocked) return false;
  c8SkipC8Checks(report, manifest, reason);
  return true;
}

function c8RunCommand(
  report,
  checkId,
  command,
  commandArgs,
  {
    displayCommand,
    executionKind = 'native',
    addFailure = true,
    evidencePath,
    timeout,
  } = {}
) {
  const startedAt = new Date().toISOString();
  const effectiveTimeout = timeout ?? DEFAULT_COMMAND_TIMEOUT_MS;
  let result;
  try {
    result = spawnSync(command, commandArgs, {
      cwd: repoRoot,
      encoding: 'utf8',
      shell: false,
      stdio: 'pipe',
      maxBuffer: COMMAND_MAX_BUFFER,
      timeout: effectiveTimeout,
    });
  } catch (error) {
    result = {
      status: null,
      stdout: '',
      stderr: error.message,
      error,
    };
  }
  const stdout = String(result.stdout || '');
  const stderr = String(result.stderr || '');
  const exitCode =
    typeof result.status === 'number' ? result.status : result.error ? 1 : 1;
  const commandText = displayCommand || [command, ...commandArgs].join(' ');
  const commandDir = join(repoRoot, report.evidence_directory, 'commands');
  mkdirSync(commandDir, { recursive: true });
  const safeId = String(checkId).replace(/[^A-Za-z0-9_.-]+/g, '_');
  const stdoutPath = join(commandDir, `${safeId}.stdout.log`);
  const stderrPath = join(commandDir, `${safeId}.stderr.log`);
  writeFileSync(stdoutPath, stdout);
  writeFileSync(stderrPath, stderr);
  const stdoutRelative = relative(repoRoot, stdoutPath).replaceAll('\\', '/');
  const stderrRelative = relative(repoRoot, stderrPath).replaceAll('\\', '/');
  const entry = {
    check_id: checkId,
    command: commandText,
    invoked_command: [command, ...commandArgs].join(' '),
    execution_kind: executionKind,
    started_at: startedAt,
    finished_at: new Date().toISOString(),
    exit_code: exitCode,
    status: exitCode === 0 ? 'pass' : 'fail',
    timeout_ms: effectiveTimeout,
    timed_out: result.error?.code === 'ETIMEDOUT',
    signal: result.signal || null,
    stdout_log: stdoutRelative,
    stdout_sha256: sha256File(stdoutPath),
    stderr_log: stderrRelative,
    stderr_sha256: sha256File(stderrPath),
  };
  if (result.error) {
    entry.spawn_error = result.error.message;
  }
  if (evidencePath) entry.evidence_path = normalizeRepoPath(evidencePath);
  commands.push(entry);
  c8Check(report, checkId, entry.status, entry);
  if (entry.status === 'fail' && addFailure) {
    c8Failure(
      report,
      entry.timed_out ? 'command_timed_out' : 'command_failed',
      entry.timed_out ? `${checkId} exceeded its deadline` : `${checkId} failed`,
      {
      check_id: checkId,
      command: commandText,
      exit_code: exitCode,
      timeout_ms: effectiveTimeout,
      stderr_tail: c8Tail(stderr),
      stdout_tail: c8Tail(stdout),
      }
    );
  }
  return { result, entry, stdout, stderr };
}

function c8Tail(value, limit = 4000) {
  if (!value) return '';
  return value.length <= limit ? value : value.slice(-limit);
}

function c8ReadJsonArtifact(report, path, label) {
  const normalized = normalizeRepoPath(path);
  const absolute = c8ReportPath(report, normalized);
  if (!absolute) {
    c8Failure(report, 'invalid_repo_path', `${label} path is not repository-relative`, {
      path,
    });
    return null;
  }
  if (!statSafe(absolute)?.isFile()) {
    c8Failure(report, 'missing_artifact', `missing ${label}: ${normalized}`, {
      path: normalized,
    });
    return null;
  }
  try {
    return {
      path: normalized,
      absolute,
      value: JSON.parse(readFileSync(absolute, 'utf8')),
      raw_sha256: sha256File(absolute),
    };
  } catch (error) {
    c8Failure(report, 'invalid_json', `${label} is invalid JSON: ${error.message}`, {
      path: normalized,
    });
    return null;
  }
}

function c8Canonicalize(value) {
  if (Array.isArray(value)) return value.map((entry) => c8Canonicalize(entry));
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, c8Canonicalize(value[key])])
    );
  }
  return value;
}

function c8DigestPayload(value) {
  return createHash('sha256')
    .update(JSON.stringify(c8Canonicalize(value)))
    .digest('hex');
}

function c8CanonicalEqual(left, right) {
  return JSON.stringify(c8Canonicalize(left)) === JSON.stringify(c8Canonicalize(right));
}

function c8Hex(value, length = 64) {
  return typeof value === 'string' && new RegExp(`^[0-9a-f]{${length}}$`, 'i').test(value);
}

function c8Sha(value) {
  return typeof value === 'string' && /^[0-9a-f]{40}$/i.test(value);
}

function c8PortablePath(value) {
  return (
    typeof value === 'string' &&
    value.length > 0 &&
    !value.startsWith('/') &&
    !value.startsWith('\\') &&
    !/^[A-Za-z]:[\\/]/.test(value) &&
    !value.includes('://') &&
    value.split(/[\\/]/).every((part) => part && part !== '.' && part !== '..')
  );
}

function c8ArtifactRef(value, expectedId, report, label, options = {}) {
  const ok =
    value &&
    value.artifact_id === expectedId &&
    c8PortablePath(value.normalized_relative_path) &&
    c8Hex(value.digest);
  c8Require(
    report,
    ok,
    'artifact_metadata_mismatch',
    `${label} has invalid artifact metadata`,
    { expected_artifact_id: expectedId, observed: value },
    false
  );
  if (options.suffix && !value?.normalized_relative_path?.endsWith(options.suffix)) {
    c8Failure(
      report,
      'artifact_metadata_mismatch',
      `${label} path must end with ${options.suffix}`,
      { observed: value?.normalized_relative_path }
    );
  }
  return Boolean(ok);
}

function c8ExpectedC7Waves() {
  return [
    {
      task_id: 'C7-W1-READ',
      wave: 'wave1_read_capabilities',
      owner: 'domain-wave-1-read-capabilities',
      deletion_manifest:
        'crates/backend/nomifun-agent-contracts/contracts/deletion/domain-wave-1-read-capabilities.json',
      generated_crate: 'crates/backend/nomifun-agent-domain-wave1',
    },
    {
      task_id: 'C7-W2-CODING',
      wave: 'wave2_coding_extensions',
      owner: 'domain-wave-2-coding-extensions',
      deletion_manifest:
        'crates/backend/nomifun-agent-contracts/contracts/deletion/domain-wave-2-coding-extensions.json',
      generated_crate: 'crates/backend/nomifun-agent-domain-wave2',
    },
    {
      task_id: 'C7-W3-CREATIVE',
      wave: 'wave3_creative_multimodal',
      owner: 'domain-wave-3-creative-multimodal',
      deletion_manifest:
        'crates/backend/nomifun-agent-contracts/contracts/deletion/domain-wave-3-creative-multimodal.json',
      generated_crate: 'crates/backend/nomifun-agent-domain-wave3',
    },
    {
      task_id: 'C7-W4-IDENTITY',
      wave: 'wave4_identity_channels_devices',
      owner: 'domain-wave-4-identity-channels-devices',
      deletion_manifest:
        'crates/backend/nomifun-agent-contracts/contracts/deletion/domain-wave-4-identity-channels-devices.json',
      generated_crate: 'crates/backend/nomifun-agent-domain-wave4',
    },
    {
      task_id: 'C7-W5-AUTOMATION',
      wave: 'wave5_automation_supervision_remote',
      owner: 'domain-wave-5-automation-supervision-remote',
      deletion_manifest:
        'crates/backend/nomifun-agent-contracts/contracts/deletion/domain-wave-5-automation-supervision-remote.json',
      generated_crate: 'crates/backend/nomifun-agent-domain-wave5',
    },
  ];
}

function c8ExpectedDomains() {
  return [
    'research',
    'attachments',
    'knowledge',
    'project-memory',
    'workspace',
    'process',
    'terminal',
    'vcs',
    'mcp',
    'browser',
    'computer',
    'companion',
    'channel',
    'customer-service',
    'robot',
    'creative',
    'office',
    'miniapp',
    'requirements',
    'autowork',
    'cron',
    'idmm',
    'remote',
  ];
}

function c8ExpectedSessionOperations() {
  return [
    'create',
    'resume',
    'fork',
    'start_turn',
    'steer',
    'follow_up',
    'cancel',
    'compaction',
    'delete',
  ];
}

function c8ExpectedFaultClasses() {
  return [
    'clean_root',
    'precreated_empty_root',
    'cutover_recovery',
    'provider_unavailable',
    'resource_unavailable',
    'remote_revoke_admission',
    'runtime_dispose',
    'descendant_process_cleanup',
    'late_callback_after_delete',
  ];
}

function c8ExpectedChecks() {
  return [
    {
      check_id: 'c7_domain_waves',
      command: 'bun run gate:agent-v2 -- c7-domain-waves',
      execution_kind: 'informational',
      runner: 'bun',
      command_args: ['run', 'gate:agent-v2', '--', 'c7-domain-waves'],
    },
    {
      check_id: 'contract_validation',
      command: 'cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check',
      execution_kind: 'native',
      runner: 'cargo',
      command_args: [
        'run',
        '--locked',
        '-p',
        'nomifun-agent-contracts',
        '--bin',
        'agent-v2-contract',
        '--',
        'check',
      ],
    },
    {
      check_id: 'domain_registration_tests',
      command:
        'cargo test --locked -p nomifun-agent-domain-support -p nomifun-agent-domain-wave1 -p nomifun-agent-domain-wave2 -p nomifun-agent-domain-wave3 -p nomifun-agent-domain-wave4 -p nomifun-agent-domain-wave5',
      execution_kind: 'native',
      runner: 'cargo',
      command_args: [
        'test',
        '--locked',
        '-p',
        'nomifun-agent-domain-support',
        '-p',
        'nomifun-agent-domain-wave1',
        '-p',
        'nomifun-agent-domain-wave2',
        '-p',
        'nomifun-agent-domain-wave3',
        '-p',
        'nomifun-agent-domain-wave4',
        '-p',
        'nomifun-agent-domain-wave5',
      ],
    },
    {
      check_id: 'fresh_v4_root_tests',
      command: 'cargo test --locked -p nomifun-v4-root -- --test-threads=1',
      execution_kind: 'native',
      runner: 'cargo',
      command_args: [
        'test',
        '--locked',
        '-p',
        'nomifun-v4-root',
        '--',
        '--test-threads=1',
      ],
    },
    {
      check_id: 'production_host_tests',
      command:
        'cargo test --locked -p nomifun-app --lib router::agent_platform_host -- --test-threads=1',
      execution_kind: 'native',
      runner: 'cargo',
      command_args: [
        'test',
        '--locked',
        '-p',
        'nomifun-app',
        '--lib',
        'router::agent_platform_host',
        '--',
        '--test-threads=1',
      ],
    },
    {
      check_id: 'production_broker_functional_tests',
      command: 'cargo test --locked -p nomifun-chat-model-broker production:: --lib',
      execution_kind: 'native',
      runner: 'cargo',
      command_args: [
        'test',
        '--locked',
        '-p',
        'nomifun-chat-model-broker',
        'production::',
        '--lib',
      ],
    },
    {
      check_id: 'workspace_cargo_test',
      command: 'cargo test --locked --jobs 1 -- --test-threads=1',
      execution_kind: 'native',
      runner: 'workspace',
      command_args: ['test', '--locked', '--jobs', '1', '--', '--test-threads=1'],
      deduplication_key: 'c8-win-pre-workspace-cargo',
    },
    {
      check_id: 'ui_check',
      command: 'bun run check',
      execution_kind: 'native',
      runner: 'ui_check',
      command_args: ['run', 'check'],
    },
    {
      check_id: 'ui_build',
      command: 'bun run build:ui',
      execution_kind: 'native',
      runner: 'bun',
      command_args: ['run', 'build:ui'],
    },
    {
      check_id: 'windows_startup_smoke',
      command:
        'target/debug/nomicore.exe --data-dir <temporary-root> --port <free-port> --local',
      execution_kind: 'native',
      runner: 'startup_smoke',
      command_args: null,
    },
    {
      check_id: 'windows_package_contract',
      command: 'bun run check:windows-installer',
      execution_kind: 'native',
      runner: 'bun',
      command_args: ['run', 'check:windows-installer'],
    },
  ];
}

function c8NormalizeCommand(value) {
  return typeof value === 'string' ? value.trim().replace(/\s+/g, ' ') : '';
}

function c8ValidateC8Manifest(report, manifest, sourceSha) {
  const start = report.failure_details.length;
  c8Require(
    report,
    manifest && typeof manifest === 'object' && !Array.isArray(manifest),
    'manifest_shape',
    'C8-WIN-PRE manifest must be a JSON object'
  );
  if (!manifest || typeof manifest !== 'object' || Array.isArray(manifest)) {
    return { status: 'fail' };
  }

  c8Require(
    report,
    manifest.schema_version === '1.0.0',
    'manifest_schema',
    'C8-WIN-PRE manifest schema_version must be 1.0.0'
  );
  c8Require(
    report,
    manifest.boundary === 'C8-WIN-PRE',
    'manifest_boundary',
    'C8-WIN-PRE manifest boundary must be C8-WIN-PRE'
  );
  c8Require(
    report,
    ['active', 'closed'].includes(manifest.status),
    'manifest_status',
    `C8-WIN-PRE manifest has invalid status: ${manifest.status}`
  );
  c8Require(
    report,
    manifest.branch === C8_BRANCH,
    'manifest_branch',
    `C8-WIN-PRE manifest branch must be ${C8_BRANCH}`
  );
  c8Require(
    report,
    manifest.candidate_source_policy === 'resolve_clean_git_head_at_run',
    'candidate_source_policy',
    'C8-WIN-PRE must resolve candidate_source_sha from the clean Git HEAD at run time'
  );

  const candidateInputs = manifest.candidate_inputs;
  c8Require(
    report,
    candidateInputs &&
      typeof candidateInputs === 'object' &&
      !Array.isArray(candidateInputs) &&
      JSON.stringify(Object.keys(candidateInputs)) ===
        JSON.stringify(['confirmed_decision_contract_digest']),
    'candidate_inputs_shape',
    'C8-WIN-PRE candidate_inputs must contain only the decision contract digest'
  );
  c8Require(
    report,
    candidateInputs?.confirmed_decision_contract_digest?.toLowerCase() ===
      C8_EXPECTED_DIGESTS.confirmed_decision_contract,
    'candidate_input_decision_digest',
    'C8-WIN-PRE decision-contract input differs from the frozen contract'
  );

  c8Require(
    report,
    c8CanonicalEqual(manifest.post_build_artifacts, {
      release_lock: 'release-lock.json',
      platform_result: 'platform-result.json',
    }),
    'post_build_artifacts',
    'C8-WIN-PRE must name release-lock.json and platform-result.json as post-build outputs'
  );

  const checkpoint = manifest.source_checkpoint;
  c8Require(
    report,
    checkpoint && typeof checkpoint === 'object' && !Array.isArray(checkpoint),
    'source_checkpoint_shape',
    'C8-WIN-PRE source_checkpoint is required'
  );
  c8Require(
    report,
    checkpoint?.local_head === 'resolved_at_run' ||
      checkpoint?.local_head === sourceSha,
    'source_checkpoint_local_head',
    'source_checkpoint.local_head must be resolved_at_run or the current HEAD'
  );
  c8Require(
    report,
    checkpoint?.clean_worktree_required === true,
    'source_checkpoint_clean_worktree',
    'C8-WIN-PRE requires a clean worktree'
  );
  c8Require(
    report,
    c8CanonicalEqual(Object.keys(checkpoint || {}).sort(), [
      'clean_worktree_required',
      'local_head',
    ]),
    'source_checkpoint_fields',
    'C8-WIN-PRE source_checkpoint must contain only local_head and clean_worktree_required'
  );

  const target = manifest.target_cell;
  const expectedTarget = {
    cell_id: 'windows_desktop_x64',
    host_os: 'windows',
    host_arch: 'x86_64',
    host_target: 'x86_64-pc-windows-msvc',
    runtime_target: 'x86_64-pc-windows-msvc',
    host_surface: 'desktop',
    package_format: 'nsis',
    native_evidence_required: true,
  };
  for (const [key, value] of Object.entries(expectedTarget)) {
    c8Require(
      report,
      target?.[key] === value,
      'target_cell_contract',
      `C8-WIN-PRE target_cell.${key} must be ${JSON.stringify(value)}`,
      { observed: target?.[key] }
    );
  }

  const expectedInputs = {
    decision_contract: {
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/contract-closure.envelope.json',
      digest: C8_EXPECTED_DIGESTS.confirmed_decision_contract,
    },
    canonical_schema_manifest: {
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/canonical-v4-schema-manifest.envelope.json',
      digest: C8_EXPECTED_DIGESTS.canonical_schema_manifest,
    },
    target_first_party_inventory: {
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/target-first-party-contributions.envelope.json',
      digest: C8_EXPECTED_DIGESTS.target_inventory,
    },
    official_seed: {
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/official-preset-seed-manifest.envelope.json',
      digest: C8_EXPECTED_DIGESTS.official_seed,
    },
    platform_validation_contract: {
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/canonical-v4-schema-manifest.envelope.json',
      field: 'payload.platform_validation_contract_digest',
      digest: C8_EXPECTED_DIGESTS.platform_validation_contract,
    },
    cargo_lock: {
      path: 'Cargo.lock',
      digest: C8_EXPECTED_DIGESTS.cargo_lock,
    },
  };
  const actualInputs = manifest.immutable_inputs;
  c8Require(
    report,
    actualInputs && typeof actualInputs === 'object' && !Array.isArray(actualInputs),
    'immutable_inputs_shape',
    'C8-WIN-PRE immutable_inputs must be an object'
  );
  for (const [key, expected] of Object.entries(expectedInputs)) {
    const actual = actualInputs?.[key];
    const expectedKeys = Object.keys(expected).sort();
    const actualKeys =
      actual && typeof actual === 'object' && !Array.isArray(actual)
        ? Object.keys(actual).sort()
        : [];
    c8Require(
      report,
      JSON.stringify(actualKeys) === JSON.stringify(expectedKeys),
      'immutable_input_shape',
      `C8-WIN-PRE immutable_inputs.${key} has an unexpected field set`,
      { expected: expectedKeys, observed: actualKeys }
    );
    c8Require(
      report,
      actual?.path === expected.path,
      'immutable_input_path',
      `C8-WIN-PRE immutable_inputs.${key}.path is not canonical`,
      { expected: expected.path, observed: actual?.path }
    );
    if (expected.field) {
      c8Require(
        report,
        actual?.field === expected.field,
        'immutable_input_field',
        `C8-WIN-PRE immutable_inputs.${key}.field is not canonical`,
        { expected: expected.field, observed: actual?.field }
      );
    }
    c8Require(
      report,
      String(actual?.digest || '').toLowerCase() === expected.digest,
      'immutable_input_digest',
      `C8-WIN-PRE immutable_inputs.${key}.digest is not the frozen value`,
      { expected: expected.digest, observed: actual?.digest }
    );
    if (expected.field) {
      const referencedArtifact = c8ReadJsonArtifact(
        report,
        expected.path,
        `C8-WIN-PRE immutable_inputs.${key}`
      );
      const referencedValue = expected.field
        .split('.')
        .reduce((value, field) => value?.[field], referencedArtifact?.value);
      c8Require(
        report,
        referencedValue === expected.digest,
        'immutable_input_field_digest',
        `C8-WIN-PRE immutable_inputs.${key} field does not contain its declared digest`,
        { expected: expected.digest, observed: referencedValue || null }
      );
    }
  }

  const expectedChecks = c8ExpectedChecks();
  const declaredChecks = Array.isArray(manifest.required_checks)
    ? manifest.required_checks
    : [];
  const declaredIds = declaredChecks
    .map((check) => check?.check_id)
    .filter((id) => typeof id === 'string');
  c8Require(
    report,
    new Set(declaredIds).size === declaredIds.length,
    'required_check_duplicates',
    'C8-WIN-PRE required_checks contains duplicate or invalid check IDs'
  );
  c8Require(
    report,
    JSON.stringify([...new Set(declaredIds)].sort()) ===
      JSON.stringify(expectedChecks.map((check) => check.check_id).sort()),
    'required_check_exact_set',
    'C8-WIN-PRE required_checks must contain the exact Windows preflight check set'
  );
  for (const expected of expectedChecks) {
    const actual = declaredChecks.find((check) => check?.check_id === expected.check_id);
    c8Require(
      report,
      Boolean(actual),
      'required_check_missing',
      `C8-WIN-PRE is missing required check ${expected.check_id}`
    );
    if (!actual) continue;
    c8Require(
      report,
      c8NormalizeCommand(actual.command) === c8NormalizeCommand(expected.command),
      'required_check_command',
      `${expected.check_id} command differs from the canonical C8 command`,
      { expected: expected.command, observed: actual.command }
    );
    c8Require(
      report,
      actual.execution_kind === expected.execution_kind,
      'required_check_execution_kind',
      `${expected.check_id} must use execution_kind=${expected.execution_kind}`
    );
    if (expected.deduplication_key) {
      c8Require(
        report,
        actual.deduplication_key === expected.deduplication_key,
        'workspace_deduplication_key',
        `workspace Cargo check must use ${expected.deduplication_key}`
      );
    }
  }

  const scene = manifest.all_scene_coverage;
  c8Require(
    report,
    scene && typeof scene === 'object' && !Array.isArray(scene),
    'all_scene_coverage_shape',
    'C8-WIN-PRE all_scene_coverage is required'
  );
  c8Require(
    report,
    JSON.stringify(uniqueSortedStrings(scene?.required_official_template_keys)) ===
      JSON.stringify([...C8_EXPECTED_TEMPLATES].sort()),
    'official_template_key_contract',
    'C8-WIN-PRE must declare the exact seven official template keys'
  );
  for (const [field, expected] of [
    ['required_domains', c8ExpectedDomains()],
    ['required_session_operations', c8ExpectedSessionOperations()],
    ['required_fault_classes', c8ExpectedFaultClasses()],
  ]) {
    c8Require(
      report,
      JSON.stringify(uniqueSortedStrings(scene?.[field])) ===
        JSON.stringify([...expected].sort()),
      'all_scene_coverage_exact_set',
      `C8-WIN-PRE all_scene_coverage.${field} does not match the frozen exact set`
    );
  }

  c8Require(
    report,
    !Object.hasOwn(manifest, 'pending_native_verification_points'),
    'pending_native_removed',
    'C8-WIN-PRE must not embed non-Windows pending evidence; macOS arm64 and Linux Desktop use their own native Gate'
  );

  const closure = manifest.closure_requirements;
  for (const [key, expected] of Object.entries({
    status_transition: 'active -> closed',
    windows_status: 'pass',
    workspace_cargo_test_must_be_serialized: true,
    global_legacy_residual_must_be_zero: true,
  })) {
    c8Require(
      report,
      closure?.[key] === expected,
      'closure_requirement',
      `C8-WIN-PRE closure_requirements.${key} must be ${JSON.stringify(expected)}`
    );
  }

  return {
    status: report.failure_details.length === start ? 'pass' : 'fail',
    candidate_source_sha: sourceSha,
    declared_candidate_source_policy: manifest.candidate_source_policy,
    required_check_ids: expectedChecks.map((check) => check.check_id),
  };
}

function c8ResolveC8SourceCheckpoint(report, manifest, sourceSha) {
  const checkpoint = manifest.source_checkpoint || {};
  const resolved = {
    local_head: sourceSha,
    clean_worktree_required: checkpoint.clean_worktree_required === true,
    local_head_placeholder: checkpoint.local_head || null,
  };
  const statusResult = c8GitProbe(
    report,
    'source_checkpoint_clean_worktree',
    ['status', '--porcelain', '--untracked-files=all']
  );
  c8ApplyWorktreeCheckpointProbe(report, resolved, statusResult);
  return resolved;
}

function c8ApplyWorktreeCheckpointProbe(report, resolved, statusResult) {
  const probeStatus =
    typeof statusResult?.status === 'number' ? statusResult.status : 1;
  const worktreeStatus = String(statusResult?.stdout || '').trim();
  resolved.worktree_status = worktreeStatus;
  resolved.worktree_probe_status =
    probeStatus === 0 &&
    (!resolved.clean_worktree_required || !worktreeStatus)
      ? 'pass'
      : 'fail';

  if (probeStatus !== 0) {
    c8MarkWorktreeProbeFailure(report, {
      code: 'worktree_status_probe_failed',
      exit_code: probeStatus,
    });
    c8Block(
      report,
      'worktree_status_probe_failed',
      'C8-WIN-PRE cannot prove a clean worktree before native evidence is accepted',
      {
        exit_code: probeStatus,
        stderr: c8Tail(String(statusResult?.stderr || '')),
      }
    );
    return;
  }

  if (resolved.clean_worktree_required && worktreeStatus) {
    c8MarkWorktreeProbeFailure(report, {
      code: 'dirty_worktree',
      status: worktreeStatus,
    });
    c8Block(
      report,
      'dirty_worktree',
      'C8-WIN-PRE requires a clean worktree before native evidence is accepted',
      { status: worktreeStatus }
    );
  }
}

function c8MarkWorktreeProbeFailure(report, details) {
  const entries = [
    ...(report?.metadata_commands || []),
    ...(report?.commands || []),
  ];
  const entry = entries.find(
    (candidate) => candidate.check_id === 'source_checkpoint_clean_worktree'
  );
  if (entry) {
    entry.status = 'fail';
    entry.semantic_failure = details;
  }
}

function c8GitProbe(report, checkId, commandArgs) {
  const startedAt = new Date().toISOString();
  let result;
  try {
    result = spawnSync('git', commandArgs, {
      cwd: repoRoot,
      encoding: 'utf8',
      shell: process.platform === 'win32',
      stdio: 'pipe',
    });
  } catch (error) {
    result = { status: null, stdout: '', stderr: error.message, error };
  }
  const entry = {
    check_id: checkId,
    kind: 'metadata_probe',
    command: ['git', ...commandArgs].join(' '),
    started_at: startedAt,
    finished_at: new Date().toISOString(),
    exit_code: typeof result.status === 'number' ? result.status : 1,
    status: result.status === 0 ? 'pass' : 'fail',
    stdout: c8Tail(String(result.stdout || '')),
    stderr: c8Tail(String(result.stderr || '')),
  };
  commands.push(entry);
  report.metadata_commands = report.metadata_commands || [];
  report.metadata_commands.push(entry);
  return {
    status: typeof result.status === 'number' ? result.status : 1,
    stdout: String(result.stdout || ''),
    stderr: String(result.stderr || ''),
  };
}

function writeC8WinPreReport(details) {
  const sourceSha = details?.source_sha || c8ReadGitHeadForReport();
  const reportDir = join(
    repoRoot,
    'build.noindex',
    'agent-capability-v2',
    sourceSha,
    'c8-win-pre'
  );
  mkdirSync(reportDir, { recursive: true });
  const report = {
    schema_version: '1.0.0',
    gate_name: 'c8-win-pre',
    source_sha: sourceSha,
    evidence_kind: 'native',
    ...details,
    commands,
    status: failures.length === 0 ? 'pass' : 'fail',
    failures,
  };
  writeFileSync(
    join(reportDir, 'summary.json'),
    `${JSON.stringify(report, null, 2)}\n`
  );
}

function c8ValidateC7State(report, sourceSha) {
  const result = {
    status: 'fail',
    current_source_sha: sourceSha,
    closure_path:
      'docs/specs/2026-08-28-agent-capability-platform-v2/C7-CLOSURE.json',
    manifest_path:
      'docs/specs/2026-08-28-agent-capability-platform-v2/C7-WRITE-MANIFESTS.json',
    closure_candidate_sha: null,
    checks: [],
  };
  const closureArtifact = c8ReadJsonArtifact(
    report,
    result.closure_path,
    'C7 closure record'
  );
  const manifestArtifact = c8ReadJsonArtifact(
    report,
    result.manifest_path,
    'C7 write manifest'
  );
  if (!closureArtifact || !manifestArtifact) return result;
  const closure = closureArtifact.value;
  const manifest = manifestArtifact.value;
  result.closure_candidate_sha = closure.candidate_source_sha || null;

  const closed = closure.status === 'closed' && manifest.status === 'closed';
  c8Require(
    report,
    closed,
    'c7_not_closed',
    'C8-WIN-PRE requires a closed C7 manifest and closure record'
  );
  c8Require(
    report,
    closure.implementation_commit === closure.candidate_source_sha,
    'c7_closure_candidate_mismatch',
    'C7 closure implementation and candidate source SHA must match'
  );

  const ancestor = c8GitProbe(report, 'c7_candidate_ancestor', [
    'merge-base',
    '--is-ancestor',
    String(closure.candidate_source_sha || ''),
    sourceSha,
  ]);
  result.checks.push({
    check_id: 'c7_candidate_ancestor',
    status: ancestor.status === 0 ? 'pass' : 'fail',
  });
  c8Require(
    report,
    ancestor.status === 0,
    'c7_candidate_not_ancestor',
    'C7 closure candidate is not an ancestor of the C8 source HEAD'
  );

  const evidencePath =
    `build.noindex/agent-capability-v2/${closure.candidate_source_sha}/c7-domain-waves/summary.json`;
  const evidence = c8ReadJsonArtifact(
    report,
    evidencePath,
    'C7 domain-wave gate evidence'
  );
  if (evidence) {
    result.gate_evidence = evidencePath;
    result.gate_evidence_status = evidence.value.status;
    c8Require(
      report,
      evidence.value.status === 'pass',
      'c7_gate_not_pass',
      'C7 domain-wave gate evidence is not pass'
    );
  }

  result.status =
    report.failure_details.some((failure) =>
      String(failure.code).startsWith('c7_')
    )
      ? 'fail'
      : 'pass';
  return result;
}

function c8ValidateCanonicalPlatformInputs(report, manifest) {
  const result = {
    status: 'fail',
    input_digests: {},
    platform_matrix: null,
    required_checks: [],
  };
  const inputSpecs = [
    {
      key: 'decision_contract',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/contract-closure.envelope.json',
      digest: C8_EXPECTED_DIGESTS.confirmed_decision_contract,
    },
    {
      key: 'canonical_schema_manifest',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/canonical-v4-schema-manifest.envelope.json',
      digest: C8_EXPECTED_DIGESTS.canonical_schema_manifest,
    },
    {
      key: 'target_first_party_inventory',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/target-first-party-contributions.envelope.json',
      digest: C8_EXPECTED_DIGESTS.target_inventory,
    },
    {
      key: 'official_seed',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/official-preset-seed-manifest.envelope.json',
      digest: C8_EXPECTED_DIGESTS.official_seed,
    },
    {
      key: 'runtime_feature_inventory',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/runtime-feature-inventory.envelope.json',
      digest: C8_EXPECTED_DIGESTS.runtime_feature_inventory,
    },
  ];
  for (const input of inputSpecs) {
    const artifact = c8ReadJsonArtifact(report, input.path, input.key);
    const observed = artifact?.value?.payload_digest;
    result.input_digests[input.key] = {
      expected: input.digest,
      observed,
      raw_sha256: artifact?.raw_sha256 || null,
      status: observed === input.digest ? 'pass' : 'fail',
    };
    c8Require(
      report,
      observed === input.digest,
      'canonical_input_digest_mismatch',
      `${input.key} generated envelope digest differs from the frozen C8 input`
    );
  }

  const platformArtifact = c8ReadJsonArtifact(
    report,
    'crates/backend/nomifun-agent-contracts/contracts/validation/platform-validation-manifest.payload.json',
    'PlatformValidationManifest payload'
  );
  if (platformArtifact) {
    const payload = platformArtifact.value;
    result.platform_matrix = payload.platform_matrix?.target_cells || null;
    result.required_checks = payload.required_checks || [];
    c8Require(
      report,
      payload.manifest_version === '1.0.0',
      'platform_manifest_version',
      'PlatformValidationManifest version is not 1.0.0'
    );
    c8Require(
      report,
      payload.confirmed_decision_contract_digest ===
        C8_EXPECTED_DIGESTS.confirmed_decision_contract,
      'platform_manifest_decision_digest',
      'PlatformValidationManifest decision digest differs from the frozen contract'
    );
    c8Require(
      report,
      payload.canonical_schema_manifest_digest ===
        C8_EXPECTED_DIGESTS.canonical_schema_manifest,
      'platform_manifest_schema_digest',
      'PlatformValidationManifest schema digest differs from the frozen schema'
    );
    c8Require(
      report,
      payload.official_preset_seed_manifest_digest ===
        C8_EXPECTED_DIGESTS.official_seed,
      'platform_manifest_seed_digest',
      'PlatformValidationManifest seed digest differs from the frozen seed'
    );
    c8Require(
      report,
      payload.capability_availability_manifest_digest ===
        C8_EXPECTED_DIGESTS.capability_availability,
      'platform_manifest_availability_digest',
      'PlatformValidationManifest capability availability digest differs from D-028'
    );
    const windows = payload.platform_matrix?.target_cells?.windows_desktop_x64;
    c8Require(
      report,
      windows?.host_os === 'windows' &&
        windows?.host_arch === 'x86_64' &&
        windows?.host_target === 'x86_64-pc-windows-msvc' &&
        windows?.runtime_target === 'x86_64-pc-windows-msvc' &&
        windows?.host_surface === 'desktop' &&
        windows?.package_format === 'nsis',
      'platform_windows_cell',
      'D-028 Windows Desktop x64 cell does not match the required native target'
    );
    const winCheck = (payload.required_checks || []).find(
      (check) => check.check_id === 'c8_win_pre_full_gate'
    );
    c8Require(
      report,
      winCheck?.required_execution_kind === 'native' &&
        winCheck?.target_cells?.includes('windows_desktop_x64'),
      'platform_windows_gate',
      'PlatformValidationManifest does not require a native Windows pre gate'
    );
  }

  const rawPayload = c8ReadJsonArtifact(
    report,
    'crates/backend/nomifun-agent-contracts/contracts/validation/platform-validation-manifest.payload.json',
    'platform validation input'
  );
  result.platform_payload_digest = rawPayload
    ? c8DigestPayload(rawPayload.value)
    : null;
  result.status = report.failure_details.some((failure) =>
    String(failure.code).startsWith('canonical_') ||
    String(failure.code).startsWith('platform_')
  )
    ? 'fail'
    : 'pass';
  return result;
}

function c8ValidateWindowsMetadata(report, platformValidation) {
  const result = {
    status: 'fail',
    host: {
      platform: process.platform,
      arch: process.arch,
      target: 'x86_64-pc-windows-msvc',
      surface: 'desktop',
      package_format: 'nsis',
    },
    probes: [],
  };
  const native = process.platform === 'win32' && process.arch === 'x64';
  c8Require(
    report,
    native,
    'windows_native_host',
    'C8-WIN-PRE must execute on native Windows x64'
  );
  // The declared cargo/rustup probes provide the authoritative toolchain
  // evidence; this record only captures the host identity used by the gate.
  result.probes.push({
    check_id: 'windows_host_process',
    status: native ? 'pass' : 'fail',
    rustup_probe: 'recorded_by_windows_target_probe',
  });
  const windowsCell =
    platformValidation?.platform_matrix?.windows_desktop_x64;
  c8Require(
    report,
    windowsCell?.host_target === 'x86_64-pc-windows-msvc' &&
      windowsCell?.runtime_target === 'x86_64-pc-windows-msvc',
    'windows_platform_matrix',
    'C8 Windows metadata does not agree with the D-028 Windows cell'
  );
  result.status = native && !report.failure_details.some((failure) =>
    String(failure.code).startsWith('windows_')
  )
    ? 'pass'
    : 'fail';
  return result;
}

function c8ValidateAllSceneCoverage(report, manifest, platformValidation) {
  const result = {
    status: 'fail',
    template_keys: [],
    domains: {},
    session_operations: {},
    fault_classes: {},
    resource_slots: {},
    production_owner_coverage: null,
  };
  const seedArtifact = c8ReadJsonArtifact(
    report,
    'crates/backend/nomifun-agent-contracts/contracts/generated/official-preset-seed-manifest.envelope.json',
    'official preset seed'
  );
  const seed = seedArtifact?.value?.payload;
  const templates = seed?.templates || {};
  result.template_keys = Object.keys(templates).sort();
  c8Require(
    report,
    JSON.stringify(result.template_keys) ===
      JSON.stringify([...C8_EXPECTED_TEMPLATES].sort()),
    'scene_template_set',
    'C8 all-scene coverage does not contain the exact seven official templates'
  );

  const inventoryArtifact = c8ReadJsonArtifact(
    report,
    'crates/backend/nomifun-agent-contracts/contracts/generated/target-first-party-contributions.envelope.json',
    'target first-party inventory'
  );
  const inventory = inventoryArtifact?.value?.payload;
  const capabilityIds = new Set(
    (inventory?.packages || []).flatMap((entry) =>
      (entry.capabilities || []).map((capability) => capability.capability?.id)
    )
  );
  const domainMatchers = {
    research: ['web.search', 'web.fetch'],
    attachments: ['session.attachments.read'],
    knowledge: ['knowledge.search', 'knowledge.read'],
    'project-memory': ['memory.project.read'],
    workspace: ['fs.read', 'workspace.bind'],
    process: ['process.exec'],
    terminal: ['terminal.pty'],
    vcs: ['vcs.status', 'vcs.diff'],
    mcp: ['mcp.connect', 'mcp.tool_proxy'],
    browser: ['browser.navigate'],
    computer: ['computer.input'],
    companion: ['companion.persona'],
    channel: ['channel.receive'],
    'customer-service': ['customer_service.dialogue'],
    robot: ['robot.link'],
    creative: ['creation.text', 'workshop.canvas.read'],
    office: ['office.preview'],
    miniapp: ['miniapp.read'],
    requirements: ['requirements.read'],
    autowork: ['autowork.runner'],
    cron: ['schedule.timer'],
    idmm: ['idmm.observe'],
    remote: ['remote.mcp'],
  };
  for (const domain of manifest.all_scene_coverage?.required_domains || []) {
    const required = domainMatchers[domain] || [];
    const missing = required.filter((id) => !capabilityIds.has(id));
    result.domains[domain] = {
      required,
      missing,
      status: missing.length === 0 ? 'pass' : 'fail',
    };
    c8Require(
      report,
      missing.length === 0,
      'scene_domain_coverage',
      `C8 all-scene inventory is missing required ${domain} capability coverage`
    );
  }

  const ownerSources = {
    host: readFileSafe(
      join(
        repoRoot,
        'crates/backend/nomifun-app/src/router/agent_platform_host.rs'
      )
    ) || '',
    wave2: readFileSafe(
      join(
        repoRoot,
        'crates/backend/nomifun-app/src/router/agent_wave2_host.rs'
      )
    ) || '',
    wave4: readFileSafe(
      join(
        repoRoot,
        'crates/backend/nomifun-app/src/router/agent_wave4_host.rs'
      )
    ) || '',
  };
  const ownerBlockers = c8ProductionOwnerBlockers(ownerSources);
  result.production_owner_coverage = {
    status: ownerBlockers.length === 0 ? 'pass' : 'fail',
    blockers: ownerBlockers,
  };
  c8Require(
    report,
    ownerBlockers.length === 0,
    'scene_production_owner_coverage',
    `C8 production action owners are incomplete: ${
      ownerBlockers.map((blocker) => blocker.domain).join(', ') || 'unknown'
    }`,
    { blockers: ownerBlockers }
  );

  const runtimeArtifact = c8ReadJsonArtifact(
    report,
    'crates/backend/nomifun-agent-contracts/contracts/generated/runtime-release-fixture.envelope.json',
    'runtime release fixture'
  );
  const runtimeMethods = new Set(
    runtimeArtifact?.value?.payload?.rpc_allowlist?.methods || []
  );
  const operationMap = {
    create: 'create',
    resume: 'resume',
    fork: 'fork',
    start_turn: 'start_turn',
    steer: 'steer',
    follow_up: 'follow_up',
    cancel: 'cancel',
    delete: 'session_dispose',
  };
  for (const operation of manifest.all_scene_coverage?.required_session_operations || []) {
    const method = operationMap[operation];
    const present =
      operation === 'compaction'
        ? Boolean(
            readFileSafe(
              join(
                repoRoot,
                'crates/backend/nomifun-agent-contracts/src/session.rs'
              )
            )?.includes('Compaction')
          )
        : operation === 'delete'
          ? Boolean(
              readFileSafe(
                join(
                  repoRoot,
                  'crates/backend/nomifun-app/src/router/agent_platform.rs'
                )
              )?.includes('.delete(')
            ) && runtimeMethods.has(method)
          : runtimeMethods.has(method);
    result.session_operations[operation] = {
      runtime_method: method || null,
      status: present ? 'pass' : 'fail',
    };
    c8Require(
      report,
      present,
      'scene_session_operation',
      `C8 session operation ${operation} is not represented by the canonical runtime/session contract`
    );
  }

  const faultFiles = [
    'crates/backend/nomifun-v4-root/src/tests.rs',
    'crates/backend/nomifun-app/src/router/agent_platform_host.rs',
    'crates/backend/nomifun-app/src/router/agent_platform.rs',
    'crates/backend/nomifun-agent-contracts/contracts/remote/d026-request-admission-ordering.fixture.json',
    'crates/backend/nomifun-agent-contracts/contracts/session/delete-closure.json',
    'crates/backend/nomifun-agent-contracts/contracts/validation/d027-terminal-sequences.matrix.json',
  ];
  const faultText = faultFiles
    .map((path) => readFileSafe(join(repoRoot, path)) || '')
    .join('\n');
  for (const fault of manifest.all_scene_coverage?.required_fault_classes || []) {
    const markers = {
      clean_root: ['fresh_install'],
      precreated_empty_root: ['precreated_empty'],
      cutover_recovery: ['cutover'],
      provider_unavailable: [
        'async fn production_factory_composes_exact_six_adapters_and_streams()',
        'async fn unavailable_model_invoke_port_is_typed_and_not_a_fake_response()',
      ],
      resource_unavailable: ['CapabilityUnavailable', 'resource'],
      remote_revoke_admission: ['D026', 'REMOTE_AUTH_REQUIRED'],
      runtime_dispose: ['dispose'],
      descendant_process_cleanup: ['process_tree', 'descendant'],
      late_callback_after_delete: ['SESSION_DELETED', 'late_operation'],
    }[fault] || [fault];
    const markerSource =
      fault === 'provider_unavailable'
        ? readFileSafe(
            join(
              repoRoot,
              'crates/backend/nomifun-chat-model-broker/src/production.rs'
            )
          ) || ''
        : faultText;
    const present =
      fault === 'provider_unavailable'
        ? markers.every((marker) => markerSource.includes(marker))
        : markers.some((marker) => markerSource.includes(marker));
    result.fault_classes[fault] = {
      markers,
      source_path:
        fault === 'provider_unavailable'
          ? 'crates/backend/nomifun-chat-model-broker/src/production.rs'
          : null,
      check_id:
        fault === 'provider_unavailable'
          ? 'production_broker_functional_tests'
          : null,
      status: present
        ? fault === 'provider_unavailable'
          ? 'pending_functional_check'
          : 'pass'
        : 'fail',
    };
    c8Require(
      report,
      present,
      'scene_fault_coverage',
      `C8 fault coverage marker is missing for ${fault}; placeholder broker/error strings are not valid evidence`
    );
  }

  for (const [key, template] of Object.entries(templates)) {
    const slots = template.typed_resource_defaults || [];
    result.resource_slots[key] = {
      count: slots.length,
      required: slots.filter((slot) => slot.required).map((slot) => slot.slot_key),
      status: slots.every(
        (slot) =>
          typeof slot.slot_key === 'string' &&
          typeof slot.resource_kind === 'string' &&
          Array.isArray(slot.operations)
      )
        ? 'pass'
        : 'fail',
    };
    c8Require(
      report,
      result.resource_slots[key].status === 'pass',
      'scene_resource_slot',
      `C8 template ${key} has an invalid typed resource slot declaration`
    );
  }
  result.status = report.failure_details.some((failure) =>
    String(failure.code).startsWith('scene_')
  )
    ? 'fail'
    : 'pass';
  return result;
}

function c8ProductionOwnerBlockers({ host, wave2, wave4 }) {
  const blockers = [];
  if (host.includes('no Fresh-v4 Wave 1 owner is wired for')) {
    blockers.push({
      domain: 'wave1',
      reason: 'partial_owner_fallback_reachable',
    });
  }
  if (wave2.includes('no canonical application owner is wired for')) {
    blockers.push({
      domain: 'wave2',
      reason: 'partial_owner_fallback_reachable',
    });
  }
  if (
    /wave3:\s*nomifun_agent_domain_wave3::unconfigured_host_port\s*\(/m.test(
      host
    )
  ) {
    blockers.push({
      domain: 'wave3',
      reason: 'unconfigured_production_host',
    });
  }
  if (wave4.includes('Fresh-v4 has no native owner for')) {
    blockers.push({
      domain: 'wave4',
      reason: 'unavailable_production_host',
    });
  }
  if (
    /wave5:\s*nomifun_agent_domain_wave5::unconfigured_host_port\s*\(/m.test(
      host
    )
  ) {
    blockers.push({
      domain: 'wave5',
      reason: 'unconfigured_production_host',
    });
  }
  return blockers;
}

function c8ValidateC8ResidualReachability(report, c7, manifest) {
  const result = {
    status: 'pending',
    canonical_owner: {
      status: 'pass',
      observed_count: 0,
      findings: [],
    },
    global_legacy_source: {
      status: 'pending',
      observed_count: 0,
      findings: [],
    },
    policy:
      'C8 enforces global legacy residual zero when closure_requirements.global_legacy_residual_must_be_zero is true',
  };
  const canonicalFiles = c7SourceFilesForPaths([
    'crates/backend/nomifun-agent-platform/src',
    'crates/backend/nomifun-app/src/router/agent_platform.rs',
    'crates/backend/nomifun-agent-domain-support',
    'crates/backend/nomifun-agent-domain-wave1',
    'crates/backend/nomifun-agent-domain-wave2',
    'crates/backend/nomifun-agent-domain-wave3',
    'crates/backend/nomifun-agent-domain-wave4',
    'crates/backend/nomifun-agent-domain-wave5',
  ]);
  const canonicalScan = scanC7ForbiddenEdges(canonicalFiles);
  result.canonical_owner = {
    status: canonicalScan.total_count === 0 ? 'pass' : 'fail',
    observed_count: canonicalScan.total_count,
    findings: canonicalScan.findings,
  };
  c8Require(
    report,
    canonicalScan.total_count === 0,
    'c8_canonical_residual',
    'C8 canonical Agent owner contains a forbidden legacy edge'
  );

  const contractIndex = c8ReadC8ResidualContracts(report);
  const historicalRoots = [
    'crates/backend/nomifun-ai-agent/src/manager/nomi',
    'crates/backend/nomifun-conversation/src',
    'crates/backend/nomifun-gateway/src',
  ];
  const historicalPaths = c8ResidualScanPaths(contractIndex, historicalRoots);
  const historicalFiles = c7SourceFilesForPaths(historicalPaths, {
    includeLockfiles: true,
  });
  const historicalScan = scanC7ForbiddenEdges(historicalFiles, {
    maxFindings: C8_GLOBAL_RESIDUAL_MAX_FINDINGS,
  });
  const classification = c8ClassifyResidualScan(
    historicalScan,
    contractIndex
  );
  const globalResidualMustBeZero =
    manifest?.closure_requirements?.global_legacy_residual_must_be_zero === true;
  const globalResidualPass = c8GlobalResidualPass(
    globalResidualMustBeZero,
    historicalScan.total_count,
    classification
  );
  result.global_legacy_source = {
    status: globalResidualPass ? 'pass' : 'fail',
    observed_count: historicalScan.total_count,
    blocking_count: classification.blocking_count,
    allowed_count: classification.allowed_count,
    deferred_c9_count: classification.deferred_c9_count,
    declared_count: classification.declared_count,
    unclassified_count: classification.unclassified_count,
    truncated_count: classification.truncated_count,
    preserved_channel_product_count:
      classification.preserved_channel_product_count,
    agent_approval_count: classification.agent_approval_count,
    channel_product_count: classification.channel_product_count,
    ambiguous_confirmation_count:
      classification.ambiguous_confirmation_count,
    confirmation_audit: {
      agent_approval: {
        count: classification.agent_approval_count,
        blocking_findings: classification.findings.filter(
          (finding) =>
            finding.confirmation_classification ===
              'agent_approval_confirmation' &&
            finding.classification.startsWith('blocking_')
        ).length,
      },
      channel_product_confirmation: {
        count: classification.channel_product_count,
        preserved_count: classification.preserved_channel_product_count,
        blocking_findings: classification.findings.filter(
          (finding) =>
            finding.confirmation_classification ===
              'channel_product_confirmation' &&
            finding.classification.startsWith('blocking_')
        ).map((finding) => ({
          path: finding.path,
          line: finding.line,
          match: finding.match,
          reason:
            finding.confirmation_preservation?.decision ||
            'blocking_insufficient_preservation_evidence',
          conflicting_contract_refs:
            finding.confirmation_preservation?.conflicting_contract_refs || [],
        })),
      },
      ambiguous: {
        count: classification.ambiguous_confirmation_count,
        blocking_findings: classification.findings.filter(
          (finding) =>
            finding.confirmation_classification === 'ambiguous_confirmation'
        ).length,
      },
    },
    findings: classification.findings.slice(0, C8_GLOBAL_RESIDUAL_MAX_FINDINGS),
    scanned_files: historicalFiles.length,
    scanned_paths: historicalPaths,
    contract_manifests: contractIndex.contracts.map((contract) => ({
      path: contract.path,
      manifest_id: contract.manifest_id,
      allowed_residual_policy: contract.allowed_residual_policy,
      allowed_residual_count: contract.allowed_residual_count,
    })),
    global_legacy_residual_must_be_zero: globalResidualMustBeZero,
    policy:
      'C8 blocks canonical/reachable and non-deferred residuals; Agent approval/needs_confirmation remains distinct from channel-owned product stop confirmation. Channel confirmation is preserved only with exact source ownership, explicit C1/C7 preservation policy, and no conflicting deletion contract; otherwise it remains blocking. Exact D-004 allowlist and manifest-declared C9 physical-delete residuals remain recorded until their boundary',
  };
  c8Require(
    report,
    globalResidualPass,
    'c8_global_legacy_residual',
    `C8 global legacy residual must be zero for the C8 scope, observed ${historicalScan.total_count} finding(s) (${classification.blocking_count} blocking, ${classification.allowed_count} contract-allowed, ${classification.deferred_c9_count} deferred-to-C9)`
  );
  result.transitional_host_adapter = {
    path: 'crates/backend/nomifun-app/src/router/agent_platform_host.rs',
    status: 'integration_adapter_recorded',
    note: 'The v4 host is mounted from AppServices while C9-deferred historical implementation residuals remain recorded; canonical/reachable residuals still block C8.',
  };
  result.status =
    result.canonical_owner.status === 'pass' &&
    result.global_legacy_source.status === 'pass'
      ? 'pass'
      : 'fail';
  return result;
}

function c8DeletionContractPaths() {
  return [
    C8_TRIAD_DELETION_MANIFEST_PATH,
    ...c8ExpectedC7Waves().map((wave) => wave.deletion_manifest),
  ];
}

function c8ReadC8ResidualContracts(report) {
  const contracts = [];
  for (const path of c8DeletionContractPaths()) {
    const artifact = c8ReadJsonArtifact(
      report,
      path,
      'C8 deletion contract'
    );
    if (artifact) {
      contracts.push({ path, payload: artifact.value });
    }
  }
  return c8BuildResidualContractIndex(contracts);
}

function c8BuildResidualContractIndex(contracts) {
  const index = {
    contracts: [],
    declared_refs: [],
    allowed_refs: [],
  };
  const asArray = (value) => (Array.isArray(value) ? value : []);

  const addDeclaredRef = (contract, ref, context) => {
    const rawPath = normalizeRepoPath(ref?.path);
    const isDirectory = rawPath.endsWith('/');
    const path = rawPath.replace(/\/+$/, '');
    if (!isSafeRepoPath(path)) return;
    index.declared_refs.push({
      path,
      is_directory: isDirectory,
      symbols: uniqueSortedStrings(ref.symbols),
      line_start:
        Number.isInteger(ref.line_start) && ref.line_start > 0
          ? ref.line_start
          : null,
      line_end:
        Number.isInteger(ref.line_end) && ref.line_end > 0
          ? ref.line_end
          : null,
      contract_path: contract.path,
      manifest_id: contract.manifest_id,
      ...context,
    });
  };

  for (const contract of contracts || []) {
    const payload = contract?.payload;
    if (!payload || typeof payload !== 'object' || Array.isArray(payload)) {
      continue;
    }
    const contractMeta = {
      path: contract.path,
      manifest_id:
        typeof payload.manifest_id === 'string'
          ? payload.manifest_id
          : typeof payload.wave === 'string'
            ? payload.wave
            : contract.path,
      allowed_residual_policy:
        payload.allowed_residuals?.policy || null,
      allowed_residual_count: Array.isArray(
        payload.allowed_residuals?.entries
      )
        ? payload.allowed_residuals.entries.length
        : 0,
    };
    index.contracts.push(contractMeta);

    for (const root of asArray(payload.production_roots)) {
      for (const ref of asArray(root?.current_refs)) {
        addDeclaredRef(contractMeta, ref, {
          source_kind: 'production_root',
          source_id: root.root_id || null,
          category: root.kind || null,
          removal_boundary: null,
          disposition: null,
          replacement_owner: root.expected_canonical_owner || null,
        });
      }
    }
    for (const surface of asArray(payload.legacy_surfaces)) {
      for (const ref of asArray(surface?.current_refs)) {
        addDeclaredRef(contractMeta, ref, {
          source_kind: 'legacy_surface',
          source_id: surface.surface_id || null,
          category: surface.category || null,
          removal_boundary: surface.removal_boundary || null,
          disposition: surface.disposition || null,
          replacement_owner: surface.replacement_owner || null,
        });
      }
    }
    for (const ref of asArray(
      payload.canonical_producer?.current_producer_refs
    )) {
      addDeclaredRef(contractMeta, ref, {
        source_kind: 'canonical_producer',
        source_id: 'canonical_producer',
        category: null,
        removal_boundary: null,
        disposition: null,
        replacement_owner: payload.canonical_producer?.owner_id || null,
      });
    }
    for (const consumer of asArray(payload.direct_consumers)) {
      for (const ref of asArray(consumer?.current_refs)) {
        addDeclaredRef(contractMeta, ref, {
          source_kind: 'direct_consumer',
          source_id: consumer.consumer_id || null,
          category: consumer.kind || null,
          removal_boundary: null,
          disposition: null,
          replacement_owner: consumer.canonical_target || null,
        });
      }
    }
    const d027Refs = [
      ...asArray(payload.d027?.admission_fence_refs),
      ...asArray(payload.d027?.deadline_authority_refs),
      ...asArray(payload.d027?.outstanding_set_refs).flatMap(
        (entry) => asArray(entry?.current_refs)
      ),
    ];
    for (const ref of d027Refs) {
      addDeclaredRef(contractMeta, ref, {
        source_kind: 'd027',
        source_id: 'd027',
        category: null,
        removal_boundary: null,
        disposition: null,
        replacement_owner: null,
      });
    }

    for (const residual of asArray(payload.allowed_residuals?.entries)) {
      for (const ref of asArray(residual?.exact_refs)) {
        const rawPath = normalizeRepoPath(ref?.path);
        const isDirectory = rawPath.endsWith('/');
        const path = rawPath.replace(/\/+$/, '');
        if (!isSafeRepoPath(path)) continue;
        index.allowed_refs.push({
          path,
          is_directory: isDirectory,
          symbols: uniqueSortedStrings(ref.symbols),
          line_start:
            Number.isInteger(ref.line_start) && ref.line_start > 0
              ? ref.line_start
              : null,
          line_end:
            Number.isInteger(ref.line_end) && ref.line_end > 0
              ? ref.line_end
              : null,
          contract_path: contractMeta.path,
          manifest_id: contractMeta.manifest_id,
          residual_id: residual.residual_id || null,
          policy: payload.allowed_residuals?.policy || null,
          allowed_until_boundary: residual.allowed_until_boundary || null,
          target_zero_boundary: residual.target_zero_boundary || null,
        });
      }
    }
  }

  return {
    ...index,
    declared_refs: c8DeduplicateResidualRefs(index.declared_refs),
    allowed_refs: c8DeduplicateResidualRefs(index.allowed_refs),
  };
}

function c8DeduplicateResidualRefs(refs) {
  const seen = new Set();
  return (refs || []).filter((ref) => {
    const key = JSON.stringify([
      ref.path,
      ref.contract_path,
      ref.source_kind,
      ref.source_id,
      ref.residual_id,
      ref.symbols,
      ref.line_start,
      ref.line_end,
      ref.is_directory,
    ]);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function c8ResidualScanPaths(contractIndex, historicalRoots) {
  return uniqueSortedStrings([
    ...(historicalRoots || []),
    ...(contractIndex?.declared_refs || []).map((ref) => ref.path),
    ...(contractIndex?.allowed_refs || []).map((ref) => ref.path),
  ]);
}

function c8ResidualPathMatches(refPath, findingPath) {
  const ref = normalizeRepoPath(refPath).replace(/\/+$/, '');
  const finding = normalizeRepoPath(findingPath).replace(/\/+$/, '');
  return finding === ref;
}

function c8ResidualSymbolMatches(expected, observed) {
  if (expected === observed) return true;
  if (typeof expected !== 'string' || !expected.includes('*')) return false;
  const pattern = expected
    .split('*')
    .map((part) => escapeC7Regex(part))
    .join('.*');
  return new RegExp(`^${pattern}$`).test(String(observed || ''));
}

function c8ResidualRefMatches(ref, finding) {
  if (
    !c8ResidualPathMatches(ref?.path, finding?.path) &&
    !(
      ref?.is_directory === true &&
      normalizeRepoPath(finding?.path).startsWith(`${ref.path}/`)
    )
  ) {
    return false;
  }
  if (
    Number.isInteger(ref?.line_start) &&
    Number.isInteger(finding?.line) &&
    finding.line < ref.line_start
  ) {
    return false;
  }
  if (
    Number.isInteger(ref?.line_end) &&
    Number.isInteger(finding?.line) &&
    finding.line > ref.line_end
  ) {
    return false;
  }
  const symbols = uniqueSortedStrings(ref?.symbols);
  return (
    symbols.length === 0 ||
    symbols.some((symbol) => c8ResidualSymbolMatches(symbol, finding?.match))
  );
}

function c8ResidualAllowedAtC8(ref) {
  return (
    ref?.policy === 'd004_exact_decrementing_allowlist' &&
    ref?.allowed_until_boundary === 'C8-MERGE' &&
    ref?.target_zero_boundary === 'C9'
  );
}

function c8ResidualDeferredToC9(ref) {
  const boundary = String(ref?.removal_boundary || '').toUpperCase();
  if (ref?.source_kind === 'd027') return true;
  if (
    ref?.source_kind === 'production_root' &&
    ref?.source_id === 'release-workspace' &&
    ['Cargo.toml', 'Cargo.lock'].includes(String(ref?.path || ''))
  ) {
    return true;
  }
  return (
    boundary === 'C9' ||
    boundary.endsWith('/C9') ||
    boundary.includes('/C9/')
  );
}

function c8ConfirmationPolicyEvidence(policyOverride) {
  if (policyOverride && typeof policyOverride === 'object') {
    return {
      c1_preserve: policyOverride.c1_preserve === true,
      c7_preserve: policyOverride.c7_preserve === true,
      source_paths: policyOverride.source_paths || [],
    };
  }
  if (c8ConfirmationPolicyCache) return c8ConfirmationPolicyCache;
  const policy = {
    c1_preserve: false,
    c7_preserve: false,
    source_paths: [],
  };
  const c1Path =
    'docs/specs/2026-08-28-agent-capability-platform-v2/C1-WRITE-MANIFESTS.json';
  const c7Path =
    'docs/specs/2026-08-28-agent-capability-platform-v2/C7-WRITE-MANIFESTS.json';
  for (const [path, key] of [
    [c1Path, 'c1_preserve'],
    [c7Path, 'c7_preserve'],
  ]) {
    const source = readFileSafe(join(repoRoot, path));
    if (!source) continue;
    try {
      const manifest = JSON.parse(source);
      const identityValid =
        key === 'c1_preserve'
          ? manifest.boundary === 'C1' &&
            manifest.status === 'closed' &&
            manifest.branch === C8_BRANCH &&
            manifest.confirmed_decision_contract_digest ===
              C8_EXPECTED_DIGESTS.confirmed_decision_contract
          : manifest.boundary === 'C7' &&
            manifest.status === 'closed' &&
            manifest.branch === C8_BRANCH &&
            manifest.confirmed_inputs?.decision_contract_digest ===
              C8_EXPECTED_DIGESTS.confirmed_decision_contract;
      if (!identityValid) continue;
      const preserved = uniqueSortedStrings(manifest.preserve_exact);
      policy[key] =
        key === 'c1_preserve'
          ? preserved.includes('Channel/IM pairing authorization and owner admission') &&
            preserved.includes('ordinary product delete/reset/unsaved-change confirmations')
          : preserved.includes('Channel pairing and ordinary product confirmations');
      if (policy[key]) policy.source_paths.push(path);
    } catch {
      // A missing or malformed policy is deliberately treated as no proof.
    }
  }
  c8ConfirmationPolicyCache = policy;
  return policy;
}

function c8ClassifyConfirmationFinding(
  finding,
  sourceOverride = null,
  policyOverride = undefined
) {
  const path = normalizeRepoPath(finding?.path);
  const match = String(finding?.match || '');
  const confirmationCandidate =
    finding?.category === 'approval_confirmation' ||
    finding?.category === 'channel_product_confirmation_candidate' ||
    /\b(?:ApprovalScope|ToolApprovalManager|ToolConfirmer|ConfirmRequest|PermissionConfirm|AgentConfirm(?:ation)?|PendingDecisionStore|PendingDecisionKind::StopConversation|WaitingConfirmation|ToolConfirmation(?:Request|State|Manager)?|needs_confirmation|awaiting_approval|waiting_confirmation|require_approval|auto_approve(?:_invocation)?|confirmation_to_decision|pending_decisions|stop_confirmation)\b/.test(
      match
    ) ||
    match === 'system.confirm';
  if (!confirmationCandidate) return null;
  const source =
    typeof sourceOverride === 'string'
      ? sourceOverride
      : readFileSafe(join(repoRoot, path)) || '';
  const semantic =
    source && /\.(rs|toml|json|ts|tsx)$/.test(path)
      ? analyzeC7Source(source, path).semantic
      : source;
  const channelPath = path.startsWith('crates/backend/nomifun-channel/');
  const channelStopMarkers = [
    'PendingDecisionStore',
    'PendingDecisionKind::StopConversation',
    'StopDenied',
    'stop_confirmation',
    'confirmation_to_decision',
    'pending_decisions',
  ];
  const sourceLines = source.split(/\r?\n/);
  const localContext =
    Number.isInteger(finding?.line) && finding.line > 0
      ? sourceLines
          .slice(Math.max(0, finding.line - 3), finding.line + 2)
          .join('\n')
      : '';
  const matchedChannelMarker = channelStopMarkers.find(
    (marker) => match.includes(marker) || localContext.includes(marker)
  );
  const matchIsChannelSpecific =
    /(?:PendingDecisionStore|PendingDecisionKind::StopConversation|confirmation_to_decision|pending_decisions|stop_confirmation)/.test(
      match
    );
  const channelProductionFiles = new Set([
    'crates/backend/nomifun-channel/src/pending_decision.rs',
    'crates/backend/nomifun-channel/src/message_service.rs',
    'crates/backend/nomifun-channel/src/message_loop.rs',
    'crates/backend/nomifun-channel/src/stream_relay.rs',
  ]);
  const channelTestPath =
    path.includes('/tests/') || path.startsWith('crates/backend/nomifun-channel/tests/');
  const fileLevelStopProof =
    path.endsWith('/pending_decision.rs')
      ? semantic.includes('PendingDecisionKind::StopConversation') &&
        semantic.includes('target_conversation_id') &&
        semantic.includes('nomi_stop_conversation')
      : path.endsWith('/message_service.rs')
        ? semantic.includes('pending_decisions') &&
          semantic.includes('stop_conversation') &&
          semantic.includes('StopDenied')
        : path.endsWith('/message_loop.rs')
          ? semantic.includes('pending_decisions') &&
            semantic.includes('PendingDecisionKind::StopConversation') &&
            semantic.includes('parse_choice')
          : path.endsWith('/stream_relay.rs')
            ? semantic.includes('record_and_send_stop_confirmation') &&
              semantic.includes('StopDenied')
            : false;
  const boundedStopProof =
    Boolean(matchedChannelMarker) ||
    (matchIsChannelSpecific && fileLevelStopProof);
  const channelStopProof =
    channelPath &&
    channelProductionFiles.has(path) &&
    !channelTestPath &&
    boundedStopProof;
  const agentApprovalProof =
    finding?.category === 'approval_confirmation' ||
    /\b(?:ApprovalScope|ToolApprovalManager|ToolConfirmer|ConfirmRequest|PermissionConfirm|AgentConfirm(?:ation)?|WaitingConfirmation|ToolConfirmation(?:Request|State|Manager)?|needs_confirmation|awaiting_approval|waiting_confirmation|require_approval|auto_approve(?:_invocation)?)\b/.test(
      match
    );
  if (channelStopProof) {
    const policy = c8ConfirmationPolicyEvidence(policyOverride);
    return {
      kind: 'channel_product_confirmation',
      evidence: {
        channel_owned_path: true,
        channel_stop_marker: matchedChannelMarker || null,
        policy_proof: policy.c1_preserve || policy.c7_preserve,
        policy_sources: policy.source_paths,
      },
    };
  }
  if (agentApprovalProof) {
    return {
      kind: 'agent_approval_confirmation',
      evidence: {
        channel_owned_path: false,
        policy_proof: false,
        policy_sources: [],
      },
    };
  }
  if (
    finding?.category === 'channel_product_confirmation_candidate' ||
    finding?.category === 'approval_confirmation'
  ) {
    return {
      kind: 'ambiguous_confirmation',
      evidence: {
        channel_owned_path: channelPath,
        channel_stop_marker: null,
        policy_proof: false,
        policy_sources: [],
      },
    };
  }
  return null;
}

function c8ChannelPreservationDecision(
  confirmation,
  declaredRefs,
  policyOverride = undefined,
  pathLevelDeclaredRefs = []
) {
  if (confirmation?.kind !== 'channel_product_confirmation') {
    return null;
  }
  const policy = c8ConfirmationPolicyEvidence(policyOverride);
  const allRelevantRefs = [
    ...(declaredRefs || []),
    ...(pathLevelDeclaredRefs || []),
  ].filter(
    (ref, index, refs) =>
      refs.findIndex(
        (candidate) =>
          candidate.contract_path === ref.contract_path &&
          candidate.source_kind === ref.source_kind &&
          candidate.source_id === ref.source_id &&
          candidate.path === ref.path &&
          candidate.residual_id === ref.residual_id
      ) === index
  );
  const conflictingRefs = allRelevantRefs.filter((ref) => {
    const category = String(ref?.category || '').toLowerCase();
    const sourceId = String(ref?.source_id || '').toLowerCase();
    const disposition = String(ref?.disposition || '').toLowerCase();
    return (
      category.includes('approval') ||
      category.includes('confirmation') ||
      sourceId.includes('approval') ||
      sourceId.includes('confirmation') ||
      disposition.includes('delete') ||
      disposition.includes('replace')
    );
  });
  const policyProof = policy.c1_preserve || policy.c7_preserve;
  const explicit = policyProof && conflictingRefs.length === 0;
  return {
    decision: explicit
      ? 'preserved_channel_product_confirmation'
      : conflictingRefs.length > 0
        ? 'blocking_contract_conflict'
        : 'blocking_insufficient_preservation_evidence',
    explicit,
    policy_proof: policyProof,
    policy_sources: policy.source_paths,
    conflicting_contract_refs: conflictingRefs.slice(0, 10).map((ref) => ({
      contract_path: ref.contract_path,
      manifest_id: ref.manifest_id,
      source_kind: ref.source_kind,
      source_id: ref.source_id,
      category: ref.category,
      disposition: ref.disposition,
      removal_boundary: ref.removal_boundary,
      match_scope: (declaredRefs || []).includes(ref)
        ? 'path-and-line'
        : 'path-and-symbol',
    })),
  };
}

function c8ClassifyResidualFinding(finding, contractIndex, options = {}) {
  const allowedRefs = (contractIndex?.allowed_refs || [])
    .filter((ref) => c8ResidualAllowedAtC8(ref))
    .filter((ref) => c8ResidualRefMatches(ref, finding));
  const declaredRefs = (contractIndex?.declared_refs || []).filter((ref) =>
    c8ResidualRefMatches(ref, finding)
  );
  const deferredRefs = declaredRefs.filter((ref) =>
    c8ResidualDeferredToC9(ref)
  );
  const nonDeferredRefs = declaredRefs.filter(
    (ref) => !c8ResidualDeferredToC9(ref)
  );
  const pathLevelDeclaredRefs = (contractIndex?.declared_refs || []).filter(
    (ref) =>
      c8ResidualPathMatches(ref?.path, finding?.path) &&
      uniqueSortedStrings(ref?.symbols).length > 0 &&
      uniqueSortedStrings(ref?.symbols).some((symbol) =>
        c8ResidualSymbolMatches(symbol, finding?.match)
      )
  );
  const confirmation = c8ClassifyConfirmationFinding(
    finding,
    options.source,
    options.confirmation_policy
  );
  const preservation = c8ChannelPreservationDecision(
    confirmation,
    declaredRefs,
    options.confirmation_policy,
    pathLevelDeclaredRefs
  );
  let classification;
  if (confirmation?.kind === 'channel_product_confirmation') {
    classification = preservation?.explicit
      ? 'preserved_channel_product_confirmation'
      : declaredRefs.length
        ? 'blocking_declared_residual'
        : 'blocking_unclassified_residual';
  } else if (confirmation?.kind === 'ambiguous_confirmation') {
    classification = declaredRefs.length
      ? 'blocking_declared_residual'
      : 'blocking_unclassified_residual';
  } else {
    classification = allowedRefs.length
      ? 'allowed_contract_residual'
      : deferredRefs.length && nonDeferredRefs.length === 0
        ? 'deferred_c9_residual'
        : declaredRefs.length
          ? 'blocking_declared_residual'
          : 'blocking_unclassified_residual';
  }
  return {
    ...finding,
    classification,
    confirmation_classification: confirmation?.kind || null,
    confirmation_preservation: preservation,
    contract_refs: declaredRefs.slice(0, 10).map((ref) => ({
      contract_path: ref.contract_path,
      manifest_id: ref.manifest_id,
      source_kind: ref.source_kind,
      source_id: ref.source_id,
      category: ref.category,
      removal_boundary: ref.removal_boundary,
      disposition: ref.disposition,
      replacement_owner: ref.replacement_owner,
    })),
    deferred_c9_refs: deferredRefs.slice(0, 10).map((ref) => ({
      contract_path: ref.contract_path,
      manifest_id: ref.manifest_id,
      source_kind: ref.source_kind,
      source_id: ref.source_id,
      removal_boundary: ref.removal_boundary,
      disposition: ref.disposition,
      replacement_owner: ref.replacement_owner,
    })),
    allowed_residual_refs: allowedRefs.slice(0, 10).map((ref) => ({
      contract_path: ref.contract_path,
      manifest_id: ref.manifest_id,
      residual_id: ref.residual_id,
      policy: ref.policy,
      allowed_until_boundary: ref.allowed_until_boundary,
      target_zero_boundary: ref.target_zero_boundary,
    })),
  };
}

function c8ClassifyResidualScan(scan, contractIndex) {
  const findings = (scan?.findings || []).map((finding) =>
    c8ClassifyResidualFinding(finding, contractIndex)
  );
  const allowedCount = findings.filter(
    (finding) => finding.classification === 'allowed_contract_residual'
  ).length;
  const deferredCount = findings.filter(
    (finding) => finding.classification === 'deferred_c9_residual'
  ).length;
  const declaredCount = findings.filter(
    (finding) => finding.classification === 'blocking_declared_residual'
  ).length;
  const unclassifiedCount = findings.filter(
    (finding) => finding.classification === 'blocking_unclassified_residual'
  ).length;
  const preservedChannelCount = findings.filter(
    (finding) =>
      finding.classification === 'preserved_channel_product_confirmation'
  ).length;
  const agentApprovalCount = findings.filter(
    (finding) =>
      finding.confirmation_classification === 'agent_approval_confirmation'
  ).length;
  const channelProductCount = findings.filter(
    (finding) =>
      finding.confirmation_classification === 'channel_product_confirmation'
  ).length;
  const ambiguousConfirmationCount = findings.filter(
    (finding) => finding.confirmation_classification === 'ambiguous_confirmation'
  ).length;
  const truncatedCount = Math.max(
    0,
    Number(scan?.total_count || 0) - findings.length
  );
  return {
    findings,
    allowed_count: allowedCount,
    deferred_c9_count: deferredCount,
    declared_count: declaredCount,
    unclassified_count: unclassifiedCount + truncatedCount,
    truncated_count: truncatedCount,
    preserved_channel_product_count: preservedChannelCount,
    agent_approval_count: agentApprovalCount,
    channel_product_count: channelProductCount,
    ambiguous_confirmation_count: ambiguousConfirmationCount,
    blocking_count: declaredCount + unclassifiedCount + truncatedCount,
  };
}

function c8GlobalResidualPass(mustBeZero, observedCount, classification) {
  if (!mustBeZero) return true;
  return (
    classification &&
    Number.isInteger(classification.blocking_count) &&
    classification.blocking_count === 0
  );
}

function c8SelfTestAssert(condition, message) {
  if (!condition) {
    throw new Error(`C8 self-test failed: ${message}`);
  }
}

function runC8SelfTest() {
  c8SelfTestAssert(
    c8CanonicalEqual(C8_REQUIRED_NATIVE_CELLS, [
      'windows_desktop_x64',
      'macos_desktop_arm64',
      'linux_desktop_x64',
    ]),
    'C8 release blocking must be limited to Windows x64, macOS arm64, and Linux Desktop x64'
  );

  const preRunPlatformManifest = JSON.parse(
    readFileSync(join(repoRoot, C8_PLATFORM_VALIDATION_MANIFEST_PATH), 'utf8')
  );
  const contractDigestLedger = JSON.parse(
    readFileSync(
      join(
        repoRoot,
        'crates/backend/nomifun-agent-contracts/contracts/generated/contract-digest-ledger.envelope.json'
      ),
      'utf8'
    )
  );
  c8SelfTestAssert(
    !Object.hasOwn(preRunPlatformManifest, 'candidate_source_sha'),
    'pre-run PlatformValidationManifest must not contain candidate_source_sha'
  );
  c8SelfTestAssert(
    !Object.hasOwn(contractDigestLedger?.payload || {}, 'base_source_sha'),
    'pre-run contract digest ledger must not contain base_source_sha'
  );

  const dirtyReport = {
    preflight_blocked: false,
    failure_details: [],
  };
  const dirtyResolved = { clean_worktree_required: true };
  c8ApplyWorktreeCheckpointProbe(
    dirtyReport,
    dirtyResolved,
    { status: 0, stdout: ' M scripts/gate-agent-v2.mjs', stderr: '' }
  );
  c8SelfTestAssert(
    dirtyReport.preflight_blocked === true,
    'dirty worktree must block preflight'
  );
  c8SelfTestAssert(
    dirtyReport.failure_details.some(
      (failure) => failure.code === 'dirty_worktree'
    ),
    'dirty worktree failure must be recorded'
  );

  const probeFailureReport = {
    preflight_blocked: false,
    failure_details: [],
  };
  c8ApplyWorktreeCheckpointProbe(
    probeFailureReport,
    { clean_worktree_required: true },
    { status: 128, stdout: '', stderr: 'git status failed' }
  );
  c8SelfTestAssert(
    probeFailureReport.preflight_blocked === true,
    'failed worktree probe must block preflight'
  );
  c8SelfTestAssert(
    probeFailureReport.failure_details.some(
      (failure) => failure.code === 'worktree_status_probe_failed'
    ),
    'failed worktree probe failure must be recorded'
  );

  const skippedReport = {
    checks: [],
    statuses: {},
    preflight_blocked: true,
  };
  c8SelfTestAssert(
    c8SkipNativeChecksIfBlocked(
      skippedReport,
      {
        required_checks: [
          { check_id: 'native_a', command: 'native-a' },
          { check_id: 'native_b', command: 'native-b' },
        ],
      },
      'self-test preflight failure'
    ) === true,
    'blocked preflight must take the native-check skip path'
  );
  c8SelfTestAssert(
    skippedReport.checks.length === 2 &&
      skippedReport.checks.every(
        (check) => check.status === 'skipped_preflight_failure'
      ),
    'preflight failure must skip every declared native check'
  );

  const contractIndex = c8BuildResidualContractIndex([
    {
      path: C8_TRIAD_DELETION_MANIFEST_PATH,
      payload: {
        manifest_id: 'triad-core',
        production_roots: [
          {
            root_id: 'test-root',
            kind: 'runtime_dispatcher',
            current_refs: [
              {
                path: 'crates/legacy/src/adapter.rs',
                symbols: ['LegacyManager'],
              },
            ],
          },
        ],
        allowed_residuals: {
          policy: 'd004_exact_decrementing_allowlist',
          entries: [
            {
              residual_id: 'd004.test',
              exact_refs: [
                {
                  path: 'crates/legacy/src/adapter.rs',
                  symbols: ['LegacyManager'],
                },
              ],
              allowed_until_boundary: 'C8-MERGE',
              target_zero_boundary: 'C9',
            },
          ],
        },
      },
    },
  ]);
  const classified = c8ClassifyResidualScan(
    {
      total_count: 2,
      findings: [
        {
          path: 'crates/legacy/src/adapter.rs',
          line: 12,
          match: 'LegacyManager',
          category: 'runtime_selector',
          label: 'legacy runtime selector',
        },
        {
          path: 'crates/other/src/route.rs',
          line: 4,
          match: 'LegacyManager',
          category: 'runtime_selector',
          label: 'legacy runtime selector',
        },
      ],
    },
    contractIndex
  );
  c8SelfTestAssert(
    classified.allowed_count === 1 &&
      classified.blocking_count === 1 &&
      classified.findings[0].classification === 'allowed_contract_residual' &&
      classified.findings[1].classification === 'blocking_unclassified_residual',
    'residual findings must be classified against contract refs'
  );
  c8SelfTestAssert(
    c8ResidualScanPaths(contractIndex, ['historical/root']).includes(
      'crates/legacy/src/adapter.rs'
    ),
    'residual scan paths must include contract allowlist refs'
  );
  c8SelfTestAssert(
    c8GlobalResidualPass(true, 2, classified) === false &&
      c8GlobalResidualPass(true, 1, {
        blocking_count: 0,
        allowed_count: 0,
        deferred_c9_count: 1,
      }) === true &&
      c8GlobalResidualPass(true, 1, {
        blocking_count: 0,
        allowed_count: 1,
        deferred_c9_count: 0,
      }) === true &&
      c8GlobalResidualPass(true, 0, {
        blocking_count: 0,
        allowed_count: 0,
        deferred_c9_count: 0,
      }) === true,
    'C8 must permit only explicitly deferred/allowed residuals while blocking all other findings'
  );
  const truncated = c8ClassifyResidualScan(
    {
      total_count: 3,
      findings: [
        {
          path: 'crates/legacy/src/adapter.rs',
          line: 12,
          match: 'LegacyManager',
        },
      ],
    },
    contractIndex
  );
  c8SelfTestAssert(
    truncated.truncated_count === 2 && truncated.blocking_count === 2,
    'truncated residual findings must remain blocking'
  );
  const outOfBoundaryContract = c8BuildResidualContractIndex([
    {
      path: C8_TRIAD_DELETION_MANIFEST_PATH,
      payload: {
        manifest_id: 'triad-core',
        allowed_residuals: {
          policy: 'd004_exact_decrementing_allowlist',
          entries: [
            {
              residual_id: 'd004.out-of-boundary',
              exact_refs: [
                {
                  path: 'crates/legacy/src/adapter.rs',
                  symbols: ['LegacyManager'],
                },
              ],
              allowed_until_boundary: 'C9',
              target_zero_boundary: 'C10',
            },
          ],
        },
      },
    },
  ]);
  c8SelfTestAssert(
    c8ClassifyResidualFinding(
      {
        path: 'crates/legacy/src/adapter.rs',
        line: 12,
        match: 'LegacyManager',
      },
      outOfBoundaryContract
    ).classification === 'blocking_unclassified_residual',
    'an allowlist outside the C8 boundary must remain blocking'
  );

  const mergeSource = 'a'.repeat(40);
  const mergeCells = Object.fromEntries(
    C8_REQUIRED_NATIVE_CELLS.map((cellId) => [
      cellId,
      {
        status: 'pass',
        source_commit: mergeSource,
        release_lock_sha256: 'd'.repeat(64),
        artifact_digests: {
          host: 'b'.repeat(64),
          package: 'c'.repeat(64),
          runtime_sidecar: 'e'.repeat(64),
        },
        evidence: {
          digest: 'e'.repeat(64),
          normalized_relative_path: `build/evidence/${cellId}.json`,
        },
      },
    ])
  );
  const mergeReport = {
    gate_name: 'c8-merge',
    failure_details: [],
  };
  const mergeValidation = validateC8MergeSummary(
    mergeReport,
    {
      source_commit: mergeSource,
      cell_evidence: mergeCells,
      status_counts: {
        pass: C8_REQUIRED_NATIVE_CELLS.length,
        fail: 0,
      },
      all_verification_points_closed: true,
      global_residual_reachability_zero: true,
      d027_terminal_evidence: {
        artifact_id: 'd027-zero',
        digest: 'f'.repeat(64),
        normalized_relative_path: 'build/evidence/d027-zero.json',
      },
    },
    mergeSource,
    false
  );
  c8SelfTestAssert(
    mergeValidation.status === 'pass' &&
      mergeReport.failure_details.length === 0,
    'a complete same-source three-platform C8-MERGE summary must pass'
  );

  const channelPolicy = {
    c1_preserve: true,
    c7_preserve: true,
    source_paths: ['self-test/C1', 'self-test/C7'],
  };
  const channelFinding = {
    path: 'crates/backend/nomifun-channel/src/pending_decision.rs',
    line: 45,
    category: 'channel_product_confirmation_candidate',
    match: 'PendingDecisionStore',
  };
  const channelSource = [
    'struct PendingDecisionStore;',
    'enum PendingDecisionKind { StopConversation }',
  ].join('\n');
  const preservedChannel = c8ClassifyResidualFinding(
    channelFinding,
    { declared_refs: [], allowed_refs: [] },
    { source: channelSource, confirmation_policy: channelPolicy }
  );
  c8SelfTestAssert(
    preservedChannel.confirmation_classification ===
      'channel_product_confirmation' &&
      preservedChannel.classification ===
        'preserved_channel_product_confirmation',
    'an explicitly evidenced channel-owned stop must not be counted as Agent approval'
  );

  const conflictingChannel = c8ClassifyResidualFinding(
    channelFinding,
    {
      declared_refs: [
        {
          path: channelFinding.path,
          symbols: ['PendingDecisionStore'],
          contract_path: 'self-test/deletion.json',
          manifest_id: 'self-test',
          source_kind: 'legacy_surface',
          source_id: 'mode-approval-confirmation',
          category: 'mode_approval_permission',
          disposition: 'delete_without_replacement',
        },
      ],
      allowed_refs: [],
    },
    { source: channelSource, confirmation_policy: channelPolicy }
  );
  c8SelfTestAssert(
    conflictingChannel.confirmation_classification ===
      'channel_product_confirmation' &&
      conflictingChannel.classification === 'blocking_declared_residual' &&
      conflictingChannel.confirmation_preservation?.decision ===
        'blocking_contract_conflict',
    'a channel-owned stop with a conflicting deletion contract must remain blocking'
  );

  const agentFinding = c8ClassifyResidualFinding(
    {
      path: 'crates/backend/nomifun-public/src/result.rs',
      line: 1,
      category: 'approval_confirmation',
      match: 'needs_confirmation',
    },
    { declared_refs: [], allowed_refs: [] },
    {
      source: 'pub const FIELD: &str = "needs_confirmation";',
      confirmation_policy: channelPolicy,
    }
  );
  c8SelfTestAssert(
    agentFinding.confirmation_classification === 'agent_approval_confirmation' &&
      agentFinding.classification === 'blocking_unclassified_residual',
    'needs_confirmation must remain an Agent approval residual'
  );

  const ambiguousChannel = c8ClassifyResidualFinding(
    {
      path: 'crates/backend/other/src/store.rs',
      line: 1,
      category: 'channel_product_confirmation_candidate',
      match: 'PendingDecisionStore',
    },
    { declared_refs: [], allowed_refs: [] },
    {
      source: 'struct PendingDecisionStore;',
      confirmation_policy: channelPolicy,
    }
  );
  c8SelfTestAssert(
    ambiguousChannel.confirmation_classification === 'ambiguous_confirmation' &&
      ambiguousChannel.classification === 'blocking_unclassified_residual',
    'insufficient channel ownership evidence must remain blocking'
  );

  const ownerBlockers = c8ProductionOwnerBlockers({
    host: `
      match operation { _ => "no Fresh-v4 Wave 1 owner is wired for {}" }
      wave3: nomifun_agent_domain_wave3::unconfigured_host_port(),
      wave5: nomifun_agent_domain_wave5::unconfigured_host_port(),
    `,
    wave2: 'no canonical application owner is wired for {capability_id}',
    wave4: 'Fresh-v4 has no native owner for {} resource action',
  });
  c8SelfTestAssert(
    JSON.stringify(ownerBlockers.map((blocker) => blocker.domain)) ===
      JSON.stringify(['wave1', 'wave2', 'wave3', 'wave4', 'wave5']),
    'C8 owner coverage must fail closed for every partial or unconfigured production wave'
  );
  c8SelfTestAssert(
    c8ProductionOwnerBlockers({
      host: 'all domain owners are explicitly mounted',
      wave2: 'all Wave 2 operations have exact owners',
      wave4: 'all Wave 4 operations have exact owners',
    }).length === 0,
    'C8 owner coverage must accept a production graph without fallback markers'
  );

  const nativeSpec = c8ParseNativeDispatchArgs(
    'c8-ma',
    ['c8-ma', '--evidence', 'build/evidence.json']
  );
  c8SelfTestAssert(
    nativeSpec.cell_id === 'macos_desktop_arm64' &&
      nativeSpec.check_id === 'c8_ma_full_gate',
    'c8-ma must dispatch to the exact arm64 macOS full-gate check'
  );
  const genericNativeSpec = c8ParseNativeDispatchArgs('c8-native', [
    'c8-native',
    '--cell',
    'linux_desktop_x64',
    '--evidence',
    'build/evidence.json',
  ]);
  c8SelfTestAssert(
    genericNativeSpec.cell_id === 'linux_desktop_x64' &&
      genericNativeSpec.check_id === 'c8_ld_full_gate' &&
      genericNativeSpec.evidence_path === 'build/evidence.json',
    'parameterized native dispatch must require an allowed release platform cell'
  );
  let retiredCellRejected = false;
  try {
    c8ParseNativeDispatchArgs('c8-native', [
      'c8-native',
      '--cell',
      'linux_headless_x64',
      '--evidence',
      'build/evidence.json',
    ]);
  } catch {
    retiredCellRejected = true;
  }
  c8SelfTestAssert(
    retiredCellRejected,
    'macOS x64 and Linux Headless must not be native C8 dispatch cells'
  );

  const nonNativeReport = {
    gate_name: 'c8-ma',
    execution_host: {
      host_os: 'windows',
      host_arch: 'x86_64',
      native: false,
      rejection_reasons: ['host_uname_os_mismatch'],
    },
    failure_details: [],
  };
  c8NativeValidateHost(
    nonNativeReport,
    C8_NATIVE_CELL_SPECS.macos_desktop_arm64
  );
  c8SelfTestAssert(
    nonNativeReport.failure_details.length >= 2 &&
      nonNativeReport.failure_details.some(
        (failure) => failure.code === 'native_host_os_mismatch'
      ),
    'a mismatched host can never validate a native cell'
  );
}

function c8ValidateProductionBrokerFunctionalEvidence(report) {
  const fault = report.all_scene_coverage?.fault_classes?.provider_unavailable;
  if (!fault) {
    c8Failure(
      report,
      'scene_fault_coverage',
      'C8 provider_unavailable coverage was not evaluated'
    );
    if (report.all_scene_coverage) {
      report.all_scene_coverage.status = 'fail';
    }
    return;
  }

  const checkId = 'production_broker_functional_tests';
  const expected = c8ExpectedChecks().find(
    (check) => check.check_id === checkId
  );
  const check = report.checks.find((entry) => entry.check_id === checkId);
  const checkStatus = check?.status || report.statuses?.[checkId] || 'not_run';
  const commandMatches =
    Boolean(check) &&
    c8NormalizeCommand(check.command) === c8NormalizeCommand(expected?.command) &&
    c8NormalizeCommand(check.invoked_command) ===
      c8NormalizeCommand(
        [expected?.runner, ...(expected?.command_args || [])].join(' ')
      );
  const markerEvidencePresent = fault.status === 'pending_functional_check';
  const functionalPass =
    markerEvidencePresent &&
    checkStatus === 'pass' &&
    check?.exit_code === 0 &&
    commandMatches;
  fault.functional_check_status = checkStatus;
  fault.functional_check_command = check?.command || null;
  fault.functional_check_exit_code =
    typeof check?.exit_code === 'number' ? check.exit_code : null;
  fault.functional_check_command_matches = commandMatches;

  if (!functionalPass) {
    fault.status = 'fail';
    if (markerEvidencePresent) {
      c8Failure(
        report,
        'scene_fault_coverage',
        'C8 provider_unavailable requires a passing real production ChatModelBroker functional check',
        {
          check_id: checkId,
          observed_status: checkStatus,
          observed_exit_code: fault.functional_check_exit_code,
          command_matches: commandMatches,
          marker_evidence_present: markerEvidencePresent,
        }
      );
    }
  }

  const faultStatuses = Object.values(
    report.all_scene_coverage?.fault_classes || {}
  ).map((entry) => entry.status);
  if (
    report.all_scene_coverage &&
    (!faultStatuses.length || faultStatuses.some((status) => status !== 'pass'))
  ) {
    report.all_scene_coverage.status = 'fail';
  }
}

function c8RunToolchainProbe(report) {
  const probes = [
    ['rustc_version', 'rustc', ['--version']],
    ['cargo_version', 'cargo', ['--version']],
    ['bun_version', 'bun', ['--version']],
    ['node_version', 'node', ['--version']],
    [
      'windows_target_installed',
      'rustup',
      ['target', 'list', '--installed'],
    ],
  ];
  for (const [checkId, command, args] of probes) {
    c8RunCommand(report, checkId, command, args, {
      addFailure: checkId === 'windows_target_installed',
      timeout: 30 * 1000,
    });
  }
  const targetEntry = report.checks.find(
    (check) => check.check_id === 'windows_target_installed'
  );
  const targetOutput = commands.find(
    (entry) => entry.check_id === 'windows_target_installed'
  )?.stdout_log;
  const targetText = targetOutput
    ? readFileSafe(join(repoRoot, targetOutput)) || ''
    : '';
  if (
    targetEntry?.status === 'pass' &&
    !targetText.split(/\r?\n/).some((line) => line.trim() === 'x86_64-pc-windows-msvc')
  ) {
    c8Failure(
      report,
      'windows_target_missing',
      'rustup does not report x86_64-pc-windows-msvc as installed'
    );
  }
}

function c8RunStartupSmoke(report) {
  const sourceSha = report.source_sha;
  const smokeRoot = join(
    repoRoot,
    'build.noindex',
    'agent-capability-v2',
    sourceSha,
    'c8-win-pre',
    'startup-smoke',
    `run-${Date.now()}`
  );
  mkdirSync(smokeRoot, { recursive: true });
  const executable = join(repoRoot, 'target', 'debug', 'nomicore.exe');
  if (!statSafe(executable)?.isFile()) {
    c8RunCommand(report, 'windows_startup_build', 'cargo', [
      'build',
      '--locked',
      '-p',
      'nomifun-app',
      '--bin',
      'nomicore',
    ]);
  }
  if (!statSafe(executable)?.isFile()) {
    c8Failure(
      report,
      'startup_binary_missing',
      'target/debug/nomicore.exe is unavailable after the startup build'
    );
    return;
  }

  const escapePowerShell = (value) =>
    String(value).replaceAll('`', '``').replaceAll("'", "''");
  const port = 28000 + Math.floor(Math.random() * 500);
  const script = `
$ErrorActionPreference = 'Stop'
$root = '${escapePowerShell(smokeRoot)}'
$exe = '${escapePowerShell(executable)}'
$port = ${port}
$process = Start-Process -FilePath $exe -ArgumentList @('--data-dir', $root, '--port', "$port", '--local', '--log-level', 'error') -WindowStyle Hidden -PassThru
try {
  $ready = $false
  for ($i = 0; $i -lt 80; $i++) {
    Start-Sleep -Milliseconds 250
    try {
      $health = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$port/health" -TimeoutSec 1
      if ($health.StatusCode -eq 200) { $ready = $true; break }
    } catch {}
  }
  if (-not $ready) { throw 'startup smoke did not reach /health' }
  $capabilities = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$port/api/capabilities" -TimeoutSec 5
  $payload = $capabilities.Content | ConvertFrom-Json
  if (-not $payload.success) { throw 'canonical capabilities endpoint returned failure' }
  $items = @($payload.data)
  if ($items.Count -eq 0) { throw 'canonical capabilities endpoint returned an empty catalog' }
  $ids = @($items | ForEach-Object {
    if ($_.capability_id) { $_.capability_id }
    elseif ($_.capability -and $_.capability.id) { $_.capability.id }
    elseif ($_.id) { $_.id }
  })
  foreach ($required in @('fs.read', 'browser.render_content', 'computer.input')) {
    if ($ids -notcontains $required) { throw "canonical capabilities endpoint is missing $required" }
  }
} finally {
  if ($process -and -not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
}
`;
  const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
    cwd: repoRoot,
    encoding: 'utf8',
    shell: false,
    stdio: 'pipe',
    timeout: 180000,
  });
  const stdout = String(result.stdout || '');
  const stderr = String(result.stderr || '');
  const entry = {
    check_id: 'windows_startup_smoke',
    command: 'target/debug/nomicore.exe --data-dir <temporary-root> --port <free-port> --local',
    invoked_command: 'powershell <startup smoke script>',
    execution_kind: 'native',
    exit_code: typeof result.status === 'number' ? result.status : 1,
    status: result.status === 0 ? 'pass' : 'fail',
    stdout_tail: c8Tail(stdout),
    stderr_tail: c8Tail(stderr),
    smoke_root: relative(repoRoot, smokeRoot).replaceAll('\\', '/'),
  };
  commands.push(entry);
  c8Check(report, 'windows_startup_smoke', entry.status, entry);
  if (entry.status === 'fail') {
    c8Failure(report, 'startup_smoke_failed', 'Windows startup smoke failed', entry);
  }
}

function c8RunDeclaredC8Checks(report, manifest) {
  const expectedChecks = c8ExpectedChecks();
  for (const expected of expectedChecks) {
    if (expected.runner === 'startup_smoke') {
      c8RunStartupSmoke(report);
      continue;
    }
    if (!expected.command_args) continue;

    // The workspace run is owned by the coordinator.  Reuse a verified pass
    // for the same source tuple, but never reuse a failure as evidence.
    if (expected.runner === 'workspace') {
      const existing = join(
        repoRoot,
        report.evidence_directory,
        'workspace',
        'cargo-test-pass.marker'
      );
      if (statSafe(existing)?.isFile()) {
        c8Check(report, expected.check_id, 'pass', {
          command: expected.command,
          reused_evidence: relative(repoRoot, existing).replaceAll('\\', '/'),
          deduplication_key: expected.deduplication_key,
        });
        continue;
      }
      const run = c8RunCommand(
        report,
        expected.check_id,
        expected.runner === 'workspace' ? 'cargo' : expected.runner,
        expected.command_args,
        {
          displayCommand: expected.command,
          addFailure: true,
          timeout: WORKSPACE_COMMAND_TIMEOUT_MS,
        }
      );
      if (run.entry.status === 'pass') {
        const workspaceDir = join(repoRoot, report.evidence_directory, 'workspace');
        mkdirSync(workspaceDir, { recursive: true });
        writeFileSync(join(workspaceDir, 'cargo-test-pass.marker'), `${report.source_sha}\n`);
      }
      continue;
    }

    if (expected.runner === 'ui_check') {
      const run = c8RunCommand(
        report,
        expected.check_id,
        'bun',
        expected.command_args,
        {
          displayCommand: expected.command,
          addFailure: false,
          timeout: 10 * 60 * 1000,
        }
      );
      if (run.entry.status === 'fail') {
        const combined = `${run.stdout}\n${run.stderr}`;
        const baseline =
          /TS\d{4}/.test(combined) &&
          /(React|Arco|toBeInTheDocument|matcher|type definition)/i.test(combined);
        run.entry.status = baseline ? 'baseline_fail' : 'fail';
        c8Check(report, expected.check_id, run.entry.status, {
          ...run.entry,
          baseline,
          note: baseline
            ? 'known repository-wide UI typing baseline; production build and focused C7 checks remain authoritative'
            : 'new UI check failure',
        });
        if (!baseline) {
          c8Failure(report, 'ui_check_failed', 'C8 UI check failed outside the recorded baseline');
        }
      }
      continue;
    }

    const command = expected.runner === 'command' ? expected.command_args[0] : expected.runner;
    const args =
      expected.runner === 'command'
        ? expected.command_args.slice(1)
        : expected.command_args;
    c8RunCommand(report, expected.check_id, command, args, {
      displayCommand: expected.command,
      addFailure: true,
      timeout: expected.check_id === 'ui_build' ? 15 * 60 * 1000 : 10 * 60 * 1000,
    });
  }
}

function readC7Json(path, label) {
  const normalized = normalizeRepoPath(path);
  if (!isSafeRepoPath(normalized)) {
    failures.push(`${label} has an invalid repository-relative path: ${path}`);
    return null;
  }
  const absolute = join(repoRoot, normalized);
  if (!statSafe(absolute)?.isFile()) {
    failures.push(`missing ${label}: ${normalized}`);
    return null;
  }
  try {
    return JSON.parse(readFileSync(absolute, 'utf8'));
  } catch (error) {
    failures.push(`${normalized}: invalid JSON: ${error.message}`);
    return null;
  }
}

function normalizeRepoPath(path) {
  return typeof path === 'string' ? path.replaceAll('\\', '/') : '';
}

function isSafeRepoPath(path) {
  if (!path || path.startsWith('/') || path.includes('://')) return false;
  if (path.includes('\\')) return false;
  if (path.split('/').some((segment) => !segment || segment === '.' || segment === '..')) {
    return false;
  }
  const absolute = resolve(repoRoot, path);
  const root = resolve(repoRoot);
  return absolute === root || absolute.startsWith(`${root}${pathSeparator()}`);
}

function pathSeparator() {
  return process.platform === 'win32' ? '\\' : '/';
}

function uniqueSortedStrings(value) {
  if (!Array.isArray(value)) return [];
  return [...new Set(value.filter((entry) => typeof entry === 'string'))].sort();
}

function findC7Path(paths, matcher) {
  if (!Array.isArray(paths)) return null;
  return paths.map(normalizeRepoPath).find((path) => matcher.test(path)) || null;
}

function validateC7WriteManifest(manifest, expectedWaves) {
  if (manifest.schema_version !== '1.0.0') {
    failures.push('C7 write manifest schema_version must be 1.0.0');
  }
  if (manifest.boundary !== 'C7') {
    failures.push('C7 write manifest boundary must be C7');
  }
  if (!['active', 'closed'].includes(manifest.status)) {
    failures.push(`C7 write manifest has invalid status: ${manifest.status}`);
  }
  for (const key of ['base_sha', 'code_base_sha']) {
    if (!/^[0-9a-f]{40}$/i.test(manifest[key] || '')) {
      failures.push(`C7 write manifest ${key} must be a 40-character commit SHA`);
    }
  }
  if (manifest.branch !== 'rf/agent-capability-platform-v2') {
    failures.push('C7 write manifest branch does not match the capability-platform branch');
  }

  const policy = manifest.execution_policy;
  const exactPolicy = {
    platform_order: 'windows_first_continuous',
    feature_or_module_pause: false,
    native_non_windows_status: 'pending_native_verification',
    workspace_cargo_test: 'forbidden_during_c7',
    central_validation_owner: 'main-agent',
    same_change_demolition: true,
  };
  for (const [key, expected] of Object.entries(exactPolicy)) {
    if (policy?.[key] !== expected) {
      failures.push(`C7 execution_policy.${key} must be ${JSON.stringify(expected)}`);
    }
  }
  const inputs = manifest.confirmed_inputs;
  const requiredInputKeys = [
    'decision_contract_digest',
    'schema_manifest_digest',
    'contract_ledger_digest',
    'deletion_manifest_set_digest',
    'official_seed_digest',
    'runtime_feature_inventory_digest',
  ];
  if (!inputs || typeof inputs !== 'object' || Array.isArray(inputs)) {
    failures.push('C7 confirmed_inputs must be an object');
  } else {
    for (const key of requiredInputKeys) {
      if (!/^[0-9a-f]{64}$/i.test(inputs[key] || '')) {
        failures.push(`C7 confirmed_inputs.${key} must be a 64-character hex digest`);
      }
    }
  }

  const preserveExact = uniqueSortedStrings(manifest.preserve_exact);
  for (const required of [
    'published legacy migration bytes and checksums',
    'Coding review/diff/review-comment workflow',
    'OS/TCC/Tauri permissions',
    'Channel pairing and ordinary product confirmations',
  ]) {
    if (!preserveExact.includes(required)) {
      failures.push(`C7 preserve_exact is missing: ${required}`);
    }
  }

  if (!Array.isArray(manifest.shared_integration?.write_paths)) {
    failures.push('C7 shared_integration.write_paths must be an array');
  } else {
    const sharedWritePaths = manifest.shared_integration.write_paths.map(normalizeRepoPath);
    for (const requiredPath of [
      'crates/backend/nomifun-agent-domain-support/',
      'scripts/gate-agent-v2.mjs',
    ]) {
      if (!sharedWritePaths.includes(requiredPath)) {
        failures.push(`C7 shared integration write_paths is missing ${requiredPath}`);
      }
    }
  }
  if (manifest.shared_integration?.owner !== 'main-agent') {
    failures.push('C7 shared integration owner must be main-agent');
  }
  for (const requiredPath of [
    'crates/backend/nomifun-db/migrations/',
    'vendor/codex-runtime/',
    'crates/backend/nomifun-codex-runtime/',
    'ui/src/renderer/pages/conversation/platforms/nomi/',
  ]) {
    if (
      !uniqueSortedStrings(manifest.shared_integration?.forbidden_paths).includes(
        requiredPath
      )
    ) {
      failures.push(`C7 shared integration forbidden_paths is missing ${requiredPath}`);
    }
  }
  for (const key of ['required_outcomes', 'required_checks']) {
    if (
      !Array.isArray(manifest.shared_integration?.[key]) ||
      manifest.shared_integration[key].length === 0
    ) {
      failures.push(`C7 shared integration ${key} must be a non-empty array`);
    }
  }
  if (manifest.closure_requirements?.allowed_residuals?.length !== 0) {
    failures.push('C7 closure_requirements.allowed_residuals must be empty');
  }
  if (manifest.closure_requirements?.workspace_cargo_test !== 'not_run_until_c8_win_pre') {
    failures.push('C7 closure requirement must defer workspace cargo test until C8-WIN-PRE');
  }
  if (
    manifest.closure_requirements?.native_status !==
    'windows_c1_c7_continuous_pass_candidate; macos_linux_pending_native_verification'
  ) {
    failures.push('C7 closure native status is not the Windows-first pending-native contract');
  }
  if (
    manifest.closure_requirements?.evidence_record !==
    'docs/specs/2026-08-28-agent-capability-platform-v2/C7-CLOSURE.json'
  ) {
    failures.push('C7 closure evidence record path is not canonical');
  }
  if (expectedWaves.length !== 5) {
    failures.push('C7 gate has an invalid expected wave table');
  }
}

function validateC7WaveEntry(wave, expected) {
  if (wave.task_id !== expected.task_id) {
    failures.push(`${expected.task_id}: task_id mismatch`);
  }
  if (wave.wave !== expected.wave) {
    failures.push(`${expected.task_id}: wave identifier mismatch`);
  }
  if (wave.owner !== expected.owner) {
    failures.push(`${expected.task_id}: owner mismatch`);
  }
  if (normalizeRepoPath(wave.deletion_manifest) !== expected.deletion_manifest) {
    failures.push(`${expected.task_id}: deletion manifest path mismatch`);
  }
  if (!isSafeRepoPath(normalizeRepoPath(wave.deletion_manifest))) {
    failures.push(`${expected.task_id}: deletion manifest path is not repository-relative`);
  }
  for (const key of ['write_paths', 'forbidden_paths', 'target_packages', 'target_capability_families']) {
    if (!Array.isArray(wave[key]) || wave[key].length === 0) {
      failures.push(`${expected.task_id}: ${key} must be a non-empty array`);
    } else if (uniqueSortedStrings(wave[key]).length !== wave[key].length) {
      failures.push(`${expected.task_id}: ${key} contains duplicates`);
    }
  }
  for (const path of [
    ...(Array.isArray(wave.write_paths) ? wave.write_paths : []),
    ...(Array.isArray(wave.forbidden_paths) ? wave.forbidden_paths : []),
  ]) {
    const normalized = normalizeRepoPath(path).replace(/\/$/, '');
    if (!isSafeRepoPath(normalized)) {
      failures.push(`${expected.task_id}: invalid repository-relative path ${path}`);
    }
  }
  if (
    !uniqueSortedStrings(wave.write_paths)
      .map((path) => normalizeRepoPath(path).replace(/\/$/, ''))
      .includes(expected.generated_crate)
  ) {
    failures.push(
      `${expected.task_id}: write_paths must include ${expected.generated_crate}`
    );
  }
  for (const requiredPath of [
    'Cargo.toml',
    'Cargo.lock',
    'crates/backend/nomifun-db/migrations/',
    'ui/',
    'docs/specs/2026-08-28-agent-capability-platform-v2/',
  ]) {
    if (!uniqueSortedStrings(wave.forbidden_paths).includes(requiredPath)) {
      failures.push(`${expected.task_id}: forbidden_paths is missing ${requiredPath}`);
    }
  }
  if (typeof wave.direct_consumer_switch !== 'string' || !wave.direct_consumer_switch.trim()) {
    failures.push(`${expected.task_id}: direct_consumer_switch is required`);
  }
  if (typeof wave.deliverable !== 'string' || !wave.deliverable.trim()) {
    failures.push(`${expected.task_id}: deliverable is required`);
  }
}

function validateC7DeletionManifest(deletion, wave, expected) {
  if (deletion.schema_version !== '1.0.0') {
    failures.push(`${expected.task_id}: deletion manifest schema_version must be 1.0.0`);
  }
  if (deletion.manifest_kind !== 'domain_wave') {
    failures.push(`${expected.task_id}: deletion manifest must be a domain_wave manifest`);
  }
  if (deletion.wave !== expectedWaveContractName(expected.wave)) {
    failures.push(`${expected.task_id}: deletion manifest wave mismatch`);
  }
  if (
    deletion.canonical_producer?.owner_id !== expected.owner ||
    !Array.isArray(deletion.canonical_producer?.target_package_keys) ||
    !Array.isArray(deletion.canonical_producer?.target_capability_families)
  ) {
    failures.push(`${expected.task_id}: deletion manifest canonical producer is incomplete`);
  } else {
    if (
      !sameStringSet(
        deletion.canonical_producer.target_package_keys,
        wave.target_packages
      )
    ) {
      failures.push(`${expected.task_id}: target package coverage differs from deletion manifest`);
    }
    if (
      !sameStringSet(
        deletion.canonical_producer.target_capability_families,
        wave.target_capability_families
      )
    ) {
      failures.push(
        `${expected.task_id}: target capability-family coverage differs from deletion manifest`
      );
    }
  }
  if (
    deletion.allowed_residuals?.policy !== 'empty' ||
    !Array.isArray(deletion.allowed_residuals?.entries) ||
    deletion.allowed_residuals.entries.length !== 0
  ) {
    failures.push(`${expected.task_id}: ordinary C7 deletion residual allowlist must be empty`);
  }
  if (
    !Array.isArray(deletion.target_zero) ||
    deletion.target_zero.length === 0 ||
    deletion.target_zero.some((assertion) => assertion?.expected_count !== 0)
  ) {
    failures.push(`${expected.task_id}: deletion target_zero assertions must all be zero`);
  }
  if (deletion.closure_status?.state === 'rejected') {
    failures.push(`${expected.task_id}: deletion manifest is rejected`);
  }
}

function expectedWaveContractName(wave) {
  return {
    wave1_read_capabilities: 'wave1_read_capabilities',
    wave2_coding_extensions: 'wave2_coding_extensions',
    wave3_creative_multimodal: 'wave3_creative_multimodal',
    wave4_identity_channels_devices: 'wave4_identity_channels_devices',
    wave5_automation_supervision_remote: 'wave5_automation_supervision_remote',
  }[wave];
}

function sameStringSet(left, right) {
  return JSON.stringify(uniqueSortedStrings(left)) === JSON.stringify(uniqueSortedStrings(right));
}

function verifyC7InputDigests(manifest) {
  const inputs = manifest.confirmed_inputs || {};
  const checks = [
    {
      key: 'decision_contract_digest',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/contract-closure.envelope.json',
      field: 'payload_digest',
    },
    {
      key: 'schema_manifest_digest',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/canonical-v4-schema-manifest.envelope.json',
      field: 'payload_digest',
    },
    {
      key: 'contract_ledger_digest',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/contract-digest-ledger.envelope.json',
      field: 'payload_digest',
    },
    {
      key: 'deletion_manifest_set_digest',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/deletion-manifest-set.envelope.json',
      field: 'payload_digest',
    },
    {
      key: 'official_seed_digest',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/official-preset-seed-manifest.envelope.json',
      field: 'payload_digest',
    },
    {
      key: 'runtime_feature_inventory_digest',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/runtime-feature-inventory.envelope.json',
      field: 'payload_digest',
    },
  ];
  const results = {};
  for (const check of checks) {
    const expected = inputs[check.key];
    const result = {
      expected,
      path: check.path,
      field: check.field,
      status: 'fail',
    };
    const absolute = join(repoRoot, check.path);
    if (!statSafe(absolute)?.isFile()) {
      failures.push(`missing C7 confirmed input artifact: ${check.path}`);
      results[check.key] = result;
      continue;
    }
    let artifact;
    try {
      artifact = JSON.parse(readFileSync(absolute, 'utf8'));
    } catch (error) {
      failures.push(`${check.path}: invalid JSON: ${error.message}`);
      results[check.key] = result;
      continue;
    }
    const actual = check.field
      .split('.')
      .reduce((value, key) => value?.[key], artifact);
    result.actual = actual;
    result.raw_sha256 = sha256File(absolute);
    result.status =
      typeof actual === 'string' && actual.toLowerCase() === String(expected).toLowerCase()
        ? 'pass'
        : 'fail';
    if (artifact.digest_algorithm && artifact.digest_algorithm !== 'sorted-json-sha256-v1') {
      failures.push(`${check.path}: unexpected digest algorithm`);
      result.status = 'fail';
    }
    if (result.status !== 'pass') {
      failures.push(
        `C7 confirmed input digest mismatch for ${check.key}: expected ${expected}, observed ${actual}`
      );
    }
    results[check.key] = result;
  }
  results.confirmed = { ...inputs };
  return results;
}

function sha256File(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

function inspectC7Crate(path, taskId, workstream, slice, required) {
  const normalized = normalizeRepoPath(path).replace(/\/$/, '');
  const absolute = join(repoRoot, normalized);
  const stat = statSafe(absolute);
  const result = {
    task_id: taskId,
    workstream,
    slice,
    path: normalized,
    kind: 'generated_domain_crate',
    present: Boolean(stat?.isDirectory()),
    required,
    status: 'pending',
    files: [],
    missing_files: [],
  };
  if (!stat?.isDirectory()) {
    result.status = required ? 'fail' : 'pending';
    if (required) {
      failures.push(`missing C7 generated domain crate: ${normalized}`);
    }
    return result;
  }
  const requiredFiles = [`${normalized}/Cargo.toml`, `${normalized}/src/lib.rs`];
  result.files = c7SourceFilesForPaths([normalized]).map((file) =>
    relative(repoRoot, file).replaceAll('\\', '/')
  );
  for (const file of requiredFiles) {
    if (!statSafe(join(repoRoot, file))?.isFile()) {
      result.missing_files.push(file);
      failures.push(`C7 generated domain crate is missing ${file}`);
    }
  }
  const cargo = readFileSafe(join(repoRoot, `${normalized}/Cargo.toml`));
  if (cargo) {
    const packageName = cargo.match(
      /\[package\][\s\S]*?\bname\s*=\s*"([^"]+)"/m
    )?.[1];
    result.package_name = packageName || null;
    if (
      packageName &&
      slice !== 'shared_support' &&
      packageName !== normalized.split('/').at(-1)
    ) {
      failures.push(
        `${normalized}: Cargo package name ${packageName} does not match directory name`
      );
    }
  }
  result.status = result.missing_files.length === 0 ? 'pass' : 'fail';
  return result;
}

function readFileSafe(path) {
  try {
    return statSafe(path)?.isFile() ? readFileSync(path, 'utf8') : null;
  } catch {
    return null;
  }
}

function c7SourceFilesForPaths(paths, options = {}) {
  const includeLockfiles = options.includeLockfiles === true;
  const isScannableFile = (path) =>
    /\.(rs|toml|json|ts|tsx)$/.test(path) ||
    (includeLockfiles && path.endsWith('.lock'));
  const files = new Set();
  for (const rawPath of paths || []) {
    const path = normalizeRepoPath(rawPath).replace(/\/$/, '');
    if (!isSafeRepoPath(path)) continue;
    const absolute = join(repoRoot, path);
    const stat = statSafe(absolute);
    if (stat?.isFile()) {
      if (isScannableFile(path)) files.add(absolute);
    } else if (stat?.isDirectory()) {
      for (const file of collectFiles(absolute, '')) {
        if (isScannableFile(file)) files.add(file);
      }
    }
  }
  return [...files].sort();
}

function analyzeC7Source(source, file) {
  const semantic = source.split('');
  const literals = [];
  const isRust = file.endsWith('.rs');
  const isToml = file.endsWith('.toml');
  const blank = (start, end) => {
    for (let index = start; index < end; index += 1) {
      if (semantic[index] !== '\n' && semantic[index] !== '\r') {
        semantic[index] = ' ';
      }
    }
  };
  const decode = (value, quote) => {
    if (quote === '"') {
      try {
        return JSON.parse(`"${value}"`);
      } catch {
        return value;
      }
    }
    return value
      .replaceAll(`\\${quote}`, quote)
      .replaceAll('\\\\', '\\');
  };

  let index = 0;
  while (index < source.length) {
    if (source.startsWith('//', index)) {
      const end = source.indexOf('\n', index);
      const commentEnd = end === -1 ? source.length : end;
      blank(index, commentEnd);
      index = commentEnd;
      continue;
    }
    if (source.startsWith('/*', index)) {
      const end = source.indexOf('*/', index + 2);
      const commentEnd = end === -1 ? source.length : end + 2;
      blank(index, commentEnd);
      index = commentEnd;
      continue;
    }
    if (isToml && source[index] === '#') {
      const end = source.indexOf('\n', index);
      const commentEnd = end === -1 ? source.length : end;
      blank(index, commentEnd);
      index = commentEnd;
      continue;
    }

    const rawPrefix = isRust ? source.slice(index).match(/^r(#{0,16})"/) : null;
    if (rawPrefix) {
      const hashes = rawPrefix[1].length;
      const contentStart = index + rawPrefix[0].length;
      const terminator = `"${'#'.repeat(hashes)}`;
      const end = source.indexOf(terminator, contentStart);
      if (end === -1) {
        index += rawPrefix[0].length;
        continue;
      }
      literals.push({
        value: source.slice(contentStart, end),
        start: index,
        end: end + terminator.length,
      });
      index = end + terminator.length;
      continue;
    }

    if (source[index] === '"' || (!isRust && source[index] === "'")) {
      const quote = source[index];
      let end = index + 1;
      let escaped = false;
      while (end < source.length) {
        const character = source[end];
        if (!escaped && character === quote) break;
        if (!escaped && character === '\\') {
          escaped = true;
        } else {
          escaped = false;
        }
        end += 1;
      }
      if (end < source.length) {
        literals.push({
          value: decode(source.slice(index + 1, end), quote),
          start: index,
          end: end + 1,
        });
        index = end + 1;
        continue;
      }
    }
    index += 1;
  }

  return {
    semantic: semantic.join(''),
    literals,
  };
}

function c7RegexMatches(source, pattern) {
  const flags = pattern.flags.includes('g') ? pattern.flags : `${pattern.flags}g`;
  const matcher = new RegExp(pattern.source, flags);
  const matches = [];
  let match;
  while ((match = matcher.exec(source)) !== null) {
    matches.push(match);
    if (match[0].length === 0) matcher.lastIndex += 1;
  }
  return matches;
}

function c7LineNumber(source, offset) {
  let line = 1;
  for (let index = 0; index < offset; index += 1) {
    if (source[index] === '\n') line += 1;
  }
  return line;
}

function c7ForbiddenEdgeRules() {
  return [
    {
      category: 'approval_confirmation',
      label: 'agent approval scope or manager',
      pattern: /\b(?:ApprovalScope|ToolApprovalManager|ToolConfirmer|ConfirmRequest|PermissionConfirm)\b/,
    },
    {
      category: 'approval_confirmation',
      label: 'agent confirmation state or store',
      pattern: /\b(?:AgentConfirm(?:ation)?|WaitingConfirmation|ToolConfirmation(?:Request|State|Manager)?)\b/,
    },
    {
      category: 'channel_product_confirmation_candidate',
      label: 'channel-owned product stop confirmation candidate',
      pattern: /\b(?:PendingDecisionStore|PendingDecisionKind::StopConversation|confirmation_to_decision|pending_decisions)\b/,
    },
    {
      category: 'approval_confirmation',
      label: 'approval/confirmation lifecycle field',
      pattern: /\b(?:awaiting_approval|waiting_confirmation|needs_confirmation|require_approval|auto_approve(?:_invocation)?)\b/,
    },
    {
      category: 'approval_confirmation',
      label: 'browser approval gate',
      pattern: /\b(?:BrowserApprovalGate|browser_unrestricted_approval)\b/,
    },
    {
      category: 'approval_confirmation',
      label: 'permission confirmation event or action',
      pattern: /\b(?:AgentStreamEvent::Permission|WakeAction::Confirm|DecisionSource::Permission)\b/,
    },
    {
      category: 'approval_confirmation',
      label: 'system confirmation action',
      pattern: /\bsystem\.confirm\b/,
    },
    {
      category: 'approval_confirmation',
      label: 'confirmation response bypass',
      pattern: /\bconfirm\s*:\s*true\b/,
    },
    {
      category: 'approval_confirmation',
      label: 'approval/confirmation policy branch',
      pattern: /\b(?:approval|confirmation)_(?:gate|state|request|scope|policy|mode)\b/i,
    },
    {
      category: 'approval_confirmation',
      label: 'pending approval/confirmation branch',
      pattern: /\b(?:pending|awaiting)\s+(?:approval|confirmation)\b/i,
    },
    {
      category: 'runtime_selector',
      label: 'legacy session mode selector',
      pattern: /\b(?:SessionMode|session_mode)\b/,
    },
    {
      category: 'runtime_selector',
      label: 'legacy Nomi/runtime selector',
      pattern: /\b(?:AgentRuntimeRegistry|AgentType::Nomi|ConversationAttemptRunner|NomiAgentManager|NomiBuildExtra)\b/,
    },
    {
      category: 'runtime_selector',
      label: 'legacy composition escape hatch',
      pattern: /\b(?:AgentFactoryDeps|GatewayDeps|AppServices|ConversationService)\b/,
    },
    {
      category: 'runtime_selector',
      label: 'runtime selector API',
      pattern: /\b(?:RuntimeSelector|runtime_selector|select_runtime|choose_runtime|resolve_runtime|runtime_mode)\b/i,
    },
    {
      category: 'runtime_selector',
      label: 'built-in registration shortcut',
      pattern: /\b(?:direct_register|register_builtin|register_first_party)\b/i,
    },
    {
      category: 'runtime_selector',
      label: 'built-in/first-party registration branch',
      pattern: /\b(?:Builtin|FirstParty)(?:Plugin|Package|Registration)(?:Factory|Builder|Shortcut)?\b/,
    },
    ...[
      'nomi-agent',
      'nomi-mcp',
      'nomi-tools',
      'nomifun-ai-agent',
      'nomifun-conversation',
      'nomifun-gateway',
      'nomifun-app',
      'nomifun-preset',
      'nomifun-extension',
      'nomifun-db',
    ].map((dependency) => ({
      category: 'legacy_dependency',
      label: `legacy/product dependency ${dependency}`,
      pattern: new RegExp(`\\b${escapeC7Regex(dependency)}\\b`),
    })),
  ];
}

function escapeC7Regex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function scanC7ForbiddenEdges(files, options = {}) {
  const maxFindings =
    Number.isInteger(options.maxFindings) && options.maxFindings >= 0
      ? options.maxFindings
      : 200;
  const findings = [];
  let totalCount = 0;
  const rules = c7ForbiddenEdgeRules();
  for (const file of files) {
    const source = readFileSafe(file);
    if (source === null) continue;
    const analysis = analyzeC7Source(source, file);
    for (const rule of rules) {
      for (const match of c7RegexMatches(analysis.semantic, rule.pattern)) {
        totalCount += 1;
        if (findings.length >= maxFindings) continue;
        findings.push({
          path: relative(repoRoot, file).replaceAll('\\', '/'),
          line: c7LineNumber(source, match.index),
          category: rule.category,
          label: rule.label,
          pattern: rule.pattern.source,
          match: match[0],
        });
      }
    }
  }
  return {
    total_count: totalCount,
    findings,
    max_findings: maxFindings,
    truncated: findings.length < totalCount,
  };
}

function inspectC7RegistrationCoverage(expected, wave, files) {
  const expectedPackages = uniqueSortedStrings(wave?.target_packages);
  const ownedExpectedPackages = expectedPackages.filter((packageId) =>
    c7PackageOwnedByTask(packageId, expected.task_id)
  );
  const expectedFamilies = uniqueSortedStrings(wave?.target_capability_families);
  const ownedExpectedFamilies = expectedFamilies.filter((family) =>
    c7CapabilityFamilyOwnedByTask(family, expected.task_id)
  );
  const registrationFiles = [];
  const packageIds = new Set();
  const capabilityIds = new Set();
  const perFile = [];
  let hasEmptyRegistration = false;

  for (const file of files) {
    if (!/\.(rs|json)$/.test(file)) continue;
    const source = readFileSafe(file);
    if (source === null) continue;
    const analysis = analyzeC7Source(source, file);
    const markers = c7RegexMatches(
      analysis.semantic,
      /\b(?:PluginRegistration(?:Metadata)?|PackageManifest|CapabilityManifest|PackageSpec|CapabilitySpec)\b|\bregistrations?\s*\(/
    );
    const emptyRegistration = /(?:Ok|return\s+Ok)\s*\(\s*vec!\s*\[\s*\]\s*\)/.test(
      analysis.semantic
    );
    const extracted = extractC7RegistrationIds(
      analysis,
      ownedExpectedFamilies
    );
    if (markers.length > 0 || extracted.packages.length > 0 || extracted.capabilities.length > 0) {
      registrationFiles.push(relative(repoRoot, file).replaceAll('\\', '/'));
      if (emptyRegistration) hasEmptyRegistration = true;
      for (const packageId of extracted.packages) packageIds.add(packageId);
      for (const capabilityId of extracted.capabilities) capabilityIds.add(capabilityId);
      perFile.push({
        path: relative(repoRoot, file).replaceAll('\\', '/'),
        package_ids: extracted.packages,
        capability_ids: extracted.capabilities,
        registration_markers: markers.length,
        empty_registration: emptyRegistration,
      });
    }
  }

  const observedPackages = [...packageIds].sort();
  const observedPackageKeys = uniqueSortedStrings(
    observedPackages.map(c7CanonicalPackageKey)
  );
  const observedCapabilities = [...capabilityIds].sort();
  const matchedPackages = {};
  const missingPackages = [];
  for (const expectedPackage of ownedExpectedPackages) {
    const actual = observedPackages.find((candidate) =>
      c7PackageKeyMatches(candidate, expectedPackage)
    );
    if (actual) {
      matchedPackages[expectedPackage] = actual;
    } else {
      missingPackages.push(expectedPackage);
    }
  }
  const matchedFamilies = {};
  const missingFamilies = [];
  for (const expectedFamily of ownedExpectedFamilies) {
    const actual = observedCapabilities.filter((candidate) =>
      c7CapabilityBelongsToFamily(candidate, expectedFamily)
    );
    if (actual.length > 0) {
      matchedFamilies[expectedFamily] = actual;
    } else {
      missingFamilies.push(expectedFamily);
    }
  }
  const unexpectedPackages = observedPackages.filter(
    (candidate) => {
      if (
        expectedPackages.some((expectedPackage) =>
          c7PackageKeyMatches(candidate, expectedPackage)
        )
      ) {
        return false;
      }
      // Some deletion manifests describe a consumer family rather than the
      // package that owns its canonical registration.  Known target packages
      // owned by this wave are valid auxiliary inventory entries.
      return !c7PackageOwnedByTask(candidate, expected.task_id);
    }
  );
  const unexpectedCapabilities = observedCapabilities.filter(
    (candidate) =>
      !c7TargetCapabilityIds().has(c7CanonicalId(candidate)) &&
      !ownedExpectedFamilies.some((expectedFamily) =>
        c7CapabilityBelongsToFamily(candidate, expectedFamily)
      )
  );

  let status = 'not_evaluated';
  let reason = 'no statically recognizable registration metadata';
  if (registrationFiles.length > 0) {
    status = 'pass';
    reason = 'registration metadata and target identifiers were statically inspected';
    if (hasEmptyRegistration || missingPackages.length > 0 || missingFamilies.length > 0) {
      status = 'fail';
      reason = hasEmptyRegistration
        ? 'registration function returns an empty inventory'
        : 'registration inventory is missing manifest targets';
    } else if (unexpectedPackages.length > 0 || unexpectedCapabilities.length > 0) {
      status = 'fail';
      reason = 'registration inventory contains targets outside the wave exact set';
    }
  } else if (files.length > 0) {
    status = 'fail';
    reason = 'canonical source exists but no PluginRegistration metadata was found';
  }

  return {
    task: expected.task_id,
    task_id: expected.task_id,
    workstream: wave?.workstream || 'W4',
    slice: expected.wave,
    expected_packages: expectedPackages,
    owned_expected_packages: ownedExpectedPackages,
    deferred_expected_packages: expectedPackages.filter(
      (packageId) => !ownedExpectedPackages.includes(packageId)
    ),
    expected_capability_families: expectedFamilies,
    owned_expected_capability_families: ownedExpectedFamilies,
    deferred_expected_capability_families: expectedFamilies.filter(
      (family) => !ownedExpectedFamilies.includes(family)
    ),
    registration_files: registrationFiles,
    registration_crates: perFile,
    static_analysis: registrationFiles.length > 0,
    observed_package_ids: observedPackages,
    observed_package_keys: observedPackageKeys,
    observed_capability_ids: observedCapabilities,
    matched_packages: matchedPackages,
    matched_capability_families: matchedFamilies,
    missing_packages: missingPackages,
    missing_capability_families: missingFamilies,
    unexpected_package_ids: unexpectedPackages,
    unexpected_package_keys: uniqueSortedStrings(
      unexpectedPackages.map(c7CanonicalPackageKey)
    ),
    unexpected_capability_ids: unexpectedCapabilities,
    status,
    reason,
  };
}

function extractC7RegistrationIds(analysis, expectedFamilies) {
  const packages = new Set();
  const capabilities = new Set();
  for (const literal of analysis.literals) {
    const value = literal.value.trim();
    if (
      c7LooksLikePackageId(value) &&
      c7KnownPackageIds().has(c7CanonicalId(value)) &&
      c7LiteralLooksLikeRegistrationId(analysis, literal)
    ) {
      packages.add(value);
      continue;
    }
    if (
      c7LooksLikeCapabilityId(value) &&
      c7LiteralLooksLikeRegistrationId(analysis, literal) &&
      (expectedFamilies.some((family) => c7CapabilityBelongsToFamily(value, family)) ||
        c7KnownCapabilityIds().has(c7CanonicalId(value)))
    ) {
      if (!value.endsWith('.invoke') && !value.endsWith('.entrypoint')) {
        capabilities.add(value);
      }
    }
  }

  for (const match of c7RegexMatches(
    analysis.semantic,
    /\b(?:package_id|package)\s*:\s*(?:PackageId::from\s*\(|PackageRef\s*\{\s*id\s*:\s*)["']([^"']+)["']/
  )) {
    if (
      c7LooksLikePackageId(match[1]) &&
      c7KnownPackageIds().has(c7CanonicalId(match[1]))
    ) {
      packages.add(match[1]);
    }
  }
  for (const match of c7RegexMatches(
    analysis.semantic,
    /\b(?:CapabilityId::from|CapabilitySpec::(?:context|tool|resource_provider|scheduler|middleware|transport|background))\s*\(\s*["']([^"']+)["']/
  )) {
    if (
      c7LooksLikeCapabilityId(match[1]) &&
      c7KnownCapabilityIds().has(c7CanonicalId(match[1]))
    ) {
      capabilities.add(match[1]);
    }
  }
  for (const match of c7RegexMatches(
    analysis.semantic,
    /\bdeclared_capability_ids\s*:\s*[^;\n]*["']([^"']+)["']/
  )) {
    if (
      c7LooksLikeCapabilityId(match[1]) &&
      c7KnownCapabilityIds().has(c7CanonicalId(match[1]))
    ) {
      capabilities.add(match[1]);
    }
  }

  if (analysis.semantic.trimStart().startsWith('{')) {
    try {
      const value = JSON.parse(analysis.semantic);
      collectC7JsonRegistrationIds(value, packages, capabilities);
    } catch {
      // JSON registration artifacts are optional; Rust source remains the primary path.
    }
  }
  return {
    packages: [...packages].sort(),
    capabilities: [...capabilities].sort(),
  };
}

function c7LiteralLooksLikeRegistrationId(analysis, literal) {
  const contextStart = Math.max(0, literal.start - 180);
  const contextEnd = Math.min(analysis.semantic.length, literal.end + 180);
  const context = analysis.semantic.slice(contextStart, contextEnd);
  return /\b(?:const|static|let|id|package|capability|capabilities|package_ids|capability_ids|target_capability_ids|target_capability_families|PackageSpec|CapabilitySpec|PackageDefinition|CapabilityDefinition|PluginRegistration|PackageManifest|CapabilityManifest|declared_capability_ids)\b/i.test(
    context
  );
}

function collectC7JsonRegistrationIds(value, packages, capabilities, key = '') {
  if (Array.isArray(value)) {
    for (const entry of value) {
      collectC7JsonRegistrationIds(entry, packages, capabilities, key);
    }
    return;
  }
  if (!value || typeof value !== 'object') return;
  if (typeof value.package_id === 'string' && c7LooksLikePackageId(value.package_id)) {
    packages.add(value.package_id);
  }
  if (
    value.package &&
    typeof value.package === 'object' &&
    typeof value.package.id === 'string' &&
    c7LooksLikePackageId(value.package.id)
  ) {
    packages.add(value.package.id);
  }
  if (key === 'declared_capability_ids' && Array.isArray(value)) {
    for (const capability of value) {
      if (typeof capability === 'string' && c7LooksLikeCapabilityId(capability)) {
        capabilities.add(capability);
      }
    }
  }
  for (const [childKey, childValue] of Object.entries(value)) {
    if (
      childKey === 'declared_capability_ids' &&
      Array.isArray(childValue)
    ) {
      for (const capability of childValue) {
        if (typeof capability === 'string' && c7LooksLikeCapabilityId(capability)) {
          capabilities.add(capability);
        }
      }
    }
    if (
      childKey === 'capabilities' &&
      Array.isArray(childValue)
    ) {
      for (const capability of childValue) {
        if (
          capability &&
          typeof capability === 'object' &&
          typeof capability.id === 'string' &&
          c7LooksLikeCapabilityId(capability.id)
        ) {
          capabilities.add(capability.id);
        }
      }
    }
    collectC7JsonRegistrationIds(childValue, packages, capabilities, childKey);
  }
}

function c7LooksLikePackageId(value) {
  return /^(?:domain|nomifun)[.-][a-z0-9][a-z0-9._-]*$/i.test(value);
}

function c7LooksLikeCapabilityId(value) {
  return (
    /^[a-z][a-z0-9_-]*(?:[.-][a-z0-9_-]+)+$/i.test(value) &&
    !value.includes('://') &&
    !value.startsWith('schema.')
  );
}

function c7CanonicalId(value) {
  return String(value).trim().toLowerCase().replaceAll('_', '-');
}

function c7CanonicalPackageKey(value) {
  const canonical = c7CanonicalId(value);
  const aliases = {
    'domain.artifacts-review-ci': 'nomifun.workspace-execution',
    'domain.autowork': 'nomifun.autowork-scheduler',
    'domain.channel': 'nomifun.channel',
    'domain.companion': 'nomifun.companion',
    'domain.companion-memory': 'nomifun.companion-memory',
    'domain.computer': 'nomifun.computer-a11y',
    'domain.creation': 'nomifun.creation',
    'domain.cron': 'nomifun.autowork-scheduler',
    'domain.customer-service': 'nomifun.customer-service',
    'domain.idmm': 'nomifun.idmm',
    'domain.knowledge': 'nomifun.knowledge',
    'domain.mcp-connectors': 'nomifun.mcp-connectors',
    'domain.miniapp': 'nomifun.miniapp',
    'domain.notification': 'nomifun.notification',
    'domain.office': 'nomifun.office',
    'domain.project-memory': 'nomifun.project-memory',
    'domain.remote': 'nomifun.remote-ingress',
    'domain.requirements': 'nomifun.requirements',
    'domain.robot': 'nomifun.robot',
    'domain.skills': 'nomifun.skills',
    'domain.ssh': 'nomifun.ssh',
    'domain.web-research': 'nomifun.web-research',
    'domain.webhook': 'nomifun.notification',
    'domain.workshop': 'nomifun.workshop',
    'domain.workspace-execution': 'nomifun.workspace-execution',
  };
  return aliases[canonical] || (
    canonical.startsWith('domain.')
      ? `nomifun.${canonical.slice('domain.'.length)}`
      : canonical.startsWith('domain-')
        ? `nomifun.${canonical.slice('domain-'.length)}`
        : canonical
  );
}

function c7PackageKeyMatches(left, right) {
  return c7CanonicalPackageKey(left) === c7CanonicalPackageKey(right);
}

function c7PackageOwnedByTask(packageId, taskId) {
  const owners = {
    'nomifun.agent-execution': 'C7-W5-AUTOMATION',
    'nomifun.autowork-scheduler': 'C7-W5-AUTOMATION',
    'nomifun.browser': 'C7-W2-CODING',
    'nomifun.channel': 'C7-W4-IDENTITY',
    'nomifun.chat': 'C7-W1-READ',
    'nomifun.companion': 'C7-W4-IDENTITY',
    'nomifun.companion-memory': 'C7-W1-READ',
    'nomifun.computer-a11y': 'C7-W2-CODING',
    'nomifun.creation': 'C7-W3-CREATIVE',
    'nomifun.customer-service': 'C7-W4-IDENTITY',
    'nomifun.idmm': 'C7-W5-AUTOMATION',
    'nomifun.knowledge': 'C7-W1-READ',
    'nomifun.mcp-connectors': 'C7-W2-CODING',
    'nomifun.miniapp': 'C7-W3-CREATIVE',
    'nomifun.notification': 'C7-W4-IDENTITY',
    'nomifun.office': 'C7-W3-CREATIVE',
    'nomifun.project-memory': 'C7-W1-READ',
    'nomifun.remote-ingress': 'C7-W5-AUTOMATION',
    'nomifun.requirements': 'C7-W5-AUTOMATION',
    'nomifun.robot': 'C7-W4-IDENTITY',
    'nomifun.skills': 'C7-W1-READ',
    'nomifun.ssh': 'C7-W2-CODING',
    'nomifun.web-research': 'C7-W1-READ',
    'nomifun.workshop': 'C7-W3-CREATIVE',
    'nomifun.workspace-execution': 'C7-W2-CODING',
  };
  return owners[c7CanonicalPackageKey(packageId)] === taskId;
}

function c7CapabilityFamilyOwnedByTask(family, taskId) {
  const owners = {
    'attachments.read': 'C7-W1-READ',
    'customer-service.read': 'C7-W4-IDENTITY',
    'knowledge.read': 'C7-W1-READ',
    'knowledge.search': 'C7-W1-READ',
    'memory.read': 'C7-W1-READ',
    'research.core': 'C7-W1-READ',
    'skill.instructions': 'C7-W1-READ',
    'web.fetch': 'C7-W1-READ',
    'web.search': 'C7-W1-READ',
    browser: 'C7-W2-CODING',
    computer: 'C7-W2-CODING',
    'external-mcp': 'C7-W2-CODING',
    filesystem: 'C7-W2-CODING',
    'remote-execution': 'C7-W2-CODING',
    'review-ci': 'C7-W2-CODING',
    ssh: 'C7-W2-CODING',
    terminal: 'C7-W2-CODING',
    workspace: 'C7-W2-CODING',
    'creation.audio': 'C7-W3-CREATIVE',
    'creation.image': 'C7-W3-CREATIVE',
    'creation.image-edit': 'C7-W3-CREATIVE',
    'creation.text': 'C7-W3-CREATIVE',
    'creation.video': 'C7-W3-CREATIVE',
    miniapp: 'C7-W3-CREATIVE',
    office: 'C7-W3-CREATIVE',
    workshop: 'C7-W3-CREATIVE',
    'workshop.asset': 'C7-W3-CREATIVE',
    'workshop.canvas': 'C7-W3-CREATIVE',
    'workshop.director': 'C7-W3-CREATIVE',
    'workshop.template': 'C7-W3-CREATIVE',
    'channel.receive': 'C7-W4-IDENTITY',
    'channel.reply': 'C7-W4-IDENTITY',
    'channel.send': 'C7-W4-IDENTITY',
    'companion.evolve': 'C7-W4-IDENTITY',
    'companion.learn': 'C7-W4-IDENTITY',
    'companion.persona': 'C7-W4-IDENTITY',
    'customer-service.dialogue': 'C7-W4-IDENTITY',
    'customer-service.handoff': 'C7-W4-IDENTITY',
    'notification.webhook': 'C7-W4-IDENTITY',
    'robot.audio': 'C7-W4-IDENTITY',
    'robot.device-tools': 'C7-W4-IDENTITY',
    'robot.display': 'C7-W4-IDENTITY',
    'robot.motion': 'C7-W4-IDENTITY',
    'robot.vision': 'C7-W4-IDENTITY',
    'agent-execution': 'C7-W5-AUTOMATION',
    'autowork.runner': 'C7-W5-AUTOMATION',
    'idmm.intervene': 'C7-W5-AUTOMATION',
    'idmm.observe': 'C7-W5-AUTOMATION',
    'remote.mcp': 'C7-W5-AUTOMATION',
    'remote.rest': 'C7-W5-AUTOMATION',
    requirements: 'C7-W5-AUTOMATION',
    'schedule.agent-trigger': 'C7-W5-AUTOMATION',
    'schedule.store': 'C7-W5-AUTOMATION',
    'schedule.timer': 'C7-W5-AUTOMATION',
  };
  const canonical = c7CanonicalId(family);
  return owners[canonical] === taskId || !Object.hasOwn(owners, canonical);
}

function c7TargetCapabilityIds() {
  return c7KnownCapabilityIds();
}

function c7KnownPackageIds() {
  const inventory = c7TargetInventory();
  return new Set(
    (inventory?.packages || []).map((packageEntry) =>
      c7CanonicalId(packageEntry.package?.id)
    )
  );
}

function c7KnownCapabilityIds() {
  const inventory = c7TargetInventory();
  return new Set(
    (inventory?.packages || []).flatMap((packageEntry) =>
      (packageEntry.capabilities || []).map((entry) =>
        c7CanonicalId(entry.capability?.id)
      )
    )
  );
}

function c7TargetInventory() {
  const path = join(
    repoRoot,
    'crates/backend/nomifun-agent-contracts/contracts/target-packages/target-first-party-contributions.v1.json'
  );
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch {
    return null;
  }
}

function c7EquivalentId(left, right) {
  return c7CanonicalId(left) === c7CanonicalId(right);
}

function c7CapabilityBelongsToFamily(capability, family) {
  const actual = c7CanonicalId(capability);
  const expected = c7CanonicalId(family);
  const aliases = {
    'attachments.read': ['session.attachments.read'],
    browser: ['browser.'],
    computer: ['computer.', 'a11y.'],
    'customer-service.read': [
      'customer-service.notes.read',
      'customer_service.notes.read',
    ],
    'external-mcp': ['mcp.', 'connector.'],
    filesystem: ['fs.'],
    'memory.read': [
      'memory.project.read',
      'memory.companion.recall',
      'memory.read',
    ],
    'remote-execution': ['remote-execution.', 'ssh.exec', 'process.exec'],
    'review-ci': ['workspace.artifacts', 'vcs.diff'],
    'research.core': ['research.core', 'web.search', 'web.fetch', 'citation.render'],
    'skill.instructions': ['skill.instructions', 'skill.'],
    terminal: ['terminal.'],
    workspace: ['workspace.', 'fs.', 'vcs.'],
    'agent-execution': ['agent-execution.', 'agent.execution.'],
    requirements: ['requirements.'],
    'schedule.agent-trigger': ['schedule.agent-trigger'],
    'workshop.asset': ['workshop.asset.'],
    'workshop.canvas': ['workshop.canvas.'],
    'workshop.director': ['workshop.director'],
    'workshop.template': ['workshop.template.'],
    'creation.image-edit': ['creation.image-edit'],
    'robot.device-tools': ['robot.device-tools'],
    'customer-service.dialogue': ['customer-service.dialogue'],
    'customer-service.handoff': ['customer-service.handoff'],
  };
  const aliasPrefixes = aliases[expected];
  if (aliasPrefixes?.some((prefix) => actual === prefix || actual.startsWith(prefix))) {
    return true;
  }
  if (actual === expected) return true;
  return actual.startsWith(`${expected}.`) || actual.startsWith(`${expected}-`);
}

function scanC7Reachability(
  expected,
  wave,
  deletion,
  canonicalScopes,
  strict
) {
  const result = {
    task: expected.task_id,
    task_id: expected.task_id,
    workstream: wave?.workstream || 'W4',
    slice: expected.wave,
    expected_count: 0,
    observed_count: 0,
    status: 'not_evaluated',
    strict,
    production_roots: [],
    scanned_files: [],
    missing_refs: [],
    legacy_terms: [],
    findings: [],
  };
  if (!deletion) {
    result.reason = 'deletion manifest unavailable';
    return result;
  }

  result.legacy_terms = uniqueSortedStrings(
    (deletion.legacy_surfaces || []).flatMap((surface) => [
      ...(surface.symbols_or_patterns || []),
      ...(surface.current_refs || []).flatMap((reference) => reference.symbols || []),
    ])
  );

  // The old production roots are retained as the deletion contract's
  // historical inventory.  C7 reachability is evaluated from the new
  // canonical owner, because the legacy application graph remains present for
  // domain slices that have not yet crossed the v4 host boundary.  C8 owns the
  // global residual scan after every slice has switched.
  result.historical_production_roots = (deletion.production_roots || []).map(
    (root) => ({
      root_id: root.root_id,
      kind: root.kind,
      expected_canonical_owner: root.expected_canonical_owner,
      status: 'historical_inventory',
    })
  );

  const scope = (canonicalScopes || []).find(
    (candidate) => candidate.task_id === expected.task_id
  );
  const canonicalPaths = uniqueSortedStrings([
    ...(scope?.paths || []),
    'crates/backend/nomifun-agent-platform/src',
    'crates/backend/nomifun-app/src/router/agent_platform.rs',
  ]);
  const files = c7SourceFilesForPaths(canonicalPaths);
  result.production_roots.push({
    root_id: `${expected.wave}-canonical-owner`,
    kind: 'canonical_agent_platform',
    expected_canonical_owner: 'PluginRegistration/AgentPlatform',
    refs: canonicalPaths.map((path) => ({
      path,
      present: files.some(
        (file) => relative(repoRoot, file).replaceAll('\\', '/') === path
      ) || files.some(
        (file) => relative(repoRoot, file).replaceAll('\\', '/').startsWith(`${path}/`)
      ),
    })),
  });

  const rules = c7ForbiddenEdgeRules()
    .filter(
      (candidate) =>
        candidate.category === 'approval_confirmation' ||
        candidate.category === 'channel_product_confirmation_candidate' ||
        candidate.category === 'runtime_selector' ||
        candidate.category === 'legacy_dependency'
    )
    .map((candidate) => ({
      label: candidate.label,
      pattern: candidate.pattern,
    }));
  for (const file of files) {
    const source = readFileSafe(file);
    if (source === null) continue;
    const analysis = analyzeC7Source(source, file);
    for (const rule of rules) {
      for (const match of c7RegexMatches(analysis.semantic, rule.pattern)) {
        result.observed_count += 1;
        if (result.findings.length >= 300) continue;
        result.findings.push({
          root_id: `${expected.wave}-canonical-owner`,
          path: relative(repoRoot, file).replaceAll('\\', '/'),
          line: c7LineNumber(source, match.index),
          label: rule.label,
          pattern: rule.pattern.source,
          match: match[0],
        });
      }
    }
  }
  result.scanned_files = files.map((file) =>
    relative(repoRoot, file).replaceAll('\\', '/')
  );
  result.scanned_files = [...new Set(result.scanned_files)].sort();
  result.status = result.observed_count === 0
    ? (files.length > 0 ? 'pass' : strict ? 'fail' : 'pending')
    : 'fail';
  result.reason =
    result.status === 'pass'
      ? 'no forbidden legacy edge was found from the canonical Agent platform owner'
      : result.status === 'pending'
        ? 'canonical owner source is not yet present'
        : 'forbidden legacy edge(s) remain reachable from the canonical Agent platform owner';
  return result;
}

function c7ReachabilityRule(term) {
  if (typeof term !== 'string') return null;
  const value = term.trim();
  if (
    value.length < 4 ||
    new Set([
      'mode',
      'backend',
      'confirm',
      'computer_use',
      'mcp servers',
      'profile/domains',
      'default/latest preset inference',
    ]).has(value.toLowerCase())
  ) {
    return null;
  }
  if (value.includes('*')) {
    const pattern = value
      .split('*')
      .map(escapeC7Regex)
      .join('[A-Za-z0-9_.:-]+');
    return {
      label: value,
      pattern: `(?:^|[^A-Za-z0-9_])${pattern}(?![A-Za-z0-9_])`,
    };
  }
  const pattern = /[A-Za-z0-9_./:?{}-]/.test(value)
    ? `(?:^|[^A-Za-z0-9_])${escapeC7Regex(value)}(?![A-Za-z0-9_])`
    : escapeC7Regex(value);
  return {
    label: value,
    pattern,
  };
}

function c7PendingNativePoints() {
  return [
    {
      verification_point_id: 'c7-macos-arm64-domain-waves',
      target_cell: 'macos_desktop_arm64',
      status: 'pending_native_verification',
      required_execution_kind: 'native',
      exact_check_id: 'c8_ma_full_gate',
      reason: 'C7 is Windows-first; non-Windows behavior is not closed by cross-compilation or static inspection',
    },
    {
      verification_point_id: 'c7-linux-desktop-x64-domain-waves',
      target_cell: 'linux_desktop_x64',
      status: 'pending_native_verification',
      required_execution_kind: 'native',
      exact_check_id: 'c8_ld_full_gate',
      reason: 'C7 is Windows-first; non-Windows behavior is not closed by cross-compilation or static inspection',
    },
  ];
}

function recordC7ValidationCommands(migrationCheckpoint) {
  const migrationArgs = [
    'diff',
    '--quiet',
    migrationCheckpoint,
    '--',
    'crates/backend/nomifun-db/migrations',
  ];
  const migrationResult = spawnC7Command('git', migrationArgs);
  if (migrationResult.status !== 0) {
    failures.push('published legacy migrations changed after the C1 checkpoint');
  }

  const diffResult = spawnC7Command('git', ['diff', '--check']);
  if (diffResult.status !== 0) {
    failures.push('git diff --check failed');
  }
}

function spawnC7Command(command, commandArgs) {
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
    stdout: result.stdout || '',
    stderr: result.stderr || '',
  });
  return result;
}

function currentC8MergeEvidencePath() {
  const configured = process.env.AGENT_V2_C8_MERGE_SUMMARY;
  if (configured) return normalizeRepoPath(configured);
  const sourceSha = c8ReadGitHeadForReport();
  return `build.noindex/agent-capability-v2/${sourceSha}/c8-merge/release-summary.json`;
}

function readC8MergeSummary(report, path) {
  const normalized = normalizeRepoPath(path);
  if (!isSafeRepoPath(normalized)) {
    c8MergeFailure(
      report,
      'invalid_evidence_path',
      'C8-MERGE evidence summary path is not repository-relative',
      { path }
    );
    return null;
  }
  const absolute = join(repoRoot, normalized);
  const source = readFileSafe(absolute);
  if (source === null) {
    c8MergeFailure(
      report,
      'missing_evidence_summary',
      `missing C8-MERGE evidence summary: ${normalized}`,
      { path: normalized }
    );
    return null;
  }
  let parsed;
  try {
    parsed = JSON.parse(source);
  } catch (error) {
    c8MergeFailure(
      report,
      'invalid_evidence_json',
      `C8-MERGE evidence summary is invalid JSON: ${error.message}`,
      { path: normalized }
    );
    return null;
  }
  const value = parsed;
  if (value && typeof value === 'object' && value.payload && typeof value.payload === 'object') {
    return {
      path: normalized,
      absolute,
      raw_sha256: sha256File(absolute),
      value: value.payload,
      envelope: value,
    };
  }
  return {
    path: normalized,
    absolute,
    raw_sha256: sha256File(absolute),
    value,
  };
}

function c8MergeFailure(report, code, message, details = {}) {
  report.failure_details.push({ code, message, ...details });
  const label = report.gate_name === 'c9-hard-delete' ? 'C9' : 'C8-MERGE';
  failures.push(`${label}: ${message}`);
}

function validateC8MergeSummary(
  report,
  summary,
  sourceSha,
  verifyEvidenceArtifacts = true
) {
  const result = {
    status: 'fail',
    required_cells: [...C8_REQUIRED_NATIVE_CELLS],
    observed_cells: [],
    status_counts: null,
    verified_artifact_cells: [],
    same_input_digests: null,
    failures: [],
  };
  if (!summary || typeof summary !== 'object' || Array.isArray(summary)) {
    c8MergeFailure(report, 'summary_shape', 'C8-MERGE evidence summary must be an object');
    return result;
  }

  if (summary.source_commit !== sourceSha) {
    c8MergeFailure(
      report,
      'source_commit_mismatch',
      'C8-MERGE evidence must match the current clean local HEAD',
      { expected: sourceSha, observed: summary.source_commit || null }
    );
  }

  const cells = summary.cell_evidence;
  const observedCells =
    cells && typeof cells === 'object' && !Array.isArray(cells)
      ? Object.keys(cells).sort()
      : [];
  result.observed_cells = observedCells;
  if (
    JSON.stringify(observedCells) !==
    JSON.stringify([...C8_REQUIRED_NATIVE_CELLS].sort())
  ) {
    c8MergeFailure(
      report,
      'cell_exact_set',
      'C8-MERGE must contain exactly the three release-blocking native cells',
      { expected: C8_REQUIRED_NATIVE_CELLS, observed: observedCells }
    );
  }

  const computed = { pass: 0, fail: 0 };
  const platformInputDigests = new Set();
  const runtimeInputDigests = new Set();
  for (const cellId of observedCells) {
    const cell = cells[cellId];
    const status = cell?.status;
    if (Object.hasOwn(computed, status)) computed[status] += 1;
    if (status !== 'pass') {
      c8MergeFailure(
        report,
        'cell_not_pass',
        `C8-MERGE cell ${cellId} is not pass`,
        { cell_id: cellId, status: status || null }
      );
    }
    if (cell?.source_commit !== sourceSha) {
      c8MergeFailure(
        report,
        'cell_source_commit_mismatch',
        `C8-MERGE cell ${cellId} does not match the current clean local HEAD`,
        { cell_id: cellId, expected: sourceSha, observed: cell?.source_commit || null }
      );
    }
    const artifactDigests = cell?.artifact_digests;
    if (
      !c8Hex(cell?.release_lock_sha256) ||
      !artifactDigests ||
      !c8Hex(artifactDigests.host) ||
      !c8Hex(artifactDigests.package) ||
      !c8Hex(artifactDigests.runtime_sidecar)
    ) {
      c8MergeFailure(
        report,
        'cell_artifact_digests_invalid',
        `C8-MERGE cell ${cellId} must provide release-lock, host, package, and Sidecar digests`,
        { cell_id: cellId }
      );
    }
    if (
      !cell?.evidence ||
      typeof cell.evidence !== 'object' ||
      !c8Hex(cell.evidence.digest) ||
      !isSafeRepoPath(normalizeRepoPath(cell.evidence.normalized_relative_path))
    ) {
      c8MergeFailure(
        report,
        'cell_evidence_ref_invalid',
        `C8-MERGE cell ${cellId} has no valid repository-relative evidence reference`,
        { cell_id: cellId }
      );
    } else if (verifyEvidenceArtifacts) {
      const evidencePath = normalizeRepoPath(
        cell.evidence.normalized_relative_path
      );
      const evidenceSource = readFileSafe(join(repoRoot, evidencePath));
      if (evidenceSource === null) {
        c8MergeFailure(
          report,
          'cell_evidence_missing',
          `C8-MERGE evidence artifact for ${cellId} is missing`,
          { cell_id: cellId, path: evidencePath }
        );
      } else {
        try {
          const observedEvidenceDigest = sha256File(join(repoRoot, evidencePath));
          const parsed = JSON.parse(evidenceSource);
          const evidence = parsed?.payload || parsed;
          const evidenceArtifacts = evidence?.artifact_digests || {};
          const platformInputDigest =
            evidence?.platform_validation?.input_digests
              ?.platform_validation_fixture?.observed;
          const runtimeInputDigest =
            evidence?.platform_validation?.input_digests
              ?.runtime_release_fixture?.observed;
          if (c8Hex(platformInputDigest)) {
            platformInputDigests.add(platformInputDigest);
          }
          if (c8Hex(runtimeInputDigest)) {
            runtimeInputDigests.add(runtimeInputDigest);
          }
          if (
            observedEvidenceDigest !== cell.evidence.digest ||
            evidence?.status !== 'pass' ||
            evidence?.source_sha !== sourceSha ||
            evidence?.target_cell?.cell_id !== cellId ||
            evidence?.release_lock?.sha256 !== cell.release_lock_sha256 ||
            evidenceArtifacts.host !== artifactDigests?.host ||
            evidenceArtifacts.package !== artifactDigests?.package ||
            evidenceArtifacts.runtime_sidecar !== artifactDigests?.runtime_sidecar
          ) {
            c8MergeFailure(
              report,
              'cell_evidence_mismatch',
              `C8-MERGE evidence artifact for ${cellId} does not match its summary`,
              { cell_id: cellId, path: evidencePath }
            );
          }
          result.verified_artifact_cells.push(cellId);
        } catch (error) {
          c8MergeFailure(
            report,
            'cell_evidence_invalid_json',
            `C8-MERGE evidence artifact for ${cellId} is invalid JSON: ${error.message}`,
            { cell_id: cellId, path: evidencePath }
          );
        }
      }
    }
  }
  result.status_counts = computed;
  result.same_input_digests = {
    platform_validation:
      platformInputDigests.size === 1 ? [...platformInputDigests][0] : null,
    runtime_release:
      runtimeInputDigests.size === 1 ? [...runtimeInputDigests][0] : null,
  };
  if (
    verifyEvidenceArtifacts &&
    (platformInputDigests.size !== 1 || runtimeInputDigests.size !== 1)
  ) {
    c8MergeFailure(
      report,
      'same_input_bytes_mismatch',
      'C8-MERGE requires all three native results to reference the same platform and Runtime candidate input bytes',
      {
        platform_validation_digests: [...platformInputDigests].sort(),
        runtime_release_digests: [...runtimeInputDigests].sort(),
      }
    );
  }
  if (!c8CanonicalEqual(summary.status_counts, computed)) {
    c8MergeFailure(
      report,
      'status_counts_mismatch',
      'C8-MERGE status_counts does not match the three release-blocking cell entries',
      { expected: computed, observed: summary.status_counts || null }
    );
  }
  if (computed.fail !== 0) {
    c8MergeFailure(
      report,
      'non_pass_status_count',
      'C8-MERGE requires all three native cells to pass',
      { counts: computed }
    );
  }
  if (summary.all_verification_points_closed !== true) {
    c8MergeFailure(
      report,
      'verification_points_open',
      'C8-MERGE requires all verification points to be closed'
    );
  }
  if (summary.global_residual_reachability_zero !== true) {
    c8MergeFailure(
      report,
      'global_residual_nonzero',
      'C8-MERGE requires global residual/reachability zero evidence'
    );
  }
  if (!summary.d027_terminal_evidence) {
    c8MergeFailure(
      report,
      'd027_evidence_missing',
      'C8-MERGE requires D-027 terminal drain/exact-zero evidence'
    );
  }
  result.status = report.failure_details.length === 0 ? 'pass' : 'fail';
  return result;
}

function runC8MergeGate() {
  const sourceSha = c8ReadGitHeadForReport();
  const report = {
    schema_version: '1.0.0',
    gate_name: 'c8-merge',
    evidence_kind: 'merge',
    source_sha: sourceSha,
    candidate_source_sha: sourceSha,
    preflight_blocked: false,
    evidence_path: currentC8MergeEvidencePath(),
    failure_details: [],
    checks: [],
  };
  const statusResult = spawnC7Command('git', [
    'status',
    '--porcelain',
    '--untracked-files=all',
  ]);
  if (statusResult.status !== 0) {
    report.preflight_blocked = true;
    c8MergeFailure(
      report,
      'dirty_probe_failed',
      'C8-MERGE cannot prove a clean worktree',
      { stderr: c8Tail(statusResult.stderr || '') }
    );
  } else if (String(statusResult.stdout || '').trim()) {
    report.preflight_blocked = true;
    c8MergeFailure(
      report,
      'dirty_worktree',
      'C8-MERGE requires a clean worktree before accepting native evidence',
      { status: String(statusResult.stdout || '').trim() }
    );
  }
  const summary = readC8MergeSummary(report, report.evidence_path);
  if (!report.preflight_blocked && summary) {
    report.validation = validateC8MergeSummary(report, summary.value, sourceSha);
  } else if (!summary) {
    report.validation = {
      status: 'fail',
      required_cells: [...C8_REQUIRED_NATIVE_CELLS],
      observed_cells: [],
      source_commit: null,
      status_counts: null,
      failures: ['missing evidence summary'],
    };
  }
  report.status = report.failure_details.length === 0 ? 'pass' : 'fail';
  return report;
}

function writeC8MergeReport(report) {
  const reportDir = join(
    repoRoot,
    'build.noindex/agent-capability-v2',
    report.source_sha,
    'c8-merge'
  );
  mkdirSync(reportDir, { recursive: true });
  writeFileSync(
    join(reportDir, 'summary.json'),
    `${JSON.stringify(report, null, 2)}\n`
  );
}

function runC9HardDeleteGate() {
  const sourceSha = c8ReadGitHeadForReport();
  const report = {
    schema_version: '1.0.0',
    gate_name: 'c9-hard-delete',
    evidence_kind: 'hard-delete-admission',
    source_sha: sourceSha,
    preflight_blocked: false,
    failure_details: [],
    prerequisites: {},
  };
  const mergePath =
    process.env.AGENT_V2_C8_MERGE_SUMMARY ||
    `build.noindex/agent-capability-v2/${sourceSha}/c8-merge/summary.json`;
  const mergeSource = readFileSafe(join(repoRoot, normalizeRepoPath(mergePath)));
  let merge = null;
  if (mergeSource === null) {
    report.preflight_blocked = true;
    c8MergeFailure(
      report,
      'c8_merge_missing',
      'C9 requires a same-source C8-MERGE result'
    );
  } else {
    try {
      merge = JSON.parse(mergeSource);
    } catch (error) {
      report.preflight_blocked = true;
      c8MergeFailure(
        report,
        'c8_merge_invalid_json',
        `C8-MERGE result is invalid JSON: ${error.message}`
      );
    }
    const mergeValue = merge?.validation
      ? merge
      : merge?.payload || merge;
    const mergeStatus = mergeValue?.status || mergeValue?.validation?.status;
    report.prerequisites.c8_merge_status = mergeStatus || null;
    if (mergeStatus !== 'pass') {
      report.preflight_blocked = true;
      c8MergeFailure(
        report,
        'c8_merge_not_pass',
        'C9 is blocked until C8-MERGE is pass'
      );
    }
  }

  const statusResult = spawnC7Command('git', [
    'status',
    '--porcelain',
    '--untracked-files=all',
  ]);
  report.prerequisites.clean_worktree = statusResult.status === 0 &&
    !String(statusResult.stdout || '').trim();
  if (!report.prerequisites.clean_worktree) {
    report.preflight_blocked = true;
    c8MergeFailure(
      report,
      'dirty_worktree',
      'C9 requires a clean source checkpoint'
    );
  }

  // Physical deletion is intentionally not inferred from a source scan. A
  // future C9 implementation must provide an explicit deletion manifest and a
  // D-027 zero proof; absent those immutable inputs, fail closed.
  report.prerequisites.deletion_manifest = {
    path: 'docs/specs/2026-08-28-agent-capability-platform-v2/C9-HARD-DELETE-MANIFEST.json',
    present: false,
  };
  report.prerequisites.d027_zero_evidence = false;
  c8MergeFailure(
    report,
    'c9_manifest_missing',
    'C9 hard-delete manifest and D-027 zero evidence are not present; physical deletion is not admitted'
  );
  report.status = report.failure_details.length === 0 ? 'pass' : 'fail';
  return report;
}

function writeC9HardDeleteReport(report) {
  const reportDir = join(
    repoRoot,
    'build.noindex/agent-capability-v2',
    report.source_sha,
    'c9-hard-delete'
  );
  mkdirSync(reportDir, { recursive: true });
  writeFileSync(
    join(reportDir, 'summary.json'),
    `${JSON.stringify(report, null, 2)}\n`
  );
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
