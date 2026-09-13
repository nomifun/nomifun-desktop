#!/usr/bin/env bun

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  mkdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const PLUGIN_CONTRACT_RELATIVE_PATH =
  'crates/backend/nomifun-agent-contracts/contracts/plugin-n1/plugin-n1-contract.v1.json';
export const MINIAPP_CONTRACT_RELATIVE_PATH =
  'crates/backend/nomifun-agent-contracts/contracts/miniapp-m1/miniapp-m1-contract.v1.json';
export const GATE_ID = 'plugin_n1_miniapp_m1';
export const RESULT_SCHEMA_VERSION = '1.0.0';

const STAGES = Object.freeze([
  'contract',
  'windows_candidate',
  'windows_signed_rc',
]);
const SCOPES = Object.freeze(['plugin_n1', 'miniapp_m1', 'combined']);
const SHA1_PATTERN = /^[0-9a-f]{40}$/i;
const COHORT_ID_PATTERN = /^[a-z0-9][a-z0-9._-]{0,127}$/;
const DEFAULT_TIMEOUT_MS = 10 * 60 * 1000;
const BUILD_ROOT = join(REPO_ROOT, 'build.noindex', 'plugin-n1-gate');

const PLUGIN_CONTRACT_CHECK = Object.freeze({
    check_id: 'plugin_n1_contract_tests',
    command: Object.freeze([
      'cargo',
      'test',
      '--locked',
      '-p',
      'nomifun-agent-contracts',
      '--lib',
      'plugin_n1',
      '--',
      '--test-threads=1',
    ]),
    timeout_ms: DEFAULT_TIMEOUT_MS,
  });
const MINIAPP_CONTRACT_CHECK = Object.freeze({
    check_id: 'miniapp_m1_contract_tests',
    command: Object.freeze([
      'cargo',
      'test',
      '--locked',
      '-p',
      'nomifun-agent-contracts',
      '--lib',
      'miniapp_m1',
      '--',
      '--test-threads=1',
    ]),
    timeout_ms: DEFAULT_TIMEOUT_MS,
  });
const CONTRACT_GENERATOR_CHECK = Object.freeze({
    check_id: 'plugin_n1_contract_generator_check',
    command: Object.freeze([
      'cargo',
      'run',
      '--locked',
      '-p',
      'nomifun-agent-contracts',
      '--bin',
      'agent-v2-contract',
      '--',
      'check',
    ]),
    timeout_ms: DEFAULT_TIMEOUT_MS,
  });

const WINDOWS_PRODUCT_CHECKS = Object.freeze({
  windows_candidate: Object.freeze({
    plugin_n1: Object.freeze([
      'plugin_n1_windows_runtime_selection',
      'plugin_n1_windows_host_protocol',
      'plugin_n1_windows_package_lifecycle',
      'plugin_n1_windows_candidate_apply_restore',
      'plugin_n1_windows_catalog_consumers',
      'plugin_n1_windows_authoring_share',
      'plugin_n1_windows_installed_app_smoke',
      'plugin_n1_windows_fault_cleanup',
    ]),
    miniapp_m1: Object.freeze([
      'miniapp_m1_windows_release_lifecycle',
      'miniapp_m1_windows_service_host',
      'miniapp_m1_windows_bridge_storage',
      'miniapp_m1_windows_catalog_consumers',
      'miniapp_m1_windows_library_workshop_surface',
      'miniapp_m1_windows_share_import_delete',
      'miniapp_m1_windows_installed_app_smoke',
      'miniapp_m1_windows_fault_cleanup',
    ]),
    combined: Object.freeze([
      'plugin_miniapp_windows_shared_runtime_isolation',
      'plugin_miniapp_windows_catalog_provenance',
    ]),
  }),
  windows_signed_rc: Object.freeze({
    plugin_n1: Object.freeze([
      'plugin_n1_windows_signed_package_provenance',
      'plugin_n1_windows_signed_install_author_apply_invoke_restore',
      'plugin_n1_windows_signed_process_cleanup',
    ]),
    miniapp_m1: Object.freeze([
      'miniapp_m1_windows_signed_package_provenance',
      'miniapp_m1_windows_signed_install_build_publish_surface_rollback',
      'miniapp_m1_windows_signed_service_bridge_storage',
      'miniapp_m1_windows_signed_process_cleanup',
    ]),
    combined: Object.freeze([
      'plugin_miniapp_windows_signed_shared_catalog',
      'plugin_miniapp_windows_signed_uninstall_cleanup',
    ]),
  }),
});

