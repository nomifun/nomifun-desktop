import { describe, expect, test } from 'bun:test';
import type {
  AgentPresetDraft,
  ChatRouteRecord,
  ResolveAgentPresetPreviewResponse,
  SaveAgentPresetRevisionResponse,
} from '@/common/types/agentPlatform';
import { asDigestHex, asResolvedSnapshotId } from '@/common/types/agentPlatform';
import {
  DEFAULT_WORKSPACE_RESOURCE_ID,
  KNOWLEDGE_NAME_PARAMETER,
  KNOWLEDGE_ROOT_PARAMETER,
  WORKSPACE_ROOT_PARAMETER,
  bindKnowledgeBaseResource,
  bindWorkspaceResource,
  saveDraftRevisionWithPreview,
  selectChatRouteCandidate,
  withHostResolvedWorkspaceBinding,
} from './model';

const draft = (): AgentPresetDraft => ({
  preset_id: '0190f5fe-7c00-7a00-8000-000000000001' as AgentPresetDraft['preset_id'],
  display_name: 'Coding',
  document: {
    schema_version: '1.0.0',
    surfaces: ['desktop'],
    model_route_refs: {},
    chat_route_records: {},
    initial_capabilities: [],
    on_demand_capabilities: [],
    skill_bindings: [],
    resource_bindings: [
      {
        binding_id: 'workspace',
        resource_kind: 'workspace',
        resource_id: '',
        owner_id: 'owner',
        operations: ['read', 'write'],
        typed_parameters: {},
      },
    ],
    system_role_provider_overrides: {},
    persona: '',
    instructions: '',
    context_policy: {},
    execution_constraints: {},
    runtime_budget: {},
  },
});

const preview = (
  status: ResolveAgentPresetPreviewResponse['status']
): ResolveAgentPresetPreviewResponse => ({
  status,
  draft_digest: asDigestHex('1'.repeat(64)),
  preview_digest: asDigestHex('2'.repeat(64)),
  candidate_revision_ref: {
    preset_id: draft().preset_id,
    revision: 1,
    revision_digest: asDigestHex('3'.repeat(64)),
  },
  resolved_snapshot_ref:
    status === 'ready'
      ? {
          snapshot_id: asResolvedSnapshotId('0190f5fe-7c00-7a00-8000-000000000003'),
          snapshot_digest: asDigestHex('4'.repeat(64)),
        }
      : undefined,
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
  diagnostics:
    status === 'ready' ? [] : [{ severity: 'error', code: 'BLOCKED', message: 'blocked' }],
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
  can_save_revision: status === 'ready',
  can_create_session: status === 'ready',
});

describe('Agent Settings host resource resolution', () => {
  test('binds an existing knowledge base without treating its id as a path', () => {
    const binding = bindKnowledgeBaseResource(
      {
        binding_id: 'knowledge-primary',
        resource_kind: 'knowledge_base',
        resource_id: '',
        owner_id: 'owner',
        operations: ['read', 'search'],
        typed_parameters: {},
      },
      {
        knowledge_base_id: '0190f5fe-7c00-7a00-8000-000000000002',
        name: 'Release runbooks',
        root_path: 'C:\\knowledge\\release',
      }
    );

    expect(binding.resource_id).toBe('0190f5fe-7c00-7a00-8000-000000000002');
    expect(binding.typed_parameters?.[KNOWLEDGE_NAME_PARAMETER]).toBe('Release runbooks');
    expect(binding.typed_parameters?.[KNOWLEDGE_ROOT_PARAMETER]).toBe('C:\\knowledge\\release');
  });

  test('adds the host workspace path without treating resource_id as a path', () => {
    const resolved = withHostResolvedWorkspaceBinding(draft(), 'C:\\work\\nomifun');
    const binding = resolved.document.resource_bindings[0];

    expect(binding.resource_id).toBe(DEFAULT_WORKSPACE_RESOURCE_ID);
    expect(binding.typed_parameters?.[WORKSPACE_ROOT_PARAMETER]).toBe('C:\\work\\nomifun');
  });

  test('is stable once the exact host path has been resolved', () => {
    const first = withHostResolvedWorkspaceBinding(draft(), '/work/nomifun');
    const second = withHostResolvedWorkspaceBinding(first, '/work/nomifun');

    expect(second).toBe(first);
  });

  test('preserves an explicitly picked workspace path when resolving the host default', () => {
    const selected = bindWorkspaceResource(
      draft().document.resource_bindings[0],
      'D:\\projects\\agent'
    );
    const resolved = withHostResolvedWorkspaceBinding(
      {
        ...draft(),
        document: {
          ...draft().document,
          resource_bindings: [selected],
        },
      },
      'C:\\work\\nomifun'
    );

    expect(
      resolved.document.resource_bindings[0].typed_parameters?.[WORKSPACE_ROOT_PARAMETER]
    ).toBe('D:\\projects\\agent');
  });

  test('reorders an exact route candidate without changing its internal contract', () => {
    const record = {
      schema: 'nomifun.chat-route-record.v1',
      task: 'agent_chat',
      primary: {
        model_route_id: 'route-primary',
        model_route_revision: 1,
        provider_id: 'provider-primary',
        model: 'model-primary',
        protocol: 'openai_chat',
        connection_config_ref: 'connection-primary',
        config_revision_digest: 'a'.repeat(64),
        credential_ref: 'credential-primary',
        features: ['text_input', 'text_output'],
      },
      failovers: [
        {
          model_route_id: 'route-fallback',
          model_route_revision: 2,
          provider_id: 'provider-fallback',
          model: 'model-fallback',
          protocol: 'openai_responses',
          connection_config_ref: 'connection-fallback',
          config_revision_digest: 'b'.repeat(64),
          credential_ref: 'credential-fallback',
          features: ['text_input', 'text_output'],
        },
      ],
    } as ChatRouteRecord;

    const selected = selectChatRouteCandidate(record, 'route-fallback@2');

    expect(selected?.primary).toEqual(record.failovers[0]);
    expect(selected?.failovers).toEqual([record.primary]);
  });

  test('always previews before save and refuses a blocked draft write', async () => {
    const calls: string[] = [];
    const saved = {} as SaveAgentPresetRevisionResponse;
    const ready = await saveDraftRevisionWithPreview(draft(), {
      preview: async () => {
        calls.push('preview');
        return preview('ready');
      },
      save: async () => {
        calls.push('save');
        return saved;
      },
    });

    expect(calls).toEqual(['preview', 'save']);
    expect(ready.saved).toBe(saved);

    calls.length = 0;
    const blocked = await saveDraftRevisionWithPreview(draft(), {
      preview: async () => {
        calls.push('preview');
        return preview('blocked');
      },
      save: async () => {
        calls.push('save');
        return saved;
      },
    });

    expect(calls).toEqual(['preview']);
    expect(blocked.saved).toBe(null);
  });
});
