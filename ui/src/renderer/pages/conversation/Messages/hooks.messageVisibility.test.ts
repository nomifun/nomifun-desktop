/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import type { TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId, type ConversationId } from '@/common/types/ids';
import {
  composeMessageForTest,
  drainPendingMessageUpdates,
  mergeFetchedMessagesForConversation,
  normalizeDbMessage,
} from './hooks';

const source = readFileSync(new URL('./hooks.ts', import.meta.url), 'utf8');

const conversationA = parseConversationId('0190f5fe-7c00-7a00-8000-000000000004');
const conversationB = parseConversationId('0190f5fe-7c00-7a00-8000-000000000005');

const textMessage = (
  conversationId: ConversationId,
  suffix: string,
  content: string,
  createdAt: number
): TMessage => {
  const msgId = parseMessageId(`0190f5fe-7c00-7a00-8000-0000000000${suffix}`);
  return {
    id: msgId,
    msg_id: msgId,
    conversation_id: conversationId,
    type: 'text',
    position: 'right',
    content: { content },
    created_at: createdAt,
  };
};

describe('message visibility across batching and conversation switches', () => {
  test('authoritative empty replacement hides an earlier non-tail draft in place', () => {
    const segmentId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000031');
    const draft: TMessage = {
      id: 'draft-local',
      msg_id: segmentId,
      conversation_id: conversationA,
      type: 'text',
      position: 'left',
      status: 'work',
      content: { content: 'discarded draft' },
      created_at: 1,
    };
    const later = textMessage(conversationA, '32', 'later process row', 2);
    const replacement: TMessage = {
      id: 'replacement-local',
      msg_id: segmentId,
      conversation_id: conversationA,
      type: 'text',
      position: 'left',
      status: 'finish',
      hidden: true,
      content: { content: '', replace: true },
      created_at: 3,
    };

    const merged = composeMessageForTest(replacement, [draft, later]);
    expect(merged).toHaveLength(2);
    expect(merged[0].id).toBe('draft-local');
    expect(merged[0].hidden).toBe(true);
    expect(merged[0].status).toBe('finish');
    expect(merged[0].type === 'text' && merged[0].content.content).toBe('');
  });

  test('rehydrates truncated recovery metadata from a persisted tips row', () => {
    const sourceMessageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000033');
    const persisted = normalizeDbMessage({
      id: 'persisted-tip',
      msg_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000034'),
      conversation_id: conversationA,
      type: 'tips',
      position: 'center',
      content: {
        content: 'output truncated',
        type: 'error',
        error: { message: 'output truncated', code: 'OUTPUT_TRUNCATED', retryable: true },
        recovery: {
          kind: 'continue_truncated',
          source_message_id: sourceMessageId,
          failure_code: 'output_truncated',
        },
      } as any,
    });
    if (persisted.type !== 'tips') throw new Error('expected tips');
    expect(persisted.content.recovery?.source_message_id).toBe(sourceMessageId);
  });

  test('drains a queued message synchronously and exactly once', () => {
    const sent = textMessage(conversationA, '01', 'persist me', 1);
    const pendingRef = { current: [{ message: sent, add: false }] };
    let list: TMessage[] = [];
    let updateCalls = 0;
    const update = (updater: (current: TMessage[]) => TMessage[]) => {
      updateCalls += 1;
      list = updater(list);
    };

    expect(drainPendingMessageUpdates(pendingRef, update)).toBe(true);
    expect(list).toEqual([sent]);
    expect(pendingRef.current).toEqual([]);

    expect(drainPendingMessageUpdates(pendingRef, update)).toBe(false);
    expect(updateCalls).toBe(1);
    expect(list).toEqual([sent]);
  });

  test('keeps a re-entrant event in a new batch instead of losing or duplicating it', () => {
    const first = textMessage(conversationA, '01', 'first', 1);
    const second = textMessage(conversationA, '02', 'second', 2);
    const pendingRef = { current: [{ message: first, add: false }] };
    let list: TMessage[] = [];
    let enqueueDuringFirstDrain = true;
    const update = (updater: (current: TMessage[]) => TMessage[]) => {
      if (enqueueDuringFirstDrain) {
        enqueueDuringFirstDrain = false;
        pendingRef.current.push({ message: second, add: false });
      }
      list = updater(list);
    };

    expect(drainPendingMessageUpdates(pendingRef, update)).toBe(true);
    expect(list.map((message) => message.id)).toEqual([first.id]);
    expect(pendingRef.current.map((item) => item.message.id)).toEqual([second.id]);

    expect(drainPendingMessageUpdates(pendingRef, update)).toBe(true);
    expect(list.map((message) => message.id)).toEqual([first.id, second.id]);
    expect(drainPendingMessageUpdates(pendingRef, update)).toBe(false);
  });

  test('does not let an empty stale read erase a same-conversation optimistic message', () => {
    const sent = textMessage(conversationA, '01', 'visible immediately', 1);

    const merged = mergeFetchedMessagesForConversation([sent], [], conversationA);

    expect(merged).toEqual([sent]);
  });

  test('replaces another conversation transcript with the requested conversation readback', () => {
    const previousConversationMessage = textMessage(conversationB, '02', 'conversation B', 1);
    const requestedConversationMessage = textMessage(conversationA, '01', 'conversation A', 2);

    const merged = mergeFetchedMessagesForConversation(
      [previousConversationMessage],
      [requestedConversationMessage],
      conversationA
    );

    expect(merged).toEqual([requestedConversationMessage]);
  });

  test('cleanup cancels the timer before synchronously draining without re-arming it', () => {
    const hookStart = source.indexOf('export const useAddOrUpdateMessage');
    const hookEnd = source.indexOf('export const useKnowledgeWritebackEvents', hookStart);
    const hookSource = source.slice(hookStart, hookEnd);
    const cleanupStart = hookSource.indexOf('useEffect(() =>');
    const cleanupEnd = hookSource.indexOf('return useCallback(', cleanupStart);
    const cleanupSource = hookSource.slice(cleanupStart, cleanupEnd);

    expect(cleanupSource.indexOf('clearTimeout(rafRef.current)')).toBeGreaterThanOrEqual(0);
    expect(cleanupSource.indexOf('rafRef.current = null')).toBeGreaterThan(
      cleanupSource.indexOf('clearTimeout(rafRef.current)')
    );
    expect(cleanupSource.indexOf('flush();')).toBeGreaterThan(cleanupSource.indexOf('rafRef.current = null'));
    expect(hookSource.match(/rafRef\.current = setTimeout\(flush\)/g)).toHaveLength(1);
  });

  test('rejects an old conversation response before it can merge into the active list', () => {
    const loadStart = source.indexOf('const loadMessages = useCallback');
    const loadEnd = source.indexOf('// Prepend the next older window', loadStart);
    const loadSource = source.slice(loadStart, loadEnd);
    const activeGuard = loadSource.indexOf('activeConversationRef.current !== key');
    const sequenceGuard = loadSource.indexOf('newestLoadSequenceRef.current !== loadSequence');
    const merge = loadSource.indexOf('mergeIntoList(messages)');

    expect(activeGuard).toBeGreaterThanOrEqual(0);
    expect(sequenceGuard).toBeGreaterThan(activeGuard);
    expect(merge).toBeGreaterThan(sequenceGuard);
  });
});
