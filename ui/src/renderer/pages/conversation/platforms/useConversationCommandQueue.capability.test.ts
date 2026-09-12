import { describe, expect, test } from 'bun:test';
import {
  createQueuedCommandItem,
  normalizeQueueState,
} from './useConversationCommandQueue';

describe('conversation command queue capability snapshot', () => {
  test('preserves the exact Skill and MCP selection through persistence normalization', () => {
    const capabilitySelection = {
      enabled_skills: ['pdf'],
      excluded_auto_skills: ['cron'],
      mcp_server_ids: ['0190f5fe-7c00-7a00-8000-000000000001'],
    };
    const item = createQueuedCommandItem({
      input: 'Review this',
      files: [],
      capability_selection: capabilitySelection,
    });

    expect(item.capability_selection).toEqual(capabilitySelection);
    expect(normalizeQueueState({ isPaused: false, items: [item] }).items[0]
      ?.capability_selection).toEqual(capabilitySelection);
  });
});
