// Published forward migrations are explicit paths, never a numeric range or a
// filename pattern that could authorize a future unknown migration.
export const AGENT_FORWARD_MIGRATIONS = Object.freeze({
  baseline: 'crates/backend/nomifun-db/migrations/001_canonical_baseline.sql',
  pluginForwardMigration: 'crates/backend/nomifun-db/migrations/002_simplify_plugin_library.sql',
  metadataForwardMigration: 'crates/backend/nomifun-db/migrations/003_remove_agent_handshake_cache.sql',
  hostingForwardMigration: 'crates/backend/nomifun-db/migrations/004_retire_plugin_hosting.sql',
  policyForwardMigration: 'crates/backend/nomifun-db/migrations/005_retire_side_question_policy.sql',
  uploadForwardMigration: 'crates/backend/nomifun-db/migrations/006_retire_upload_workspace_preference.sql',
  terminalForwardMigration: 'crates/backend/nomifun-db/migrations/007_retire_terminal_content_mirrors.sql',
  payloadForwardMigration: 'crates/backend/nomifun-db/migrations/008_retire_payload_object_placeholder.sql',
  cronInputForwardMigration: 'crates/backend/nomifun-db/migrations/009_cron_input_handoff.sql',
  mcpOAuthForwardMigration: 'crates/backend/nomifun-db/migrations/010_mcp_oauth_authority.sql',
  cronCancellationForwardMigration: 'crates/backend/nomifun-db/migrations/011_cron_run_cancellation.sql',
  cronWorkbenchForwardMigration: 'crates/backend/nomifun-db/migrations/012_unify_cron_agent_binding.sql',
  modelInputConstraintsForwardMigration: 'crates/backend/nomifun-db/migrations/013_model_input_constraints.sql',
  queueSettingForwardMigration: 'crates/backend/nomifun-db/migrations/014_retire_command_queue_setting.sql',
  executionPlanGateForwardMigration: 'crates/backend/nomifun-db/migrations/015_retire_execution_plan_gate.sql',
  officeHubForwardMigration: 'crates/backend/nomifun-db/migrations/016_office_hub.sql',
  officeCompactForwardMigration: 'crates/backend/nomifun-db/migrations/017_compact_office_receipts.sql',
});
const KNOWN = new Set(Object.values(AGENT_FORWARD_MIGRATIONS));
export function areKnownAgentMigrationPaths(paths) {
  return paths.includes(AGENT_FORWARD_MIGRATIONS.baseline)
    && paths.every(path => KNOWN.has(path));
}

