import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const gateName = args[0];

if (
  ![
    'contract-closure',
    'c1-fullauto',
    'c2-c5-foundations',
    'c6-triad',
    'c7-domain-waves',
  ].includes(gateName)
) {
  console.error(
    'usage: bun run gate:agent-v2 -- <contract-closure|c1-fullauto|c2-c5-foundations|c6-triad|c7-domain-waves>'
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
  if (policy?.agent_count_target !== '6-8') {
    failures.push('C7 execution_policy.agent_count_target must be 6-8');
  }

  const inputs = manifest.confirmed_inputs;
  const requiredInputKeys = [
    'decision_contract_digest',
    'schema_manifest_digest',
    'contract_ledger_digest',
    'deletion_manifest_set_digest',
    'official_seed_digest',
    'runtime_feature_inventory_digest',
    'runtime_release_digest',
    'platform_validation_manifest_digest',
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
    {
      key: 'runtime_release_digest',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/runtime-release-fixture.envelope.json',
      field: 'payload_digest',
    },
    {
      key: 'platform_validation_manifest_digest',
      path: 'crates/backend/nomifun-agent-contracts/contracts/generated/canonical-v4-schema-manifest.envelope.json',
      field: 'payload.platform_validation_contract_digest',
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

function c7SourceFilesForPaths(paths) {
  const files = new Set();
  for (const rawPath of paths || []) {
    const path = normalizeRepoPath(rawPath).replace(/\/$/, '');
    if (!isSafeRepoPath(path)) continue;
    const absolute = join(repoRoot, path);
    const stat = statSafe(absolute);
    if (stat?.isFile()) {
      if (/\.(rs|toml|json|ts|tsx)$/.test(path)) files.add(absolute);
    } else if (stat?.isDirectory()) {
      for (const file of collectFiles(absolute, '')) {
        if (/\.(rs|toml|json|ts|tsx)$/.test(file)) files.add(file);
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
      pattern: /\b(?:AgentConfirm(?:ation)?|PendingDecisionStore|WaitingConfirmation|ToolConfirmation(?:Request|State|Manager)?)\b/,
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

function scanC7ForbiddenEdges(files) {
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
        if (findings.length >= 200) continue;
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
      verification_point_id: 'c7-macos-x64-domain-waves',
      target_cell: 'macos_desktop_x64',
      status: 'pending_native_verification',
      required_execution_kind: 'native',
      exact_check_id: 'c8_mx_full_gate',
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
    {
      verification_point_id: 'c7-linux-headless-x64-domain-waves',
      target_cell: 'linux_headless_x64',
      status: 'pending_native_verification',
      required_execution_kind: 'native',
      exact_check_id: 'c8_lh_full_gate',
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
