import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { AGENT_FORWARD_MIGRATIONS, areKnownAgentMigrationPaths, officeHubMigrationErrors, officeCompactMigrationErrors, feishuConnectorRenameMigrationErrors } from './lib/agent-migration-boundary.mjs';

test('the office extension is explicit and never authorizes an unknown forward migration', () => {
  const known = Object.values(AGENT_FORWARD_MIGRATIONS);
  assert.equal(known.length, 21);
  assert(areKnownAgentMigrationPaths(known));
  assert(known.includes('crates/backend/nomifun-db/migrations/016_office_hub.sql'));
  assert(known.includes('crates/backend/nomifun-db/migrations/021_feishu_connector_tables.sql'));
  assert(known.includes('crates/backend/nomifun-db/migrations/018_remove_office_activity.sql'));
  assert(known.includes('crates/backend/nomifun-db/migrations/019_remove_office_overview.sql'));
  assert(!areKnownAgentMigrationPaths(known.filter(path => path !== AGENT_FORWARD_MIGRATIONS.baseline)));
  for (const unknown of ['017_unknown.sql', '016_agent_compatibility.sql', '999_future.sql']) {
    assert(!areKnownAgentMigrationPaths([...known, 'crates/backend/nomifun-db/migrations/' + unknown]));
  }
});
test('office compaction cannot read Agent facts or widen metadata and sequence authority', () => {
  const source = readFileSync(new URL('../crates/backend/nomifun-db/migrations/017_compact_office_receipts.sql', import.meta.url), 'utf8');
  assert.deepEqual(officeCompactMigrationErrors(source), []);
  for (const extra of ['DELETE FROM agent_sessions;',
    'INSERT INTO office_operation_receipts(receipt_id) SELECT payload_id FROM agent_payloads;',
    "UPDATE sqlite_sequence SET seq=0 WHERE name='agent_sessions';",
    "UPDATE schema_metadata SET data_generation=8;",
    'CREATE TABLE office_other AS SELECT * FROM office_operations;']) {
    assert(officeCompactMigrationErrors(source + '\n' + extra).length > 0);
  }
  assert(officeCompactMigrationErrors(source.replace('data_generation=7', 'data_generation=8')).length > 0);
});
test('office migration preserves the exact non-Agent metadata transition', () => {
  const source = readFileSync(new URL('../crates/backend/nomifun-db/migrations/016_office_hub.sql', import.meta.url), 'utf8');
  assert.deepEqual(officeHubMigrationErrors(source), []);
  assert(officeHubMigrationErrors(source + '\nDELETE FROM agent_sessions;').length > 0);
  assert(officeHubMigrationErrors(source + '\nCREATE TABLE office_other AS SELECT * FROM agent_messages;').length > 0);
  assert(officeHubMigrationErrors(source.replace('data_generation=7', 'data_generation=8')).length > 0);
  assert(officeHubMigrationErrors(source.replaceAll('ce5cb186b9ba1a44c7c4699f4e1932542068537149c2a76ec880e2b8cc0bf8d9', 'a'.repeat(64))).length > 0);
});
test('feishu connector rename only renames its declared product objects', () => {
  const source = readFileSync(new URL('../crates/backend/nomifun-db/migrations/021_feishu_connector_tables.sql', import.meta.url), 'utf8');
  assert.deepEqual(feishuConnectorRenameMigrationErrors(source), []);
  assert(feishuConnectorRenameMigrationErrors(source + '\nDELETE FROM agent_sessions;').length > 0);
  assert(feishuConnectorRenameMigrationErrors(source + '\nDROP TABLE office_connections;').length > 0);
  assert(feishuConnectorRenameMigrationErrors(source.replace('feishu_operation_receipts', 'feishu_operation_receipts_v2')).length > 0);
  assert(feishuConnectorRenameMigrationErrors(source.replace('data_generation=7', 'data_generation=8')).length > 0);
  assert(feishuConnectorRenameMigrationErrors(source.replaceAll('565ff15ff4419e0fad8cf0fbcaa7ade3bd0f4fb50263851efed93f7a39921430', 'a'.repeat(64))).length > 0);
});
