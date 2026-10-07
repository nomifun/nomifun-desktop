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
if (!migrations.includes(baseline) || migrations.some((path) => ![baseline, pluginForwardMigration].includes(path))) {
  failures.push('Agent clean cut requires its canonical baseline and only explicitly supported forward product migrations');
}
if (migrations.includes(pluginForwardMigration)) {
  const source = readFileSync(resolve(ROOT, pluginForwardMigration), 'utf8');
  if (/\b(?:agent_\w+|schema_metadata|_sqlx_migrations)\b/i.test(source)) {
    failures.push(`${pluginForwardMigration}: a Plugin forward migration cannot alter canonical Agent data or lineage receipts`);
  }
}
if (failures.length) {
  console.error(`Agent Session boundary failed (${failures.length}):\n${failures.map((message) => `  - ${message}`).join('\n')}`);
  process.exitCode = 1;
} else {
  console.log('Agent Session boundary passed: canonical event history, native reasoning, current stream and immutable baseline.');
}