const PRODUCT_CHECK_COMMANDS = Object.freeze({
  plugin_n1_windows_installed_app_smoke: [
    'bun',
    'scripts/validation/run-windows-plugin-product-candidate.mjs',
    '--current-candidate',
  ],
  miniapp_m1_windows_installed_app_smoke: [
    'bun',
    'scripts/validation/run-windows-miniapp-product-candidate.mjs',
    '--current-candidate',
  ],
  plugin_n1_windows_runtime_selection: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-js-runtime',
    '--lib',
    '--',
    '--test-threads=1',
  ],
  plugin_n1_windows_host_protocol: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-js-host',
    '--lib',
    '--',
    '--test-threads=1',
  ],
  plugin_n1_windows_package_lifecycle: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-plugin-platform',
    '--test',
    'application_service',
    '--',
    '--test-threads=1',
  ],
  plugin_n1_windows_candidate_apply_restore: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-plugin-platform',
    '--test',
    'application_service',
    'candidate_stale_apply_then_restore_use_exact_digest_cas',
    '--',
    '--test-threads=1',
  ],
  plugin_n1_windows_catalog_consumers: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-agent-platform',
    '--lib',
    'catalog_materialization_tests',
    '--',
    '--test-threads=1',
  ],
  plugin_n1_windows_authoring_share: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-js-authoring',
    '--lib',
    '--',
    '--test-threads=1',
  ],
  plugin_n1_windows_fault_cleanup: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-js-host',
    '--lib',
    '--',
    '--test-threads=1',
  ],
  miniapp_m1_windows_release_lifecycle: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-plugin-platform',
    '--test',
    'm1_build_tests',
    '--test',
    'm1_application',
    '--',
    '--test-threads=1',
  ],
  miniapp_m1_windows_service_host: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-plugin-platform',
    '--test',
    'service_process',
    '--',
    '--test-threads=1',
  ],
  miniapp_m1_windows_bridge_storage: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-plugin-platform',
    '--test',
    'service_storage_ipc',
    '--test',
    'managed_storage',
    '--',
    '--test-threads=1',
  ],
  miniapp_m1_windows_catalog_consumers: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-agent-platform',
    '--lib',
    'catalog_materialization_tests',
    '--',
    '--test-threads=1',
  ],
  miniapp_m1_windows_library_workshop_surface: [
    'bun',
    'test',
    'ui/src/renderer/components/layout/Sider/pluginRuntimeNav.structure.test.ts',
    'ui/src/renderer/pages/plugins/runtime/model.test.ts',
  ],
  miniapp_m1_windows_share_import_delete: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-plugin-platform',
    '--test',
    'share_application',
    '--test',
    'backup_application',
    '--test',
    'service_application',
    '--',
    '--test-threads=1',
  ],
  miniapp_m1_windows_fault_cleanup: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-plugin-platform',
    '--test',
    'service_process',
    '--test',
    'service_storage_ipc',
    '--',
    '--test-threads=1',
  ],
  plugin_miniapp_windows_shared_runtime_isolation: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-js-runtime',
    '-p',
    'nomifun-js-host',
    '-p',
    'nomifun-plugin-platform',
    '--tests',
    '--',
    '--test-threads=1',
  ],
  plugin_miniapp_windows_catalog_provenance: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-agent-platform',
    '--lib',
    'catalog_materialization_tests',
    '--',
    '--test-threads=1',
  ],
  plugin_n1_windows_signed_package_provenance: [
    'bun',
    'test',
    'scripts/release/release-lock.test.mjs',
  ],
  plugin_n1_windows_signed_install_author_apply_invoke_restore: [
    'bun',
    'scripts/validation/run-windows-signed-rc-product.mjs',
    '--scope',
    'plugin_n1',
  ],
  plugin_n1_windows_signed_process_cleanup: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-js-host',
    '--lib',
    '--',
    '--test-threads=1',
  ],
  miniapp_m1_windows_signed_package_provenance: [
    'cargo',
    'run',
    '--locked',
    '-p',
    'nomifun-agent-contracts',
    '--bin',
    'agent-v2-contract',
    '--',
    'check',
  ],
  miniapp_m1_windows_signed_install_build_publish_surface_rollback: [
    'bun',
    'scripts/validation/run-windows-signed-rc-product.mjs',
    '--scope',
    'miniapp_m1',
  ],
  miniapp_m1_windows_signed_service_bridge_storage: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-plugin-platform',
    '--test',
    'service_storage_ipc',
    '--test',
    'managed_storage',
    '--',
    '--test-threads=1',
  ],
  miniapp_m1_windows_signed_process_cleanup: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-plugin-platform',
    '--test',
    'service_process',
    '--',
    '--test-threads=1',
  ],
  plugin_miniapp_windows_signed_shared_catalog: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-agent-platform',
    '--lib',
    'catalog_materialization_tests',
    '--',
    '--test-threads=1',
  ],
  plugin_miniapp_windows_signed_uninstall_cleanup: [
    'cargo',
    'test',
    '--locked',
    '-p',
    'nomifun-plugin-platform',
    '--test',
    'application_service',
    '--',
    '--test-threads=1',
  ],
});

