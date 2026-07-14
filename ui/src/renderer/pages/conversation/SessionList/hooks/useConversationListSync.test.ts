import { describe, expect, test } from 'bun:test';

import { isGeneratingStreamMessage } from './useConversationListSync';

describe('conversation list stream activity', () => {
  test('ordinary content raises the sidebar generating state', () => {
    expect(isGeneratingStreamMessage({ type: 'content', data: { content: 'chunk' } })).toBe(true);
  });

  test('a complete assistant projection never raises a stuck sidebar spinner', () => {
    expect(
      isGeneratingStreamMessage({
        type: 'content',
        data: { content: 'final execution report' },
        stream_complete: true,
      })
    ).toBe(false);
  });
});
