#!/usr/bin/env bun

import { execFileSync } from 'node:child_process';
import { existsSync, lstatSync, readFileSync, readdirSync } from 'node:fs';
import { dirname, extname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SELF = 'scripts/check-unified-plugin-boundary.mjs';
const SOURCE_EXTENSIONS = new Set(['.js', '.json', '.mjs', '.rs', '.sql', '.ts', '.tsx']);

const CANONICAL_FILES = [
  'crates/backend/nomifun-agent-contracts/src/plugin.rs',
  'crates/backend/nomifun-agent-contracts/contracts/plugin/unified-plugin-contract.v1.json',
  'crates/backend/nomifun-agent-contracts/contracts/generated/unified-plugin-contract.envelope.json',
  'crates/backend/nomifun-api-types/src/plugin_platform.rs',
  'crates/backend/nomifun-plugin-platform/src/model.rs',
  'crates/backend/nomifun-plugin-platform/src/install.rs',
  'crates/backend/nomifun-plugin-platform/src/data_root.rs',
  'crates/backend/nomifun-plugin-platform/src/draft.rs',
  'crates/backend/nomifun-plugin-platform/src/bindings.rs',
  'crates/backend/nomifun-plugin-platform/src/service_process.rs',
  'crates/backend/nomifun-plugin-platform/src/assets/plugin-sdk.js',
  'crates/backend/nomifun-app/src/router/plugin.rs',
  'ui/src/common/types/pluginPlatform.ts',
  'ui/src/common/adapter/pluginPlatformBridge.ts',
  'ui/src/renderer/pages/plugins/PluginLibraryPage.tsx',
  'ui/src/renderer/pages/plugins/PluginCreatorPage.tsx',
  'ui/src/renderer/pages/plugins/PluginRunPage.tsx',
  'ui/src/renderer/pages/plugins/PluginSurfacePanel.tsx',
];

const RETIRED_FILES = [
  'crates/backend/nomifun-agent-contracts/src/plugin_n1.rs',
  'crates/backend/nomifun-agent-contracts/src/plugin_runtime.rs',
  'crates/backend/nomifun-agent-contracts/contracts/plugin-n1/plugin-n1-contract.v1.json',
  'crates/backend/nomifun-agent-contracts/contracts/plugin-runtime/plugin-runtime-contract.v1.json',
  'crates/backend/nomifun-agent-contracts/contracts/generated/plugin-n1-contract.envelope.json',
  'crates/backend/nomifun-agent-contracts/contracts/generated/plugin-runtime-contract.envelope.json',
  'crates/backend/nomifun-api-types/src/plugin_runtime.rs',
  'crates/backend/nomifun-plugin-platform/src/application',
  'crates/backend/nomifun-plugin-platform/src/runtime',
  'crates/backend/nomifun-js-authoring',
  'crates/backend/nomifun-js-host',
  'crates/backend/nomifun-js-kernel-adapter',
  'crates/backend/nomifun-app/examples/configure_plugin_test_model.rs',
  'crates/backend/nomifun-app/src/router/plugin_platform.rs',
  'crates/backend/nomifun-app/src/router/plugin_runtime.rs',
  'crates/backend/nomifun-app/src/router/plugin_runtime_host.rs',
  'crates/backend/nomifun-app/src/router/plugin_product',
  'ui/src/common/types/pluginRuntimePlatform.ts',
  'ui/src/common/adapter/pluginRuntimeProductBridge.ts',
  'ui/src/common/adapter/ipcBridge.plugin-runtime-wire.test.ts',
  'ui/src/renderer/pages/plugins/runtime',
  'ui/src/renderer/pages/settings/JavaScriptRuntimeSettings.tsx',
  'ui/src/renderer/pages/settings/RuntimeManager',
  'ui/src/common/types/javascriptRuntime.ts',
  'ui/src/common/adapter/javascriptRuntimeBridge.ts',
  'scripts/gate-plugin-n1.mjs',
  'scripts/release/n1-cohort-evidence.mjs',
  'scripts/release/n1-cohort-evidence.test.mjs',
  'scripts/validation/run-windows-plugin-product-candidate.mjs',
  'scripts/validation/run-windows-plugin-runtime-candidate.mjs',
  'scripts/validation/run-windows-signed-rc-product.mjs',
  'scripts/validation/run-windows-signed-rc-product.test.mjs',
  'scripts/validation/linux-webkit-smoke.mjs',
  'docs/specs/2026-08-09-miniapps.zh.md',
  'docs/specs/2026-08-10-miniapps-v2-workspace.zh.md',
  'docs/specs/2026-08-10-miniapps-v3-unified-conversations.zh.md',
  'docs/specs/2026-08-28-agent-capability-platform-v2/06-phase-n1-plugin-miniapp-simplified-implementation-plan.zh.md',
  'docs/specs/2026-08-28-agent-capability-platform-v2/PHASE-N1-M1-CLOSURE-TODO.zh.md',
];

const RETIRED_TEXT = [
  { id: 'parallel-api', pattern: /\/api\/plugins\/(?:runtimes|projects|installations|operations|authoring)(?:\/|['"`]|$)/i },
  { id: 'parallel-domain-type', pattern: /\b(?:PluginProduct|PluginProject|PluginMount|ReadyCandidate|ReadyRelease|AutoApply|AutoPublish|PublishAuthorization)\b/ },
  { id: 'parallel-host', pattern: /\b(?:SharedExtensionHost|RuntimeBoundExtensionHost|CandidateTestHost|ServiceTestReceipt)\b/ },
  { id: 'stage-name', pattern: /\bplugin[_-]?(?:n1|m1)\b/i },
  { id: 'retired-entity-kind', pattern: /\bplugin[-_](?:project|mount|candidate|operation|runtime)\b/i },
  { id: 'persisted-surface-session', pattern: /\bpreview_session_id\b/i },
  { id: 'selectable-js-runtime', pattern: /\/settings\/javascript-runtime|\/api\/javascript-runtime\/(?:probe|switch|decision|download)/i },
];

const RETIRED_PLUGIN_TABLE = /\b(?:plugin_(?:build_operation_lineage|candidate_test_receipts|catalog_publications|deletion_intents|dependency_mutation_commits|dependency_mutation_intents|mount_credential_bindings|mount_kv|mount_revisions|product_documents|products|projects|publish_authorizations|ready_candidates|release_artifacts|releases|service_test_receipts|source_mutation_commits|source_mutation_intents|surface_sessions)|product_operations|javascript_runtime_selection)\b/i;

const EXPECTED_CORE_TABLES = [
  'plugin_artifacts',
  'plugin_credential_bindings',
  'plugin_drafts',
  'plugin_grants',
  'plugin_library_state',
  'plugin_mutations',
  'plugins',
];

const HISTORICAL_PLUGIN_DOCS = [
  'docs/specs/2026-08-28-agent-capability-platform-v2/02-capability-catalog-and-agent-presets.zh.md',
  'docs/specs/2026-08-28-agent-capability-platform-v2/03-target-architecture.zh.md',
  'docs/specs/2026-08-28-agent-capability-platform-v2/05-system-capability-replacement-foundation.zh.md',
  'docs/specs/2026-09-06-coding-agent-runtime-internalization/README.zh.md',
  'docs/specs/2026-09-06-coding-agent-runtime-internalization/CODING-EXTENSIONS-2026-09-13.zh.md',
  'docs/reviews/audit-progress.zh.md',
  'docs/reviews/2026-09-12-runtime-protocol.zh.md',
  'docs/reviews/2026-09-12-cleanup-proof.zh.md',
  'docs/reviews/2026-09-15-agent-session-view-retirement.zh.md',
];

const normalize = (path) => path.replaceAll('\\', '/');

function workspacePaths() {
  const output = execFileSync(
    'git',
    ['ls-files', '--cached', '--others', '--exclude-standard', '-z', '--',
      'package.json', '.github', 'scripts', 'crates/backend', 'ui/src'],
    { cwd: ROOT, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 },
  );
  return [...new Set(output.split('\0').filter(Boolean).map(normalize))];
}

function isTest(path) {
  return /(?:^|\/)(?:tests?|fixtures)(?:\/|$)|\.(?:test|spec)\.[^.]+$/.test(path);
}

function isPluginBoundarySource(path) {
  if (path === 'package.json' || path.startsWith('.github/')) return true;
  if (path.startsWith('scripts/')) return !isTest(path);
  if (path.startsWith('crates/backend/nomifun-plugin-platform/src/')) return true;
  if (path === 'crates/backend/nomifun-api-types/src/plugin_platform.rs') return true;
  if (path === 'crates/backend/nomifun-agent-contracts/src/plugin.rs') return true;
  if (path.startsWith('crates/backend/nomifun-app/src/router/')) {
    const name = path.split('/').at(-1) ?? '';
    return name === 'routes.rs' || name === 'state.rs' || name === 'trace.rs' || name.startsWith('plugin');
  }
  if (path.startsWith('ui/src/')) {
    return path.includes('/plugins/') ||
      path.endsWith('/pluginPlatform.ts') ||
      path.endsWith('/pluginPlatformBridge.ts') ||
      path.endsWith('/ipcBridge.ts') ||
      path.endsWith('/ids.ts') ||
      path.endsWith('/Router.tsx') ||
      path.includes('/RuntimeManager/') ||
      path.endsWith('/JavaScriptRuntimeSettings.tsx');
  }
  return false;
}

function pluginTables(source) {
  return [...source.matchAll(/^CREATE TABLE\s+["']?(plugins|plugin_[a-z0-9_]+)["']?\s*\(/gim)]
    .map((match) => match[1])
    .sort();
}

function containsFilesystemEntry(path) {
  if (!existsSync(path)) return false;
  const metadata = lstatSync(path);
  if (!metadata.isDirectory()) return true;
  return readdirSync(path, { withFileTypes: true }).some((entry) =>
    entry.isFile() || entry.isSymbolicLink() ||
    entry.isDirectory() && containsFilesystemEntry(resolve(path, entry.name)));
}

function textViolations(path, source) {
  if (path === SELF || isTest(path) || !isPluginBoundarySource(path) || !SOURCE_EXTENSIONS.has(extname(path))) {
    return [];
  }
  return source.split(/\r?\n/).flatMap((line, index) =>
    RETIRED_TEXT.filter(({ pattern }) => pattern.test(line)).map(({ id }) =>
      `${path}:${index + 1}: ${id}: ${line.trim()}`));
}

export function auditUnifiedPluginBoundary(paths = workspacePaths()) {
  const failures = [];
  for (const path of CANONICAL_FILES) {
    if (!existsSync(resolve(ROOT, path))) failures.push(`missing canonical file: ${path}`);
  }
  for (const path of RETIRED_FILES) {
    if (containsFilesystemEntry(resolve(ROOT, path))) failures.push(`retired file still exists: ${path}`);
  }
  for (const path of paths) {
    const absolute = resolve(ROOT, path);
    if (!existsSync(absolute)) continue;
    const source = readFileSync(absolute, 'utf8');
    failures.push(...textViolations(path, source));
    if (
      path.startsWith('crates/backend/') &&
      path !== 'crates/backend/nomifun-db/src/database.rs' &&
      SOURCE_EXTENSIONS.has(extname(path))
    ) {
      source.split(/\r?\n/).forEach((line, index) => {
        if (RETIRED_PLUGIN_TABLE.test(line)) {
          failures.push(`${path}:${index + 1}: retired-plugin-table: ${line.trim()}`);
        }
      });
    }
  }

  for (const path of HISTORICAL_PLUGIN_DOCS) {
    const source = readFileSync(resolve(ROOT, path), 'utf8');
    if (!/(?:归档边界|Plugin 退役边界|Plugin 条款已由 Unified Plugin Core 取代)/.test(source)) {
      failures.push(`historical Plugin document is not explicitly archived: ${path}`);
    }
  }

  const baselinePath = resolve(ROOT, 'crates/backend/nomifun-db/migrations/001_canonical_baseline.sql');
  if (!existsSync(baselinePath)) failures.push('missing canonical database baseline');
  else {
    const baseline = readFileSync(baselinePath, 'utf8');
    const observed = pluginTables(baseline);
    if (JSON.stringify(observed) !== JSON.stringify(EXPECTED_CORE_TABLES)) {
      failures.push(`canonical Plugin tables differ: ${observed.join(', ')}`);
    }
    if (/\b(?:preview|surface)_session_id\b/i.test(baseline)) {
      failures.push('canonical Plugin schema persists a runtime-only Surface session');
    }
  }

  const packageJson = JSON.parse(readFileSync(resolve(ROOT, 'package.json'), 'utf8'));
  if (packageJson.scripts?.['test:plugin-sdk'] !==
    'node --test crates/backend/nomifun-plugin-platform/tests/plugin_sdk.test.mjs') {
    failures.push('test:plugin-sdk must run the Unified Plugin SDK contract');
  }
  if (packageJson.scripts?.['check:unified-plugin-boundary'] !==
    'bun scripts/check-unified-plugin-boundary.mjs') {
    failures.push('check:unified-plugin-boundary is not registered');
  }

  const router = readFileSync(resolve(ROOT, 'crates/backend/nomifun-app/src/router/plugin.rs'), 'utf8');
  if (!router.includes('/api/plugin-drafts') || !router.includes('/api/plugins')) {
    failures.push('Plugin router must expose the two canonical resource roots');
  }
  const bridge = readFileSync(resolve(ROOT, 'ui/src/common/adapter/pluginPlatformBridge.ts'), 'utf8');
  if ((bridge.match(/export const pluginPlatform/g) ?? []).length !== 1) {
    failures.push('frontend must export exactly one pluginPlatform Bridge');
  }

  const apiInventory = readFileSync(
    resolve(ROOT, 'crates/backend/nomifun-agent-contracts/contracts/presets/canonical-api-inventory.payload.json'),
    'utf8',
  );
  if (/\/api\/plugins\/installations|plugin_mounts\./i.test(apiInventory)) {
    failures.push('canonical API inventory still exposes the retired Plugin Mount API');
  }
  const primitives = readFileSync(
    resolve(ROOT, 'crates/backend/nomifun-agent-contracts/src/primitives.rs'),
    'utf8',
  );
  if (/CandidateTestReceiptId/.test(primitives)) {
    failures.push('retired CandidateTestReceiptId still exists');
  }
  const agentStoreSchema = readFileSync(
    resolve(ROOT, 'crates/backend/nomifun-agent-contracts/schema/0001_agent_store.sql'),
    'utf8',
  );
  if (/idx_agent_presets_ui_plugin|ui_binding\.selection\.plugin_id/.test(agentStoreSchema)) {
    failures.push('Agent Store schema still indexes the retired Plugin UI graph');
  }
  const agentApi = readFileSync(
    resolve(ROOT, 'crates/backend/nomifun-api-types/src/agent_platform.rs'),
    'utf8',
  );
  if (/\bpub\s+plugin_id\s*:/.test(agentApi)) {
    failures.push('Agent provenance DTO still carries the retired Plugin graph identity');
  }
  const agentUiGraph = [
    'ui/src/renderer/components/agent/AgentResourcePicker.tsx',
    'ui/src/renderer/hooks/agent/agentResourceSelection.ts',
    'ui/src/renderer/pages/agentSettings/model.ts',
    'ui/src/renderer/pages/agentSettings/capabilityGroups.ts',
    'ui/src/renderer/pages/agentSettings/AgentCapabilityWorkspace.tsx',
    'ui/src/renderer/pages/plugins/PluginRunPage.tsx',
  ].map((path) => readFileSync(resolve(ROOT, path), 'utf8')).join('\n');
  if (/plugin\.development|plugin\.surface|source=plugin|setPluginsOnly|wanted\.has\(['"]plugin['"]\)|kind\s*===\s*['"]plugin['"]/.test(agentUiGraph)) {
    failures.push('Agent authoring UI still routes Plugin through the retired resource/capability graph');
  }

  return failures;
}

export function assertSelfTest() {
  const oldApi = textViolations(
    'ui/src/common/adapter/pluginPlatformBridge.ts',
    "const path = '/api/plugins/runtimes';",
  );
  if (oldApi.length !== 1) throw new Error('retired API self-test did not fail closed');
  const tables = pluginTables('CREATE TABLE plugins (x);\nCREATE TABLE plugin_drafts (x);');
  if (JSON.stringify(tables) !== JSON.stringify(['plugin_drafts', 'plugins'])) {
    throw new Error('Plugin table parser self-test failed');
  }
  return { status: 'pass', checks: ['retired-api', 'table-parser'] };
}

const args = process.argv.slice(2);
if (args.length === 1 && args[0] === '--self-test') {
  console.log(JSON.stringify(assertSelfTest()));
} else if (args.length === 0) {
  const failures = auditUnifiedPluginBoundary();
  if (failures.length) {
    console.error(`Unified Plugin boundary check failed (${failures.length} issue(s))`);
    for (const failure of failures) console.error(`  - ${failure}`);
    process.exit(1);
  }
  console.log('Unified Plugin boundary check passed');
} else {
  console.error('usage: bun scripts/check-unified-plugin-boundary.mjs [--self-test]');
  process.exit(2);
}
