#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { rustProductionText } from './check-uarc-boundary.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const retiredPrefixes = [
  'docs/specs/2026-08-28-agent-capability-platform-v2/',
  'docs/specs/2026-09-06-coding-agent-runtime-internalization/',
  'docs/specs/2026-09-16-unified-agent-overhaul/',
  'crates/backend/nomifun-agent-contracts/contracts/historical/',
];
const retiredFiles = new Set([
  'crates/backend/nomifun-agent-execution/src/artifact_contract.rs',
  'ui/src/renderer/creation/legacyDraftImport.ts',
  'ui/src/renderer/pages/conversation/Messages/planToolVisibility.ts',
  'ui/src/renderer/pages/conversation/platforms/nomi/localCronCommands.ts',
  'ui/src/renderer/pages/conversation/platforms/nomi/nomiMessageBuffer.ts',
  'ui/src/renderer/pages/conversation/platforms/nomi/nomiPostProcessState.ts',
  'crates/backend/nomifun-agent-contracts/src/deletion.rs',
  'crates/backend/nomifun-agent-contracts/schema/0001_agent_store.sql',
  'crates/backend/nomifun-agent-contracts/contracts/session/checkpoint-contract.json',
  'crates/backend/nomifun-agent-contracts/contracts/runtime/checkpoint-mismatch.json',
  'crates/backend/nomifun-agent-contracts/contracts/events/runtime-event-envelope.json',
  'crates/backend/nomifun-agent-contracts/contracts/events/runtime-event-ack.json',
  'crates/backend/nomifun-agent-contracts/contracts/inventory/service-key-target-map.json',
  'docs/specs/2026-09-25-agent-reliability/DEVELOPMENT-HANDOFF.zh.md',
  'docs/reviews/2026-09-23-agent-session-reliability.zh.md',
  'docs/specs/2026-08-23-creative-studio-canvas-domain-redesign.zh.md',
  'docs/specs/2026-09-14-companion-unified-conversation.zh.md',
  'docs/specs/2026-09-14-desktop-conversation-creation-redesign.zh.md',
  'crates/backend/nomifun-agent-contracts/contracts/validation/d025-compatibility-fixture-reference.payload.json',
  'crates/backend/nomifun-agent-contracts/contracts/validation/d025-fixture-envelope-reference.json',
  'crates/backend/nomifun-agent-contracts/contracts/validation/d025-compatibility-fixture-reference.envelope.json',
]);
const rules = [
  { roots: ['crates/backend/nomifun-agent-execution/src/'], pattern: /\b(?:MessageResponse|MessageListResponse|ListMessagesQuery|TurnArtifactProjection|artifact_delivery_committed|validate_required_artifacts|requires_artifact_delivery)\b/, reason: 'Execution output and recovery must consume exact canonical Turn facts, not UI or inferred artifact contracts' },
  { roots: ['crates/backend/nomifun-app/src/router/engine_history.rs', 'crates/backend/nomifun-app/src/router/unified_runtime_history.rs', 'crates/backend/nomifun-app/src/router/runtime_history_port.rs', 'crates/backend/nomifun-agent-session/src/context_snapshot.rs'], pattern: /\b(?:MessageProjection|read_message_history|message_history_before|messages_before)\b/, reason: 'Model context cannot consume UI projection readers' },
  { roots: ['crates/backend/nomifun-app/src/router/'], pattern: /\b(?:EngineHistoryMessage|EngineMessageHistoryWindow|read_message_history_before_turn|project_messages)\b/, reason: 'Runtime history must read typed canonical events' },
  { roots: ['crates/backend/nomifun-agent-session/src/', 'crates/backend/nomifun-db/migrations/'], pattern: /\breasoning_effort_v\d+\b/, reason: 'Session reasoning has one native source' },
  { roots: ['crates/backend/nomifun-agent-session/src/', 'crates/backend/nomifun-agent-contracts/src/'], pattern: /\b(?:RuntimeCheckpointBinding|RuntimeCheckpointValidationInput|RuntimeCheckpointValidationResult|CheckpointAdmission|RuntimeAppendContext|RuntimeEventEnvelope|runtime_producer_seq|runtime_bound_event_id)\b/, reason: 'Unused generic Runtime binding and checkpoint chain is retired' },
  { roots: ['ui/src/renderer/', 'ui/src/common/'], pattern: /\b(?:final_text_authoritative|finalTextAuthoritative|processLocalCronResponse|NomiPostProcessState|importLegacyWorkbenchDraft)\b/, reason: 'Renderer must consume the current stream contract' },
];

const paths = [...new Set(execFileSync('git', [
  'ls-files', '--cached', '--others', '--exclude-standard', '-z', '--',
  'crates/backend', 'ui/src', 'scripts', 'docs',
], { cwd: ROOT, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 }).split('\0'))]
  .filter((path) => path && existsSync(resolve(ROOT, path)));