const SOURCE = 'ce5cb186b9ba1a44c7c4699f4e1932542068537149c2a76ec880e2b8cc0bf8d9';
const TARGET = '0ef8e3eeec9835238c865441913383bb80880bf89e3f0d78b538780c7534bdfc';
const normalize = text => text.replace(/\s+/g, ' ').trim();
const compact = text => normalize(text).replace(/\s*([(),=])\s*/g, '$1');
export function officeHubMigrationErrors(source) {
  const statements = source.replace(/--[^\r\n]*/g, '').split(';').map(normalize).filter(Boolean);
  const expectedGuard = "INSERT INTO office_hub_metadata_guard SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM schema_metadata)"
    + " OR EXISTS(SELECT 1 FROM schema_metadata WHERE singleton_key='canonical'"
    + " AND data_generation=7 AND migration_head=1 AND projection_schema_version=1"
    + " AND canonical_schema_manifest_digest IN ('" + SOURCE + "','" + TARGET + "')) THEN 1 ELSE 0 END";
  const expectedUpdate = "UPDATE schema_metadata SET canonical_schema_manifest_digest='" + TARGET
    + "' WHERE singleton_key='canonical' AND data_generation=7 AND migration_head=1 AND projection_schema_version=1"
    + " AND canonical_schema_manifest_digest='" + SOURCE + "'";
  const allowed = [
    /^CREATE TABLE office_(?:connections|operations|operation_items|operation_attempts|operation_observations|recent_resources) \(/,
    /^CREATE INDEX idx_office_operations_owner_history ON office_operations\(/,
    /^CREATE UNIQUE INDEX idx_office_items_pending_target ON office_operation_items\(/,
    /^CREATE INDEX idx_office_observations_item ON office_operation_observations\(/,
    /^CREATE INDEX idx_office_recent_owner ON office_recent_resources\(/,
  ];
  const special = new Set([
    compact('CREATE TEMP TABLE office_hub_metadata_guard(valid INTEGER NOT NULL CHECK(valid=1))'),
    compact(expectedGuard),
    compact('DROP TABLE office_hub_metadata_guard'),
    compact(expectedUpdate),
  ]);
  const errors = [];
  for (const statement of statements) {
    if (!special.has(compact(statement)) && !allowed.some(rule => rule.test(statement))) {
      errors.push('Office migration may only add its declared product tables and exact preserved manifest transition (' + statement.slice(0, 100) + ')');
    }
  }
  for (const statement of special) {
    if (!statements.some(value => compact(value) === statement)) {
      errors.push('Office migration must retain its exact metadata guard and manifest transition');
    }
  }
  return errors;
}

// This migration compacts only the Office product's own facts. Scope SQL reads
// as well as writes: copying Agent rows into a product table is also forbidden.
export function officeCompactMigrationErrors(source) {
  const target = '565ff15ff4419e0fad8cf0fbcaa7ade3bd0f4fb50263851efed93f7a39921430';
  const statements = source.replace(/--[^\r\n]*/g, '').split(';').map(normalize).filter(Boolean);
  const guard = "INSERT INTO office_compact_integrity SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM schema_metadata)"
    + " OR EXISTS(SELECT 1 FROM schema_metadata WHERE singleton_key='canonical'"
    + " AND data_generation=7 AND migration_head=1 AND projection_schema_version=1"
    + " AND canonical_schema_manifest_digest IN ('" + TARGET + "','" + target + "')) THEN 1 ELSE 0 END";
  const update = "UPDATE schema_metadata SET canonical_schema_manifest_digest='" + target
    + "' WHERE singleton_key='canonical' AND data_generation=7 AND migration_head=1 AND projection_schema_version=1"
    + " AND canonical_schema_manifest_digest='" + TARGET + "'";
  const special = new Set([
    compact('CREATE TEMP TABLE office_compact_integrity(valid INTEGER NOT NULL CHECK(valid=1))'),
    compact(guard), compact(update), compact('DROP TABLE office_compact_integrity'),
  ]);
  for (const table of ['office_operations', 'office_operation_items']) {
    special.add(compact(`INSERT INTO sqlite_sequence(name,seq) SELECT '${table}_next',seq FROM sqlite_sequence WHERE name='${table}' AND NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='${table}_next')`));
    special.add(compact(`UPDATE sqlite_sequence SET seq=MAX(seq,COALESCE((SELECT seq FROM sqlite_sequence WHERE name='${table}'),seq)) WHERE name='${table}_next'`));
  }
  const allowedTables = new Set(['office_connections', 'office_operations', 'office_operation_items',
    'office_operation_attempts', 'office_operation_observations', 'office_recent_resources',
    'office_operation_receipts', 'office_operations_next', 'office_operation_items_next', 'office_compact_integrity']);
  const allowed = [
    /^INSERT INTO office_compact_integrity SELECT CASE WHEN /,
    /^ALTER TABLE office_connections ADD COLUMN recent_resources_json /,
    /^UPDATE office_connections SET recent_resources_json=/,
    /^CREATE TABLE office_(?:operation_receipts|operations_next|operation_items_next) \(/,
    /^INSERT INTO office_(?:operation_receipts|operations_next|operation_items_next)\(/,
    /^DROP TABLE office_(?:operation_attempts|operation_observations|recent_resources|operation_items|operations)$/,
    /^ALTER TABLE office_operations_next RENAME TO office_operations$/,
    /^ALTER TABLE office_operation_items_next RENAME TO office_operation_items$/,
    /^CREATE INDEX idx_office_operations_owner_history ON office_operations\(/,
    /^CREATE UNIQUE INDEX idx_office_items_pending_target ON office_operation_items\(/,
  ];
  const errors = [];
  for (const statement of statements) {
    if (special.has(compact(statement))) continue;
    const unquoted = statement.replace(/'(?:''|[^'])*'/g, "''");
    const references = [...unquoted.matchAll(/\b(?:FROM|JOIN|INTO|UPDATE|TABLE)\s+([a-z_][a-z_0-9]*)/gi)]
      .map(match => match[1].toLowerCase());
    if (!allowed.some(rule => rule.test(statement)) || references.some(table => !allowedTables.has(table))) {
      errors.push('Office compaction may only read and change its declared product tables (' + statement.slice(0, 100) + ')');
    }
  }
  for (const expected of special) {
    if (!statements.some(statement => compact(statement) === expected)) {
      errors.push('Office compaction must retain exact metadata and sequence transitions');
    }
  }
  return errors;
}