class GateUsageError extends Error {
  constructor(message) {
    super(message);
    this.name = 'GateUsageError';
  }
}

class GateAdmissionError extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.name = 'GateAdmissionError';
    this.code = code;
    this.details = details;
  }
}

function usage() {
  return [
    'usage:',
    '  bun run gate:plugin-n1 -- --self-test',
    '  bun run gate:plugin-n1 -- --stage <contract|windows_candidate|windows_signed_rc>',
    '    --scope <plugin_n1|miniapp_m1|combined> [--cohort <id>] [--cell <cell_id>]',
    '    [--not-delivered] [--output <build.noindex path>] [--dry-run]',
    '',
    'Candidate and signed RC runs require --cohort. --source-commit is intentionally unsupported;',
    'an operational result captures it only from a clean Git HEAD.',
  ].join('\n');
}

export function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === '--self-test') {
    return { selfTest: true };
  }
  if (argv.includes('--self-test')) {
    throw new GateUsageError('--self-test cannot be combined with other arguments');
  }

  const values = new Map();
  const booleans = new Set();
  const valueFlags = new Set([
    '--stage',
    '--scope',
    '--cohort',
    '--cell',
    '--output',
  ]);
  const booleanFlags = new Set(['--dry-run', '--not-delivered']);
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (booleanFlags.has(flag)) {
      if (booleans.has(flag)) throw new GateUsageError(`duplicate argument: ${flag}`);
      booleans.add(flag);
      continue;
    }
    if (!valueFlags.has(flag)) {
      throw new GateUsageError(`unknown argument: ${flag}`);
    }
    if (values.has(flag)) throw new GateUsageError(`duplicate argument: ${flag}`);
    const value = argv[index + 1];
    if (!value || value.startsWith('--')) {
      throw new GateUsageError(`${flag} requires a value`);
    }
    values.set(flag, value);
    index += 1;
  }

  const stage = values.get('--stage');
  const scope = values.get('--scope');
  if (!STAGES.includes(stage)) {
    throw new GateUsageError(`--stage must be one of: ${STAGES.join(', ')}`);
  }
  if (!SCOPES.includes(scope)) {
    throw new GateUsageError(`--scope must be one of: ${SCOPES.join(', ')}`);
  }

  const cohortId = values.get('--cohort') || null;
  if (cohortId && !COHORT_ID_PATTERN.test(cohortId)) {
    throw new GateUsageError(
      '--cohort must be a lowercase machine key with at most 128 characters',
    );
  }
  if (stage !== 'contract' && !cohortId) {
    throw new GateUsageError('--cohort is required for candidate and signed RC stages');
  }

  const cellId = values.get('--cell') || null;
  const notDelivered = booleans.has('--not-delivered');
  if (stage === 'contract' && (cellId || notDelivered)) {
    throw new GateUsageError('contract stage does not accept --cell or --not-delivered');
  }
  if (notDelivered && !cellId) {
    throw new GateUsageError('--not-delivered requires an explicit --cell');
  }

  return {
    selfTest: false,
    stage,
    scope,
    cohortId,
    cellId,
    output: values.get('--output') || null,
    dryRun: booleans.has('--dry-run'),
    notDelivered,
  };
}