const failures = [];
for (const path of paths) {
  if (retiredFiles.has(path) || retiredPrefixes.some((prefix) => path.startsWith(prefix))) {
    failures.push(`${path}: retired source or design document must be deleted`);
    continue;
  }
  if (!/\.(?:rs|sql|ts|tsx|md)$/.test(path)) continue;
  const source = readFileSync(resolve(ROOT, path), 'utf8');
  if (path.endsWith('.md')) {
    for (const prefix of retiredPrefixes) {
      if (source.includes(prefix.replace(/^docs\//, '')) && /\]\([^)]*2026-(?:08-28-agent-capability-platform-v2|09-06-coding-agent-runtime-internalization|09-16-unified-agent-overhaul)/.test(source)) {
        failures.push(`${path}: links to a deleted design document`);
      }
    }
    continue;
  }
  if (path.includes('/tests/') || /(?:\.test\.[^.]+|_tests\.rs|\/tests\.rs)$/.test(path)) continue;
  const production = path.endsWith('.rs') ? rustProductionText(source) : source;
  for (const rule of rules) {
    if (rule.roots.some((prefix) => path.startsWith(prefix)) && rule.pattern.test(production)) {
      failures.push(`${path}: ${rule.reason}`);
    }
  }
}

const migrations = paths.filter((path) => path.startsWith('crates/backend/nomifun-db/migrations/') && path.endsWith('.sql'));
const baseline = 'crates/backend/nomifun-db/migrations/001_canonical_baseline.sql';
const pluginForwardMigration = 'crates/backend/nomifun-db/migrations/002_simplify_plugin_library.sql';
const metadataForwardMigration = 'crates/backend/nomifun-db/migrations/003_remove_agent_handshake_cache.sql';
const hostingForwardMigration = 'crates/backend/nomifun-db/migrations/004_retire_plugin_hosting.sql';
const policyForwardMigration = 'crates/backend/nomifun-db/migrations/005_retire_side_question_policy.sql';
const uploadForwardMigration = 'crates/backend/nomifun-db/migrations/006_retire_upload_workspace_preference.sql';
if (!migrations.includes(baseline) || migrations.some((path) => ![baseline, pluginForwardMigration, metadataForwardMigration, hostingForwardMigration, policyForwardMigration, uploadForwardMigration].includes(path))) {
  failures.push('Agent clean cut requires its canonical baseline and only explicitly supported forward product migrations');
}
if (migrations.includes(uploadForwardMigration)) {
  const source = readFileSync(resolve(ROOT, uploadForwardMigration), 'utf8');
  const statements = source.replace(/--[^\r\n]*/g, '').split(';').map((statement) => statement.replace(/\s+/g, ' ').trim()).filter(Boolean);
  const expected = [
    'ALTER TABLE system_settings DROP COLUMN save_upload_to_workspace',
    "DELETE FROM client_preferences WHERE key IN ('upload.saveToWorkspace', 'saveUploadToWorkspace')",
  ];
  if (JSON.stringify(statements) !== JSON.stringify(expected)) {
    failures.push(`${uploadForwardMigration}: only the unused upload setting and its exact preference keys may be removed; canonical facts, other settings and lineage receipts cannot change`);
  }
}
if (migrations.includes(policyForwardMigration)) {
  const source = readFileSync(resolve(ROOT, policyForwardMigration), 'utf8').replace(/--[^\r\n]*/g, '');
  const statements = source.split(';').map((statement) => statement.replace(/\s+/g, ' ').trim()).filter(Boolean);
  const integrity = 'retired_side_question_policy_integrity';
  const update = "UPDATE agent_metadata SET behavior_policy = json_remove(behavior_policy, '$.supports_side_question') WHERE behavior_policy IS NOT NULL AND json_type(behavior_policy, '$.supports_side_question') IS NOT NULL";
  const checks = [statements[1], statements[3]];
  if (statements.length !== 5
    || statements[0] !== `CREATE TEMP TABLE ${integrity} ( valid INTEGER NOT NULL CHECK (valid = 1) )`
    || statements[2] !== update || statements[4] !== `DROP TABLE ${integrity}`
    || checks.some((statement) => !statement?.startsWith(`INSERT INTO ${integrity} SELECT CASE WHEN `)
      || !statement.endsWith('FROM agent_metadata WHERE behavior_policy IS NOT NULL')
      || /\b(?:UPDATE|DELETE|ALTER|DROP|CREATE|REPLACE)\b/i.test(statement))
    || !statements[1]?.includes("json_valid(behavior_policy) = 1")
    || !statements[1]?.includes("json_type(behavior_policy) = 'object'")
    || /\b(?:agent_(?!metadata\b)\w+|schema_metadata|_sqlx_migrations)\b/i.test(source)) {
    failures.push(`${policyForwardMigration}: only the exact retired metadata key may be removed after fail-closed JSON checks; canonical facts, other settings and lineage receipts cannot change`);
  }
}
if (migrations.includes(metadataForwardMigration)) {
  const source = readFileSync(resolve(ROOT, metadataForwardMigration), 'utf8');
  const statements = source.replace(/--[^\r\n]*/g, '').split(';').map((statement) => statement.trim()).filter(Boolean);
  const expected = [
    'yolo_id', 'agent_capabilities', 'auth_methods', 'config_options',
    'available_modes', 'available_models', 'available_commands',
  ].map((column) => `ALTER TABLE agent_metadata DROP COLUMN ${column}`);
  if (JSON.stringify(statements) !== JSON.stringify(expected)) {
    failures.push(`${metadataForwardMigration}: only the exact retired configuration cache columns may be dropped; canonical Agent facts and lineage receipts cannot change`);
  }
}
for (const migration of [pluginForwardMigration, hostingForwardMigration].filter(path => migrations.includes(path))) {
  const source = readFileSync(resolve(ROOT, migration), 'utf8');
  if (/\b(?:agent_\w+|schema_metadata|_sqlx_migrations)\b/i.test(source)) {
    failures.push(`${migration}: a hosting forward migration cannot alter canonical Agent data or lineage receipts`);
  }
}
if (failures.length) {
  console.error(`Agent Session boundary failed (${failures.length}):\n${failures.map((message) => `  - ${message}`).join('\n')}`);
  process.exitCode = 1;
} else {
  console.log('Agent Session boundary passed: canonical event history, native reasoning, current stream and immutable baseline.');
}
