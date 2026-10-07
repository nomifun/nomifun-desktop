import { describe, expect, test } from 'bun:test';
import {
  createQueuedCommandItem,
  normalizeQueueState,
} from './useConversationCommandQueue';

describe('conversation command queue frozen binding', () => {
  test('drops a legacy session capability overlay while preserving the queued turn', () => {
    const item = createQueuedCommandItem({
      input: 'Review this',
      files: [],
    });
    const restored = normalizeQueueState({
      isPaused: false,
      items: [{
        ...item,
        capability_selection: {
          enabled_skills: ['pdf'],
          excluded_auto_skills: ['cron'],
          mcp_server_ids: ['0190f5fe-7c00-7a00-8000-000000000001'],
        },
      }],
    }).items[0] as unknown as Record<string, unknown>;

    expect(restored.input).toBe('Review this');
    expect(restored).not.toHaveProperty('capability_selection');
  });
});
