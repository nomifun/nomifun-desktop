import { describe, expect, test } from 'bun:test';
import type {
  AgentBindingSummary,
  AgentPresetSummary,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import {
  filterConversationAgentPresets,
  isConversationAgentTemplate,
  isConversationAgentTemplateKey,
} from './conversationAgentCatalog';

const preset = (presetId: string, displayName: string): AgentPresetSummary => ({
  preset_id: presetId,
  source: 'user',
  display_name: displayName,
  bound_target_count: 0,
} as AgentPresetSummary);

const binding = (targetKind: string, presetId: string): AgentBindingSummary => ({
  target_kind: targetKind,
  target_id: `${targetKind}-1`,
  preset_revision_ref: {
    preset_id: presetId,
    revision: 1,
    revision_digest: 'a'.repeat(64),
  },
  resolved_snapshot_ref: {
    snapshot_id: '0190f5fe-7c00-7a00-8000-000000000901',
    snapshot_digest: 'b'.repeat(64),
  },
  binding_version: 1,
} as AgentBindingSummary);

describe('Conversation Agent catalog boundary', () => {
  test('keeps dedicated product Agents out of ordinary Conversation templates', () => {
    expect(isConversationAgentTemplateKey('companion.default')).toBe(false);
    expect(isConversationAgentTemplateKey('customer-service.default')).toBe(false);
    expect(isConversationAgentTemplateKey('assistant.general')).toBe(true);
    expect(isConversationAgentTemplateKey('removed.template')).toBe(false);
    expect(isConversationAgentTemplate({
      template_key: 'customer-service.default',
    } as OfficialPresetTemplate)).toBe(false);
    expect(isConversationAgentTemplate({
      template_key: 'companion.default',
    } as OfficialPresetTemplate)).toBe(false);
  });

  test('uses product ownership rather than names to hide Customer Service presets', () => {
    const customerBound = preset('customer-preset', 'General helper');
    const ordinary = preset('ordinary-preset', 'Customer support expert');
    const companionBound = preset('companion-preset', 'Companion');

    expect(filterConversationAgentPresets(
      [customerBound, ordinary, companionBound],
      [
        binding('customer', customerBound.preset_id),
        binding('companion', companionBound.preset_id),
      ],
    )).toEqual([ordinary, companionBound]);
  });
});
