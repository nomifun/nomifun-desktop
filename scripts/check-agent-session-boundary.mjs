#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { rustProductionText } from './check-uarc-boundary.mjs';
import { AGENT_FORWARD_MIGRATIONS, areKnownAgentMigrationPaths, officeHubMigrationErrors, officeCompactMigrationErrors, feishuConnectorRenameMigrationErrors } from './lib/agent-migration-boundary.mjs';

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
const { baseline, pluginForwardMigration, metadataForwardMigration, hostingForwardMigration,
  policyForwardMigration, uploadForwardMigration, terminalForwardMigration, payloadForwardMigration,
  cronInputForwardMigration, mcpOAuthForwardMigration, cronCancellationForwardMigration,
  cronWorkbenchForwardMigration, modelInputConstraintsForwardMigration, queueSettingForwardMigration,
  executionPlanGateForwardMigration, officeHubForwardMigration, officeCompactForwardMigration,
  officeActivityRetirementForwardMigration, officeOverviewRetirementForwardMigration,
  feishuSourceKindForwardMigration, feishuConnectorTablesForwardMigration } = AGENT_FORWARD_MIGRATIONS;
if (!areKnownAgentMigrationPaths(migrations)) {
  failures.push('Agent clean cut requires its canonical baseline and only explicitly supported forward migrations');
}
const migrationStatements = (path) => readFileSync(resolve(ROOT, path), 'utf8')
  .replace(/--[^\r\n]*/g, '').split(';').map((statement) => statement.replace(/\s+/g, ' ').trim()).filter(Boolean);
