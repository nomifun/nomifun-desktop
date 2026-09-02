import { describe, expect, test } from 'bun:test';
import {
  asAgentPresetId,
  asCapabilityId,
  asDigestHex,
  capabilityPlacement,
  createEmptyAgentPresetDocument,
  placeCapability,
  type ChatRouteRecord,
  type AgentPresetDraft,
} from './index';

const capability = {
  id: asCapabilityId('fs.read'),
  version: '1.0.0',
};

describe('AgentPreset draft model', () => {
  test('keeps initial and on-demand exact sets mutually exclusive', () => {
    const empty = createEmptyAgentPresetDocument();
    const initial = placeCapability(empty, capability, 'initial');
    const onDemand = placeCapability(initial, capability, 'on_demand');

    expect(capabilityPlacement(initial, capability.id)).toBe('initial');
    expect(onDemand.initial_capabilities).toHaveLength(0);
    expect(onDemand.on_demand_capabilities).toHaveLength(1);
  });

  test('chat-style empty documents contain no hidden capability surface', () => {
    const draft: AgentPresetDraft = {
      preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001'),
      display_name: 'Minimal Chat',
      document: createEmptyAgentPresetDocument(),
    };

    expect(draft.document.initial_capabilities).toEqual([]);
    expect(draft.document.on_demand_capabilities).toEqual([]);
    expect(draft.document.skill_bindings).toEqual([]);
    expect(draft.document.resource_bindings).toEqual([]);
    expect(draft.document.chat_route_records).toEqual({});
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