function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

function uniqueSortedStrings(values, field) {
  if (!Array.isArray(values) || values.length === 0) {
    throw new GateAdmissionError(
      'invalid_contract_matrix',
      `${field} must be a non-empty array`,
    );
  }
  for (const value of values) {
    if (typeof value !== 'string' || value.length === 0 || value.trim() !== value) {
      throw new GateAdmissionError(
        'invalid_contract_matrix',
        `${field} contains an invalid cell id`,
      );
    }
  }
  const unique = [...new Set(values)].sort();
  if (unique.length !== values.length) {
    throw new GateAdmissionError(
      'invalid_contract_matrix',
      `${field} contains duplicate cell ids`,
    );
  }
  return unique;
}

export function validationMatrixFromContract(contract) {
  const requiredCells = uniqueSortedStrings(contract?.required_cells, 'required_cells');
  const optionalCells = uniqueSortedStrings(contract?.optional_cells, 'optional_cells');
  const overlap = requiredCells.filter((cell) => optionalCells.includes(cell));
  if (overlap.length > 0) {
    throw new GateAdmissionError(
      'invalid_contract_matrix',
      `required and optional cells overlap: ${overlap.join(', ')}`,
    );
  }

  const allCells = [...requiredCells, ...optionalCells].sort();
  const windowsCells = allCells.filter((cell) => cell.startsWith('windows_'));
  if (windowsCells.length !== 1 || !requiredCells.includes(windowsCells[0])) {
    throw new GateAdmissionError(
      'invalid_contract_matrix',
      'the canonical contract must expose exactly one required Windows cell',
    );
  }
  return {
    requiredCells,
    optionalCells,
    allCells,
    windowsCell: windowsCells[0],
  };
}

export function validateStageCell({
  stage,
  requestedCell,
  notDelivered,
  hostPlatform,
  hostArch,
  matrix,
}) {
  if (stage === 'contract') {
    if (requestedCell || notDelivered) {
      throw new GateAdmissionError(
        'contract_stage_has_platform_input',
        'contract stage cannot issue a platform result',
      );
    }
    return {
      cellId: null,
      requirement: null,
      status: null,
      notDeliveredReason: null,
    };
  }

  const cellId = requestedCell || matrix.windowsCell;
  if (!matrix.allCells.includes(cellId)) {
    throw new GateAdmissionError(
      'unknown_platform_cell',
      `cell is not declared by the canonical contract: ${cellId}`,
    );
  }
  const requirement = matrix.requiredCells.includes(cellId) ? 'required' : 'optional';
  if (notDelivered) {
    if (requirement !== 'optional') {
      throw new GateAdmissionError(
        'required_cell_not_delivered',
        'a required cell cannot be marked not_delivered',
      );
    }
    return {
      cellId,
      requirement,
      status: 'not_delivered',
      notDeliveredReason: 'not_in_release_scope',
    };
  }

  if (hostPlatform !== 'win32') {
    throw new GateAdmissionError(
      'windows_gate_wrong_host',
      'Windows candidate and signed RC evidence must run on a Windows host',
      { host_platform: hostPlatform },
    );
  }
  if (cellId !== matrix.windowsCell) {
    throw new GateAdmissionError(
      'host_cell_impersonation',
      `a Windows host cannot issue delivered evidence for ${cellId}`,
      { host_platform: hostPlatform, allowed_cell: matrix.windowsCell },
    );
  }
  if (!cellId.endsWith(`_${hostArch}`)) {
    throw new GateAdmissionError(
      'host_arch_mismatch',
      `host architecture ${hostArch} does not match ${cellId}`,
      { host_arch: hostArch, cell_id: cellId },
    );
  }
  return {
    cellId,
    requirement,
    status: null,
    notDeliveredReason: null,
  };
}

export function evaluateCleanHead({
  headStatus,
  headOutput,
  worktreeStatus,
  worktreeOutput,
}) {
  const head = String(headOutput || '').trim().toLowerCase();
  const dirtyEntries = String(worktreeOutput || '')
    .split(/\r?\n/)
    .map((line) => line.trimEnd())
    .filter(Boolean);
  const errors = [];
  if (headStatus !== 0 || !SHA1_PATTERN.test(head)) errors.push('head_probe_failed');
  if (worktreeStatus !== 0) errors.push('worktree_probe_failed');
  if (dirtyEntries.length > 0) errors.push('dirty_worktree');
  return {
    status: errors.length === 0 ? 'pass' : 'fail',
    sourceCommit: SHA1_PATTERN.test(head) ? head : null,
    dirtyEntries,
    errors,
  };
}

