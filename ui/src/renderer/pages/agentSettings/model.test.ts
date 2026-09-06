import { describe, expect, test } from 'bun:test';
import type {
  AgentPresetDraft,
  CapabilityCatalogItem,
  ChatRouteRecord,
  ResolveAgentPresetPreviewResponse,
  SaveAgentPresetRevisionResponse,
} from '@/common/types/agentPlatform';
import {
  asCapabilityId,
  asDigestHex,
  asPackageId,
  asResolvedSnapshotId,
} from '@/common/types/agentPlatform';
import {
  capabilityMatchesSearch,
  capabilityProductCopy,
  capabilityPlacement,
  classifyAgentUiError,
  editorCapabilityReferences,
  placeCapability,
  saveDraftRevisionWithPreview,
  selectChatRouteCandidate,
  selectedRequiredResourceKinds,
  sortCapabilitiesByPlacement,
} from './model';

const draft = (): AgentPresetDraft => ({
  preset_id: '0190f5fe-7c00-7a00-8000-000000000001' as AgentPresetDraft['preset_id'],
  display_name: 'Coding',
  document: {
    schema_version: '1.0.0',
    model_route_refs: {},
    chat_route_records: {},
    initial_capabilities: [],
    on_demand_capabilities: [],
    skill_bindings: [],
    system_role_provider_overrides: {},
    persona: '',
    instructions: '',
    starter_prompts: [],
  },
});

const capability = (
  id: string,
  requiredResourceKinds: string[] = []
): CapabilityCatalogItem => ({
  capability: {
    id: asCapabilityId(id),
    version: '1.0.0',
  },
  kind: 'tool',
  display_name: id,
  description: `${id} description`,
  source_package: {
    id: asPackageId('nomifun.test'),
    version: '1.0.0',
  },
  source_kind: 'first_party',
  materialization_state: 'materialized',
  supported_surfaces: ['desktop'],
  required_runtime_features: [],
  required_resource_kinds: requiredResourceKinds,
  required_capabilities: [],
  conflicting_capabilities: [],
  action_count: 1,
  context_contributor_count: 0,
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
    required_resource_kind_count: 0,
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
    required_resource_kinds: [],
    service_key_diagnostics: [],
  },
  can_save_revision: status === 'ready',
  can_create_session: status === 'ready',
});

describe('Agent Settings capability authoring model', () => {
  test('turns placeholder catalog metadata into product-facing capability copy', () => {
    const placeholder = {
      ...capability('knowledge.search', ['knowledge_base']),
      description: 'knowledge.search',
    };

    expect(capabilityProductCopy(placeholder, 'zh-CN')).toEqual({
      name: '搜索知识库',
      description: '搜索使用时由当前会话选择的知识库。',
    });
    expect(capabilityProductCopy(placeholder, 'en-US')).toEqual({
      name: 'Search the knowledge base',
      description: 'Search the knowledge base selected at use time.',
    });
  });

  test('preserves real capability metadata supplied by the owning package', () => {
    const described = {
      ...capability('knowledge.search', ['knowledge_base']),
      display_name: 'Knowledge search',
      description: 'Search the knowledge base selected by this conversation.',
    };

    expect(capabilityProductCopy(described, 'zh-CN')).toEqual({
      name: described.display_name,
      description: described.description,
    });
  });

  test('searches by localized product copy as well as canonical identifiers', () => {
    const item = capability('knowledge.search', ['knowledge_base']);

    expect(capabilityMatchesSearch(item, '知识库', 'zh-CN')).toBe(true);
    expect(capabilityMatchesSearch(item, 'knowledge.search', 'zh-CN')).toBe(true);
    expect(capabilityMatchesSearch(item, 'workspace', 'zh-CN')).toBe(false);
  });

  test('moves a capability through off, startup, and requestable states', () => {
    const target = capability('process.exec').capability;
    const initial = placeCapability(draft().document, target, 'initial');
    expect(capabilityPlacement(initial, target.id)).toBe('initial');

    const onDemand = placeCapability(initial, target, 'on_demand');
    expect(capabilityPlacement(onDemand, target.id)).toBe('on_demand');
    expect(onDemand.initial_capabilities).toEqual([]);
    expect(onDemand.on_demand_capabilities).toEqual([{ capability: target }]);

    const off = placeCapability(onDemand, target, 'none');
    expect(capabilityPlacement(off, target.id)).toBe('none');
    expect(off.initial_capabilities).toEqual([]);
    expect(off.on_demand_capabilities).toEqual([]);
  });

  test('keeps concrete resource identities out of capability selections and documents', () => {
    const target = capability('knowledge.search', ['knowledge_base']).capability;
    const document = placeCapability(draft().document, target, 'initial');

    expect('resource_bindings' in document).toBe(false);
    expect('resource_binding_refs' in document.initial_capabilities[0]).toBe(false);
  });

  test('derives required resource kinds only from selected capabilities', () => {
    const process = capability('process.exec', ['process_session', 'workspace']);
    const knowledge = capability('knowledge.search', ['knowledge_base', 'workspace']);
    const unused = capability('robot.motion', ['robot']);
    const withProcess = placeCapability(draft().document, process.capability, 'initial');
    const selected = placeCapability(withProcess, knowledge.capability, 'on_demand');

    expect(selectedRequiredResourceKinds(selected, [process, knowledge, unused])).toEqual([
      'knowledge_base',
      'process_session',
      'workspace',
    ]);
  });

  test('does not project a different catalog version onto a saved selection', () => {
    const saved = capability('knowledge.search', ['knowledge_base']);
    const newer = {
      ...saved,
      capability: {
        ...saved.capability,
        version: '2.0.0',
      },
      required_resource_kinds: ['workspace'],
    };
    const selected = placeCapability(draft().document, saved.capability, 'initial');

    expect(capabilityPlacement(selected, saved.capability)).toBe('initial');
    expect(capabilityPlacement(selected, newer.capability)).toBe('none');
    expect(selectedRequiredResourceKinds(selected, [newer])).toEqual([]);
  });

  test('keeps selected capabilities visible before the closed catalog entries', () => {
    const startup = capability('knowledge.search');
    const requestable = capability('agent.delegate');
    const closed = capability('a11y.observe');
    const withStartup = placeCapability(
      draft().document,
      startup.capability,
      'initial'
    );
    const selected = placeCapability(
      withStartup,
      requestable.capability,
      'on_demand'
    );

    expect(
      sortCapabilitiesByPlacement(selected, [closed, requestable, startup]).map(
        (item) => item.capability.id
      )
    ).toEqual(['knowledge.search', 'agent.delegate', 'a11y.observe']);
  });

  test('keeps a selected capability visible when the current catalog no longer has it', () => {
    const selected = placeCapability(
      draft().document,
      capability('legacy.capability').capability,
      'on_demand'
    );

    expect(
      editorCapabilityReferences(selected, [capability('fs.read')], [
        capability('fs.read'),
      ]).map((reference) => reference.id)
    ).toEqual(['legacy.capability', 'fs.read']);
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

  test('distinguishes a preset that is already absent', () => {
    expect(
      classifyAgentUiError({ code: 'AGENT_PRESET_NOT_FOUND', status: 404 }, 'delete')
    ).toBe('preset-not-found');
  });
});