if (migrations.includes(officeHubForwardMigration)) {
  failures.push(...officeHubMigrationErrors(readFileSync(resolve(ROOT, officeHubForwardMigration), 'utf8')));
}
if (migrations.includes(officeCompactForwardMigration)) {
  failures.push(...officeCompactMigrationErrors(readFileSync(resolve(ROOT, officeCompactForwardMigration), 'utf8')));
}
if (migrations.includes(feishuConnectorTablesForwardMigration)) {
  failures.push(...feishuConnectorRenameMigrationErrors(readFileSync(resolve(ROOT, feishuConnectorTablesForwardMigration), 'utf8')));
}
if (migrations.includes(officeActivityRetirementForwardMigration)
  && JSON.stringify(migrationStatements(officeActivityRetirementForwardMigration)) !== JSON.stringify([
    'DROP INDEX idx_office_operations_owner_history',
    'ALTER TABLE office_operations DROP COLUMN created_at',
    'ALTER TABLE office_operations DROP COLUMN updated_at',
  ])) {
  failures.push('Office activity retirement may only drop history ordering and timestamps; request identities, receipts, pending targets and Agent facts stay unchanged');
}
if (migrations.includes(executionPlanGateForwardMigration)
  && JSON.stringify(migrationStatements(executionPlanGateForwardMigration)) !== JSON.stringify([
    'ALTER TABLE agent_executions DROP COLUMN plan_gate',
  ])) {
  failures.push('Execution plan gate retirement may only drop its unused column; identities, policies, canonical Agent facts and lineage receipts stay unchanged');
}
if (migrations.includes(feishuSourceKindForwardMigration)) {
  const allowed = [
    /^CREATE TABLE knowledge_sources_next \(/,
    /^INSERT INTO knowledge_sources_next \(/,
    /^DROP TABLE knowledge_sources$/,
    /^ALTER TABLE knowledge_sources_next RENAME TO knowledge_sources$/,
    /^CREATE (?:UNIQUE )?INDEX (?:idx_knowledge_sources_default_parent_entry_id|idx_knowledge_sources_knowledge_base_id|uq_knowledge_sources_live_kind) ON knowledge_sources\b/,
    /^CREATE TEMP TABLE feishu_source_integrity\b/,
    /^INSERT INTO feishu_source_integrity SELECT COUNT\(\*\) FROM pragma_foreign_key_check$/,
    /^DROP TABLE feishu_source_integrity$/,
    /^PRAGMA legacy_alter_table = OFF$/,
  ];
  for (const statement of migrationStatements(feishuSourceKindForwardMigration)) {
    if (!allowed.some(rule => rule.test(statement))) {
      failures.push(`${feishuSourceKindForwardMigration}: may only widen the knowledge_sources kind constraint; canonical Agent facts and other tables stay unchanged (${statement.slice(0, 100)})`);
    }
  }
}
if (migrations.includes(officeOverviewRetirementForwardMigration)
  && JSON.stringify(migrationStatements(officeOverviewRetirementForwardMigration)) !== JSON.stringify([
    'ALTER TABLE office_connections DROP COLUMN recent_resources_json',
  ])) {
  failures.push('Office overview retirement may only drop its recent-resource cache; connection settings, requests, receipts and Agent facts stay unchanged');
}
if (migrations.includes(queueSettingForwardMigration)
  && JSON.stringify(migrationStatements(queueSettingForwardMigration)) !== JSON.stringify([
    'ALTER TABLE system_settings DROP COLUMN command_queue_enabled',
  ])) {
  failures.push('Queue setting retirement may only drop its unused system setting column; other configuration, canonical Agent facts and lineage receipts stay unchanged');
}
if (migrations.includes(modelInputConstraintsForwardMigration)
  && JSON.stringify(migrationStatements(modelInputConstraintsForwardMigration)) !== JSON.stringify([
    'ALTER TABLE provider_model_capabilities ADD COLUMN input_constraints TEXT',
  ])) {
  failures.push('Model input constraints migration may only add nullable provider configuration; canonical Agent facts and schema metadata stay unchanged');
}
if (migrations.includes(cronWorkbenchForwardMigration)) {
  const allowed = [
    /^CREATE TEMP TABLE cron_binding_(?:json_proof|sequence)\b/,
    /^INSERT INTO cron_binding_json_proof\b/,
    /^DROP TABLE cron_binding_(?:json_proof|sequence)$/,
    /^ALTER TABLE cron_jobs ADD COLUMN (?:agent_selection|model_selection|agent_binding|clear_context_each_run|configuration_error|last_run_id)\b/,
    /^UPDATE cron_jobs\b/,
    /^CREATE TABLE cron_jobs_binding_next\b/,
    /^INSERT INTO cron_jobs_binding_next\b/,
    /^DROP TABLE cron_jobs$/,
    /^ALTER TABLE cron_jobs_binding_next RENAME TO cron_jobs$/,
    /^UPDATE sqlite_sequence SET seq=MAX\(seq,COALESCE\(\(SELECT seq FROM cron_binding_sequence\),0\)\) WHERE name='cron_jobs'$/,
    /^CREATE INDEX idx_cron_jobs_(?:next_run|owner_conversation) ON cron_jobs\b/,
    /^ALTER TABLE cron_run_reservations ADD COLUMN domain_failure_json\b/,
    /^DROP TABLE cron_job_runs$/,
    /^DROP INDEX idx_cron_run_reservations_cron_job_id$/,
    /^CREATE INDEX idx_cron_run_reservations_cron_job_id ON cron_run_reservations\b/,
  ];
  for (const statement of migrationStatements(cronWorkbenchForwardMigration)) {
    if (!allowed.some(rule => rule.test(statement))) {
      failures.push(`${cronWorkbenchForwardMigration}: may only normalize the existing Cron definition/history; canonical Agent facts are read-only (${statement.slice(0, 100)})`);
    }
  }
}
if (migrations.includes(cronCancellationForwardMigration)
  && JSON.stringify(migrationStatements(cronCancellationForwardMigration)) !== JSON.stringify([
    'ALTER TABLE cron_run_reservations ADD COLUMN cancel_requested_at_ms INTEGER',
  ])) {
  failures.push('Cron cancellation migration may only add the original reservation cancellation fence');
}
if (migrations.includes(mcpOAuthForwardMigration)) {
  const statements = migrationStatements(mcpOAuthForwardMigration);
  const allowed = [
    'CREATE TEMP TABLE mcp_oauth_metadata_guard ',
    'INSERT INTO mcp_oauth_metadata_guard SELECT ',
    'DROP TABLE mcp_oauth_metadata_guard',
    'DROP TABLE oauth_tokens',
    'CREATE TABLE mcp_oauth_clients ',
    'CREATE INDEX idx_mcp_oauth_clients_profile ON mcp_oauth_clients(',
    'CREATE TABLE mcp_oauth_grants ',
    'CREATE INDEX idx_mcp_oauth_grants_client ON mcp_oauth_grants(',
    'UPDATE schema_metadata SET canonical_schema_manifest_digest=',
    'CREATE TEMP TABLE mcp_oauth_integrity ',
    'INSERT INTO mcp_oauth_integrity SELECT ',
    'DROP TABLE mcp_oauth_integrity',
  ];
  const manifest = JSON.parse(readFileSync(resolve(ROOT,
    'crates/backend/nomifun-agent-contracts/contracts/generated/agent-store-migration-manifest.envelope.json'), 'utf8'));
  // Migration 010's declared target is the post-010 manifest lineage (before
  // product tables landed), not the ever-advancing current manifest digest.
  const OAUTH_TARGET_DIGEST = 'ce5cb186b9ba1a44c7c4699f4e1932542068537149c2a76ec880e2b8cc0bf8d9';
  const digestUpdate = statements[8] ?? '';
  const exactDigestTransition = /^UPDATE schema_metadata SET canonical_schema_manifest_digest='[a-f0-9]{64}' WHERE singleton_key='canonical' AND data_generation=7 AND migration_head=1 AND projection_schema_version=1 AND canonical_schema_manifest_digest='[a-f0-9]{64}'$/;
  if (statements.length !== allowed.length
    || statements.some((statement, index) => !statement.startsWith(allowed[index]))
    || !exactDigestTransition.test(digestUpdate)
    || !digestUpdate.startsWith(`UPDATE schema_metadata SET canonical_schema_manifest_digest='${OAUTH_TARGET_DIGEST}' `)
    || !['mcp_oauth_clients', 'mcp_oauth_grants'].every(name => manifest.payload.tables.some(table =>
      table.table_name === name && table.reset_scope === 'preserve'))) {
    failures.push(`${mcpOAuthForwardMigration}: retire only unbound OAuth credentials, create the two preserved OAuth tables, and advance only the known non-Agent manifest digest; preserve Agent facts and other configuration`);
  }
}
if (migrations.includes(cronInputForwardMigration)) {
  const original = migrationStatements(baseline);
  const tables = ['cron_job_runs', 'cron_jobs', 'cron_run_reservations'];
  const expected = [
    'CREATE TEMP TABLE cron_wait_sequences (name TEXT PRIMARY KEY, seq INTEGER NOT NULL)',
    "INSERT INTO cron_wait_sequences SELECT name, seq FROM sqlite_sequence WHERE name IN ('cron_job_runs', 'cron_jobs', 'cron_run_reservations')",
  ];
  for (const table of tables) {
    const create = original.find(statement => statement.startsWith(`CREATE TABLE ${table} (`));
    expected.push(create?.replace(`CREATE TABLE ${table} (`, `CREATE TABLE ${table}_next (`)
      .replaceAll("'missed')", "'missed', 'input_required', 'cancelled')"),
      `INSERT INTO ${table}_next SELECT * FROM ${table}`,
      `DROP TABLE ${table}`,
      `ALTER TABLE ${table}_next RENAME TO ${table}`);
  }
  expected.push(...original.filter(statement => /^CREATE (?:UNIQUE )?INDEX /.test(statement)
    && tables.some(table => statement.includes(` ON ${table}(`))),
    "UPDATE sqlite_sequence SET seq = MAX(seq, COALESCE((SELECT old.seq FROM cron_wait_sequences old WHERE old.name = sqlite_sequence.name), 0)) WHERE name IN ('cron_job_runs', 'cron_jobs', 'cron_run_reservations')",
    'DROP TABLE cron_wait_sequences',
    'CREATE TEMP TABLE cron_wait_integrity (valid INTEGER NOT NULL CHECK(valid = 1))',
    'INSERT INTO cron_wait_integrity SELECT CASE WHEN COUNT(*) = 0 THEN 1 ELSE 0 END FROM pragma_foreign_key_check',
    'DROP TABLE cron_wait_integrity');
  if (JSON.stringify(migrationStatements(cronInputForwardMigration)) !== JSON.stringify(expected)) {
    failures.push(`${cronInputForwardMigration}: only extend existing Cron status constraints; preserve all columns, rows, indexes, sequence high-water marks and canonical Agent facts`);
  }
}
if (migrations.includes(terminalForwardMigration)) {
  const statements = migrationStatements(terminalForwardMigration);
  const drops = [
    'ALTER TABLE agent_turns DROP COLUMN result_json',
    'ALTER TABLE agent_turns DROP COLUMN error_json',
    'ALTER TABLE agent_effects DROP COLUMN bounded_observation_json',
  ];
  const checks = [statements[1], statements[2]];
  if (statements.length !== 7
    || statements[0] !== 'CREATE TEMP TABLE terminal_content_integrity ( valid INTEGER NOT NULL CHECK (valid = 1) )'
    || JSON.stringify(statements.slice(3, 6)) !== JSON.stringify(drops)
    || statements[6] !== 'DROP TABLE terminal_content_integrity'
    || checks.some((statement) => !statement?.startsWith('INSERT INTO terminal_content_integrity SELECT CASE WHEN ')
      || /\b(?:UPDATE|DELETE|ALTER|DROP|CREATE|REPLACE)\b/i.test(statement))
    || !statements[1]?.endsWith("FROM agent_turns WHERE state IN ('completed', 'failed', 'cancelled', 'interrupted')")
    || !statements[2]?.endsWith("FROM agent_effects WHERE state <> 'pending'")
    || !statements[1]?.includes('event.session_id = agent_turns.session_id')
    || !statements[1]?.includes('event.seq = agent_turns.finished_at')
    || !statements[2]?.includes('event.session_id = agent_effects.session_id')
    || !statements[2]?.includes("WHEN 'effect/reconciled' THEN CASE COALESCE(")) {
    failures.push(`${terminalForwardMigration}: validate exact canonical terminal references before dropping only redundant copies; facts and lineage must remain unchanged`);
  }
}
if (migrations.includes(payloadForwardMigration)) {
  const expected = [
    'CREATE TEMP TABLE payload_storage_integrity ( valid INTEGER NOT NULL CHECK (valid = 1) )',
    "INSERT INTO payload_storage_integrity SELECT CASE WHEN storage_kind = 'inline' AND object_ref IS NULL AND body IS NOT NULL THEN 1 ELSE 0 END FROM agent_payloads",
    'CREATE TABLE agent_payloads_next ( payload_id TEXT PRIMARY KEY, session_id TEXT NOT NULL, media_type TEXT NOT NULL, byte_len INTEGER NOT NULL CHECK (byte_len >= 0), digest TEXT NOT NULL CHECK (length(digest) = 64), body BLOB NOT NULL, FOREIGN KEY (session_id) REFERENCES agent_sessions (agent_session_id) ON UPDATE RESTRICT ON DELETE CASCADE ) STRICT',
    'INSERT INTO agent_payloads_next (payload_id, session_id, media_type, byte_len, digest, body) SELECT payload_id, session_id, media_type, byte_len, digest, body FROM agent_payloads',
    'DROP TABLE agent_payloads',
    'ALTER TABLE agent_payloads_next RENAME TO agent_payloads',
    'CREATE INDEX idx_agent_payloads_session ON agent_payloads(session_id)',
    'INSERT INTO payload_storage_integrity SELECT CASE WHEN COUNT(*) = 0 THEN 1 ELSE 0 END FROM pragma_foreign_key_check',
    'DROP TABLE payload_storage_integrity',
  ];
  if (JSON.stringify(migrationStatements(payloadForwardMigration)) !== JSON.stringify(expected)) {
    failures.push(`${payloadForwardMigration}: preserve every payload identity and body byte and validate references; unreadable object rows must fail closed`);
  }
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