function gitProbe(args) {
  return spawnSync('git', args, {
    cwd: REPO_ROOT,
    encoding: 'utf8',
    shell: false,
    stdio: 'pipe',
    timeout: 30_000,
  });
}

function captureCleanHead() {
  const head = gitProbe(['rev-parse', '--verify', 'HEAD']);
  const worktree = gitProbe([
    'status',
    '--porcelain=v1',
    '--untracked-files=all',
    '--',
    '.',
    ':(exclude)build.noindex',
    ':(exclude).githooks',
    ':(exclude).githooks/**',
  ]);
  const evaluation = evaluateCleanHead({
    headStatus: head.status,
    headOutput: head.stdout,
    worktreeStatus: worktree.status,
    worktreeOutput: worktree.stdout,
  });
  if (evaluation.status !== 'pass') {
    throw new GateAdmissionError(
      'clean_head_required',
      `operational Gate requires a clean HEAD (${evaluation.errors.join(', ')})`,
      { dirty_entries: evaluation.dirtyEntries },
    );
  }
  return evaluation.sourceCommit;
}

export function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, canonicalize(value[key])]),
    );
  }
  return value;
}

export function canonicalDigest(value) {
  return createHash('sha256')
    .update(JSON.stringify(canonicalize(value)))
    .digest('hex');
}

