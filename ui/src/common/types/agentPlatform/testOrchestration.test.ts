import { describe, expect, test } from 'bun:test';
import {
  asAgentPresetId,
  asAgentSessionId,
  asDigestHex,
  asResolvedSnapshotId,
  createEmptyAgentPresetDocument,
  runAgentPresetTest,
  type AgentPresetDraft,
  type AgentPresetTestPorts,
} from './index';

const presetId = asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001');
const sessionId = asAgentSessionId('0190f5fe-7c00-7a00-8000-000000000002');

const draft: AgentPresetDraft = {
  preset_id: presetId,
  display_name: 'Coding',
  document: createEmptyAgentPresetDocument(),
};

const preview = {
  status: 'ready' as const,
  draft_digest: asDigestHex('1'.repeat(64)),
  preview_digest: asDigestHex('2'.repeat(64)),
  candidate_revision_ref: {
    preset_id: presetId,
    revision: 1,
    revision_digest: asDigestHex('3'.repeat(64)),
  },
  resolved_snapshot_ref: {
    snapshot_id: asResolvedSnapshotId('0190f5fe-7c00-7a00-8000-000000000003'),
    snapshot_digest: asDigestHex('4'.repeat(64)),
  },
  summary: {
    initial_count: 0,
    on_demand_count: 0,
    active_at_start_count: 0,
    model_tool_count: 0,
    context_contributor_count: 0,
    on_demand_index_count: 0,
    skill_count: 0,
    mcp_count: 0,
    resource_binding_count: 0,
    provider_initialization_count: 0,
  },
  diagnostics: [],
  revision_diff: {
    added_initial: [],
    removed_initial: [],
    added_on_demand: [],
    removed_on_demand: [],
    added_skills: [],
    removed_skills: [],
    resource_bindings_changed: false,
    model_routes_changed: false,
    instructions_changed: false,
  },
  inspector: {
    required_runtime_protocol_version: '1.0.0',
    required_runtime_features: [],
    initial_capabilities: [],
    on_demand_capabilities: [],
    compact_on_demand_index: [],
    tool_schema_refs: [],
    context_schema_refs: [],
    mcp_materializations: [],
    typed_resource_bindings: [],
    service_key_diagnostics: [],
  },
  can_save_revision: true,
  can_create_session: true,
};

describe('D-022 Agent Settings Test orchestration', () => {
  test('dirty draft saves before creating one ordinary Session', async () => {
    const calls: string[] = [];
    const ports: AgentPresetTestPorts = {
      preview: async () => {
        calls.push('preview');
        return preview;
      },
      save: async (request) => {
        calls.push('save');
        return {
          preset: {
            preset_id: presetId,
            source: 'user',
            display_name: 'Coding',
            current_stable_revision: request.draft.current_revision,
            bound_target_count: 0,
          },
          revision: {
            reference: preview.candidate_revision_ref,
            document: request.draft.document,
            created_by: 'owner',
            created_at_ms: 1,
          },
          resolved_snapshot_ref: preview.resolved_snapshot_ref!,
          preview_digest: preview.preview_digest,
        };
      },
      createSession: async () => {
        calls.push('session');
        return {
          agent_session_id: sessionId,
          agent_binding: {
            preset_revision_ref: preview.candidate_revision_ref,
            resolved_snapshot_ref: preview.resolved_snapshot_ref!,
            typed_resource_bindings: [],
            binding_version: 1,
          },
          state: 'opening',
          cursor: { agent_session_id: sessionId, seq: 1 },
        };
      },
      createTurn: async () => {
        calls.push('turn');
        return {
          agent_session_id: sessionId,
          operation_id: 'turn-1',
          cursor: { agent_session_id: sessionId, seq: 2 },
          status: 'accepted',
        };
      },
    };

    await runAgentPresetTest({
      draft,
      dirty: true,
      input: 'Inspect the workspace',
      idempotencyKey: 'editor-test-1',
      ports,
    });

    expect(calls).toEqual(['preview', 'save', 'session', 'turn']);
  });

  test('save failure creates no Session or Turn', async () => {
    const calls: string[] = [];
    const ports: AgentPresetTestPorts = {
      preview: async () => preview,
      save: async () => {
        calls.push('save');
        throw new Error('PRESET_REVISION_SAVE_FAILED');
      },
      createSession: async () => {
        calls.push('session');
        throw new Error('must not run');
      },
      createTurn: async () => {
        calls.push('turn');
        throw new Error('must not run');
      },
    };

    await expect(
      runAgentPresetTest({
        draft,
        dirty: true,
        input: 'Run',
        idempotencyKey: 'editor-test-2',
        ports,
      })
    ).rejects.toThrow('PRESET_REVISION_SAVE_FAILED');
    expect(calls).toEqual(['save']);
  });
});
