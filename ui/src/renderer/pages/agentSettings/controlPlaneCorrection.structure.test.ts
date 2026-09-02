import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const repoFile = (relativePath: string) =>
  readFileSync(new URL(`../../../../../${relativePath}`, import.meta.url), 'utf8');

const structBlock = (source: string, name: string): string => {
  const start = source.indexOf(`pub struct ${name}`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = source.indexOf('\n}', start);
  expect(end).toBeGreaterThan(start);
  return source.slice(start, end + 2);
};

describe('C5 template expansion correction', () => {
  test('keeps template identity transient and out of StoredPreset/API summary', () => {
    const store = repoFile('crates/backend/nomifun-agent-control-plane/src/store.rs');
    const api = repoFile('crates/backend/nomifun-api-types/src/agent_platform.rs');
    const templateField = 'source_template' + '_key';

    expect(structBlock(store, 'StoredPreset').includes(templateField)).toBe(false);
    expect(structBlock(api, 'AgentPresetSummaryDto').includes(templateField)).toBe(false);
    expect(structBlock(api, 'AgentPresetDraftDto').includes(templateField)).toBe(true);
  });

  test('create-from-template commits an initial normal Revision and Snapshot atomically', () => {
    const service = repoFile('crates/backend/nomifun-agent-control-plane/src/service.rs');
    const store = repoFile('crates/backend/nomifun-agent-control-plane/src/store.rs');

    expect(service.includes('create_with_initial_revision(')).toBe(true);
    expect(service.includes('.insert_preset_with_revision(')).toBe(true);
    expect(store.includes('async fn insert_preset_with_revision(')).toBe(true);
    expect(service.includes('current_stable_revision: Some(revision.reference.clone())')).toBe(
      true
    );
  });
});