function sha256File(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

export function buildCohort({
  cohortId,
  sourceCommit,
  contractDigests,
  cargoLockDigest,
  matrix,
}) {
  if (!SHA1_PATTERN.test(sourceCommit)) {
    throw new GateAdmissionError(
      'invalid_source_commit',
      'cohort source commit must come from a clean 40-hex HEAD',
    );
  }
  const contractSetDigest = canonicalDigest(contractDigests);
  const resolvedCohortId =
    cohortId ||
    `n1-m1-${sourceCommit.slice(0, 12)}-${contractSetDigest.slice(0, 12)}`;
  const digestInput = {
    schema_version: RESULT_SCHEMA_VERSION,
    cohort_id: resolvedCohortId,
    source_commit: sourceCommit.toLowerCase(),
    input_digests: {
      cargo_lock: cargoLockDigest,
      plugin_n1_contract: contractDigests.plugin_n1_contract,
      miniapp_m1_contract: contractDigests.miniapp_m1_contract,
    },
    required_cells: [...matrix.requiredCells].sort(),
    optional_cells: [...matrix.optionalCells].sort(),
  };
  return {
    ...digestInput,
    digest_algorithm: 'sorted-json-sha256-v1',
    cohort_digest: canonicalDigest(digestInput),
  };
}

function productCheckIds(stage, scope) {
  if (stage === 'contract') return [];
  const registry = WINDOWS_PRODUCT_CHECKS[stage];
  if (!registry) {
    throw new GateAdmissionError('unknown_stage_registry', `no registry for ${stage}`);
  }
  if (scope === 'plugin_n1') return [...registry.plugin_n1];
  if (scope === 'miniapp_m1') return [...registry.miniapp_m1];
  return [
    ...registry.plugin_n1,
    ...registry.miniapp_m1,
    ...registry.combined,
  ];
}

export function checkPlan(stage, scope, notDelivered = false) {
  if (notDelivered) return [];
  const contractChecks = [
    ...(scope === 'plugin_n1' || scope === 'combined'
      ? [PLUGIN_CONTRACT_CHECK]
      : []),
    ...(scope === 'miniapp_m1' || scope === 'combined'
      ? [MINIAPP_CONTRACT_CHECK]
      : []),
    CONTRACT_GENERATOR_CHECK,
  ];
  const runnable = contractChecks.map((check) => ({
    check_id: check.check_id,
    required: true,
    implementation: 'implemented',
    command: [...check.command],
    timeout_ms: check.timeout_ms,
  }));
  const productChecks = productCheckIds(stage, scope).map((checkId) => {
    const command = PRODUCT_CHECK_COMMANDS[checkId];
    return {
      check_id: checkId,
      required: true,
      implementation: command ? 'implemented' : 'pending',
      command: command ? [...command] : null,
      timeout_ms: command ? DEFAULT_TIMEOUT_MS : null,
    };
  });
  return [...runnable, ...productChecks];
}

function runCheck(check) {
  if (check.implementation !== 'implemented' || !check.command) {
    return {
      ...check,
      status: 'blocked',
      reason: 'check_runner_not_implemented',
      exit_code: null,
      duration_ms: 0,
    };
  }
  const startedAt = Date.now();
  const [command, ...args] = check.command;
  const result = spawnSync(command, args, {
    cwd: REPO_ROOT,
    encoding: 'utf8',
    shell: false,
    stdio: 'pipe',
    timeout: check.timeout_ms,
  });
  const timedOut = result.error?.code === 'ETIMEDOUT';
  return {
    ...check,
    status: result.status === 0 && !result.error ? 'pass' : 'fail',
    exit_code: result.status,
    duration_ms: Date.now() - startedAt,
    ...(timedOut ? { reason: 'timeout' } : {}),
    ...(!timedOut && result.error ? { reason: 'spawn_failed' } : {}),
  };
}

function isPathWithin(parent, candidate) {
  const pathFromParent = relative(resolve(parent), resolve(candidate));
  return (
    pathFromParent === '' ||
    (!pathFromParent.startsWith(`..${sep}`) &&
      pathFromParent !== '..' &&
      !isAbsolute(pathFromParent))
  );
}

function validateOutputPath(path) {
  const output = resolve(REPO_ROOT, path);
  if (!isPathWithin(BUILD_ROOT, output)) {
    throw new GateUsageError('Gate output must remain under build.noindex/plugin-n1-gate');
  }
  return output;
}

function defaultOutputPath(cohort, stage, scope, cellId) {
  const leaf = cellId ? `${scope}-${cellId}.result.json` : `${scope}.result.json`;
  return join(BUILD_ROOT, cohort.cohort_id, stage, leaf);
}

function artifactDigests() {
  const artifacts = {
    cargo_lock: sha256File(join(REPO_ROOT, 'Cargo.lock')),
    plugin_n1_contract: sha256File(
      join(REPO_ROOT, PLUGIN_CONTRACT_RELATIVE_PATH),
    ),
    miniapp_m1_contract: sha256File(
      join(REPO_ROOT, MINIAPP_CONTRACT_RELATIVE_PATH),
    ),
  };
  const generatedArtifacts = [
    [
      'plugin_n1_contract_envelope',
      'crates/backend/nomifun-agent-contracts/contracts/generated/plugin-n1-contract.envelope.json',
    ],
    [
      'miniapp_m1_contract_envelope',
      'crates/backend/nomifun-agent-contracts/contracts/generated/miniapp-m1-contract.envelope.json',
    ],
    [
      'contract_schema_registry',
      'crates/backend/nomifun-agent-contracts/contracts/generated/schemas.json',
    ],
  ];
  for (const [artifactId, path] of generatedArtifacts) {
    const absolute = join(REPO_ROOT, path);
    try {
      if (statSync(absolute).isFile()) artifacts[artifactId] = sha256File(absolute);
    } catch {
      // The generator check reports a missing canonical artifact.
    }
  }
  return artifacts;
}

function dryRunPlan(options, contractDigests, matrix, cell) {
  return {
    schema_version: RESULT_SCHEMA_VERSION,
    gate: GATE_ID,
    mode: 'dry_run',
    pre_run_inputs: {
      stage: options.stage,
      scope: options.scope,
      cohort_id: options.cohortId,
      cell_id: cell.cellId,
      not_delivered: options.notDelivered,
      contract_digests: contractDigests,
    },
    runtime_admission: {
      source_commit: 'capture_from_clean_head',
      clean_worktree_required: true,
      delivered_windows_evidence_host: 'win32',
    },
    validation_matrix: {
      required_cells: matrix.requiredCells,
      optional_cells: matrix.optionalCells,
    },
    checks: checkPlan(options.stage, options.scope, options.notDelivered),
  };
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function assertThrows(callback, pattern, message) {
  try {
    callback();
  } catch (error) {
    assert(pattern.test(error.message), `${message}: ${error.message}`);
    return;
  }
  throw new Error(`${message}: expected an error`);
}

function runSelfTest() {
  const repositoryContract = readJson(
    join(REPO_ROOT, PLUGIN_CONTRACT_RELATIVE_PATH),
  );
  readJson(join(REPO_ROOT, MINIAPP_CONTRACT_RELATIVE_PATH));
  const matrix = validationMatrixFromContract(repositoryContract);
  const foreignRequiredCell = matrix.requiredCells.find(
    (cell) => cell !== matrix.windowsCell,
  );
  const optionalCell = matrix.optionalCells[0];
  assert(foreignRequiredCell, 'contract must include a non-Windows required cell');
  assert(optionalCell, 'contract must include an optional cell');

  assertThrows(
    () =>
      parseArgs([
        '--stage',
        'contract',
        '--scope',
        'combined',
        '--source-commit',
        'a'.repeat(40),
      ]),
    /unknown argument: --source-commit/,
    'source commit must not be accepted as pre-run input',
  );
  assertThrows(
    () => parseArgs(['--stage', 'candidate', '--scope', 'combined']),
    /--stage must be one of/,
    'unknown stage must fail',
  );

  const dirty = evaluateCleanHead({
    headStatus: 0,
    headOutput: 'a'.repeat(40),
    worktreeStatus: 0,
    worktreeOutput: ' M package.json\n',
  });
  assert(dirty.status === 'fail', 'dirty tree must be rejected');
  assert(dirty.errors.includes('dirty_worktree'), 'dirty tree reason is missing');

  assertThrows(
    () =>
      validateStageCell({
        stage: 'windows_candidate',
        requestedCell: foreignRequiredCell,
        notDelivered: false,
        hostPlatform: 'win32',
        hostArch: 'x64',
        matrix,
      }),
    /Windows host cannot issue delivered evidence/,
    'Windows host must not impersonate macOS',
  );
  assertThrows(
    () =>
      validateStageCell({
        stage: 'windows_candidate',
        requestedCell: foreignRequiredCell,
        notDelivered: true,
        hostPlatform: 'win32',
        hostArch: 'x64',
        matrix,
      }),
    /required cell cannot be marked not_delivered/,
    'required cells must not be marked not delivered',
  );
  const optional = validateStageCell({
    stage: 'windows_candidate',
    requestedCell: optionalCell,
    notDelivered: true,
    hostPlatform: 'win32',
    hostArch: 'x64',
    matrix,
  });
  assert(optional.status === 'not_delivered', 'optional not-delivered status is missing');
  assert(
    optional.notDeliveredReason === 'not_in_release_scope',
    'optional not-delivered reason is missing',
  );

  const cohortInput = {
    cohortId: 'cohort-fixture',
    sourceCommit: 'a'.repeat(40),
    contractDigests: {
      plugin_n1_contract: 'b'.repeat(64),
      miniapp_m1_contract: 'd'.repeat(64),
    },
    cargoLockDigest: 'c'.repeat(64),
    matrix,
  };
  const first = buildCohort(cohortInput);
  const second = buildCohort({
    ...cohortInput,
    matrix: {
      ...matrix,
      requiredCells: [...matrix.requiredCells].reverse(),
      optionalCells: [...matrix.optionalCells].reverse(),
    },
  });
  assert(first.cohort_digest === second.cohort_digest, 'cohort digest is not stable');

  const plan = dryRunPlan(
    {
      stage: 'contract',
      scope: 'combined',
      cohortId: null,
      notDelivered: false,
    },
    {
      plugin_n1_contract: 'd'.repeat(64),
      miniapp_m1_contract: 'e'.repeat(64),
    },
    matrix,
    { cellId: null },
  );
  assert(
    !Object.hasOwn(plan.pre_run_inputs, 'source_commit'),
    'pre-run inputs must not contain source_commit',
  );
  assert(
    !JSON.stringify(plan.pre_run_inputs).includes('a'.repeat(40)),
    'pre-run inputs contain a source SHA self-reference',
  );
  assert(
    checkPlan('windows_signed_rc', 'combined')
      .filter((check) => check.required)
      .every((check) => check.implementation === 'implemented' && check.command),
    'signed RC registry still contains an unimplemented required check',
  );
  assert(
    checkPlan('windows_signed_rc', 'combined').filter((check) =>
      [
        'plugin_n1_windows_signed_install_author_apply_invoke_restore',
        'miniapp_m1_windows_signed_install_build_publish_surface_rollback',
      ].includes(check.check_id),
    ).every((check) =>
      check.command.includes('scripts/validation/run-windows-signed-rc-product.mjs'),
    ),
    'signed RC product checks do not use the fail-closed Authenticode runner',
  );
  assert(
    checkPlan('contract', 'combined').some(
      (check) => check.check_id === 'miniapp_m1_contract_tests',
    ),
    'combined contract plan is missing MiniApp M1 tests',
  );

  console.log('plugin N1/MiniApp M1 gate self-test passed');
}

function writeResult(path, result) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(result, null, 2)}\n`);
}

function runOperational(options, contractDigests, matrix, cell) {
  const sourceCommit = captureCleanHead();
  const artifacts = artifactDigests();
  const cohort = buildCohort({
    cohortId: options.cohortId,
    sourceCommit,
    contractDigests,
    cargoLockDigest: artifacts.cargo_lock,
    matrix,
  });
  const checks = checkPlan(options.stage, options.scope, options.notDelivered).map(runCheck);
  const blockingChecks = checks.filter(
    (check) => check.required && check.status !== 'pass',
  );
  const status = options.notDelivered
    ? 'not_delivered'
    : blockingChecks.length === 0
      ? 'pass'
      : 'fail';
  const result = {
    schema_version: RESULT_SCHEMA_VERSION,
    gate: GATE_ID,
    source_commit: sourceCommit,
    cohort,
    stage: options.stage,
    scope: options.scope,
    cell_id: cell.cellId,
    requirement: cell.requirement,
    host: {
      platform: process.platform,
      arch: process.arch,
    },
    status,
    ...(cell.notDeliveredReason
      ? { not_delivered_reason: cell.notDeliveredReason }
      : {}),
    checks,
    artifact_digests: options.notDelivered ? {} : artifacts,
    generated_at: new Date().toISOString(),
  };
  const output = options.output
    ? validateOutputPath(options.output)
    : defaultOutputPath(cohort, options.stage, options.scope, cell.cellId);
  writeResult(output, result);
  console.log(`plugin N1/MiniApp M1 gate result: ${relative(REPO_ROOT, output)}`);
  console.log(`status=${status} cohort_digest=${cohort.cohort_digest}`);
  if (blockingChecks.length > 0) {
    console.error(
      `blocked checks: ${blockingChecks.map((check) => check.check_id).join(', ')}`,
    );
  }
  return status === 'pass' || status === 'not_delivered' ? 0 : 1;
}

function main() {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    console.error(usage());
    return 2;
  }
  if (options.selfTest) {
    try {
      runSelfTest();
      return 0;
    } catch (error) {
      console.error(`self-test failed: ${error.message}`);
      return 1;
    }
  }

  try {
    const pluginContractPath = join(REPO_ROOT, PLUGIN_CONTRACT_RELATIVE_PATH);
    const miniappContractPath = join(REPO_ROOT, MINIAPP_CONTRACT_RELATIVE_PATH);
    const contract = readJson(pluginContractPath);
    readJson(miniappContractPath);
    const contractDigests = {
      plugin_n1_contract: sha256File(pluginContractPath),
      miniapp_m1_contract: sha256File(miniappContractPath),
    };
    const matrix = validationMatrixFromContract(contract);
    const cell = validateStageCell({
      stage: options.stage,
      requestedCell: options.cellId,
      notDelivered: options.notDelivered,
      hostPlatform: process.platform,
      hostArch: process.arch,
      matrix,
    });
    if (options.output) validateOutputPath(options.output);
    if (options.dryRun) {
      console.log(
        JSON.stringify(dryRunPlan(options, contractDigests, matrix, cell), null, 2),
      );
      return 0;
    }
    return runOperational(options, contractDigests, matrix, cell);
  } catch (error) {
    if (error instanceof GateAdmissionError) {
      console.error(`${error.code}: ${error.message}`);
    } else {
      console.error(error.message);
    }
    return error instanceof GateUsageError ? 2 : 1;
  }
}

process.exitCode = main();
