import { describe, expect, test } from 'bun:test';
import {
  asAgentPresetId,
  asCapabilityId,
  capabilityPlacement,
  createEmptyAgentPresetDocument,
  placeCapability,
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
  });
});
