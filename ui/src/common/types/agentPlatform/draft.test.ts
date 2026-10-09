import { describe, expect, test } from 'bun:test';
import {
  asAgentPresetId,
  asCapabilityId,
  asDigestHex,
  agentPresetDocumentsEqual,
  capabilityPlacement,
  createEmptyAgentPresetDocument,
  cloneDraft,
  isDraftDirty,
  placeCapability,
  type ChatRouteRecord,
  type AgentPresetDraft,
} from './index';

const capability = {
  id: asCapabilityId('workspace.files/read'),
  version: '1.0.0',
};

describe('AgentPreset draft model', () => {
  test('draft cloning preserves authoring data without a Runtime selector', () => {
    const saved: AgentPresetDraft = {
      preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001'),
      display_name: 'My Agent', document: createEmptyAgentPresetDocument(),
    };
    const draft = cloneDraft(saved);
    draft.document.persona = 'Careful collaborator';
    expect(isDraftDirty(saved, draft)).toBe(true);
    expect(cloneDraft(draft).document.persona).toBe('Careful collaborator');
    expect('runtime_build' in draft.document).toBe(false);
    draft.document.persona = '';
    expect(isDraftDirty(saved, draft)).toBe(false);
  });

  test('restores default-valued fields omitted by the compact wire document', () => {
    const wireDocument: Partial<AgentPresetDraft['document']> = createEmptyAgentPresetDocument();
    delete wireDocument.chat_route_records;
    delete wireDocument.system_role_provider_overrides;
    delete wireDocument.starter_prompts;
    delete wireDocument.runtime_policy;
    const wireDraft: AgentPresetDraft = {
      preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001'),
      display_name: 'Wire Agent',
      document: wireDocument as AgentPresetDraft['document'],
    };

    const draft = cloneDraft(wireDraft);

    expect(draft.document.chat_route_records).toEqual({});
    expect(draft.document.system_role_provider_overrides).toEqual({});
    expect(draft.document.starter_prompts).toEqual([]);
    expect(draft.document.runtime_policy.idmm.mode).toBe('off');
    expect('system_role_provider_overrides' in wireDocument).toBe(false);
  });

  test('normalizes a null wire default without replacing configured overrides', () => {
    const nullDocument = createEmptyAgentPresetDocument();
    (nullDocument as unknown as { system_role_provider_overrides: null })
      .system_role_provider_overrides = null;
    const nullDraft = cloneDraft({
      preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001'),
      display_name: 'Null Wire Agent',
      document: nullDocument,
    });
    expect(nullDraft.document.system_role_provider_overrides).toEqual({});

    const configuredDocument = createEmptyAgentPresetDocument();
    configuredDocument.system_role_provider_overrides.search = {
      role: {
        key: { role_id: 'search', contract_version: '1.0.0' },
        contract_digest: asDigestHex('a'.repeat(64)),
      },
      provider_mount_id: 'local-search',
    };
    const configuredDraft = cloneDraft({
      preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000002'),
      display_name: 'Configured Agent',
      document: configuredDocument,
    });
    expect(configuredDraft.document.system_role_provider_overrides)
      .toEqual(configuredDocument.system_role_provider_overrides);
  });

  test('compares normalized documents by value rather than object key order', () => {
    const complete = createEmptyAgentPresetDocument();
    const sparse = {
      instructions: '',
      persona: '',
      skill_bindings: [],
      enabled_capabilities: [],
      model_route_refs: {},
      schema_version: '1.0.0',
    } as unknown as AgentPresetDraft['document'];

    expect(agentPresetDocumentsEqual(complete, sparse)).toBe(true);
    sparse.persona = 'Different';
    expect(agentPresetDocumentsEqual(complete, sparse)).toBe(false);
  });
  test('preserves a restricted action allowlist when enabling an existing capability', () => {
    const empty = createEmptyAgentPresetDocument();
    const enabled = { ...empty, enabled_capabilities: [{ capability, action_allowlist: ['workspace.files/read'] }] };
    expect(capabilityPlacement(enabled, capability)).toBe('enabled');
    expect(placeCapability(enabled, capability, 'enabled').enabled_capabilities[0].action_allowlist).toEqual(['workspace.files/read']);
    expect(placeCapability(enabled, capability, 'none').enabled_capabilities).toEqual([]);
  });

  test('chat-style empty documents contain no hidden capability surface', () => {
    const draft: AgentPresetDraft = {
      preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001'),
      display_name: 'Minimal Chat',
      document: createEmptyAgentPresetDocument(),
    };

    expect(draft.document.enabled_capabilities).toEqual([]);
    expect(draft.document.skill_bindings).toEqual([]);
    expect(draft.document.runtime_policy.idmm.mode).toBe('off');
    expect('resource_bindings' in draft.document).toBe(false);
    expect(draft.document.chat_route_records).toEqual({});
  });

  test('runtime policy changes are revision-bearing authoring data', () => {
    const saved: AgentPresetDraft = {
      preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001'),
      display_name: 'Guarded Agent',
      document: createEmptyAgentPresetDocument(),
    };
    const draft = cloneDraft(saved);
    draft.document.runtime_policy.idmm.mode = 'rule_only';
    expect(isDraftDirty(saved, draft)).toBe(true);
    expect(saved.document.runtime_policy.idmm.mode).toBe('off');
  });

  test('clone preserves the explicit agent_chat route record', () => {
    const record: ChatRouteRecord = {
      schema: 'nomifun.chat-route-record.v1' as const,
      task: 'agent_chat' as const,
      primary: {
        model_route_id: 'opaque-route',
        model_route_revision: 1,
        provider_id: 'provider',
        model: 'model',
        protocol: 'openai_chat' as const,
        connection_config_ref: 'default',
        config_revision_digest: asDigestHex('a'.repeat(64)),
        credential_ref: 'credential',
        features: ['text_input', 'text_output'],
      },
      failovers: [],
    };
    const draft: AgentPresetDraft = {
      preset_id: asAgentPresetId('0190f5fe-7c00-7a00-0000-000000000001'),
      display_name: 'Route',
      document: {
        ...createEmptyAgentPresetDocument(),
        model_route_refs: { agent_chat: record.primary.model_route_id },
        chat_route_records: { agent_chat: record },
      },
    };
    const cloned = structuredClone(draft);
    expect(cloned.document.model_route_refs.agent_chat).toBe('opaque-route');
    expect(cloned.document.chat_route_records.agent_chat?.primary.provider_id).toBe('provider');
  });
});
