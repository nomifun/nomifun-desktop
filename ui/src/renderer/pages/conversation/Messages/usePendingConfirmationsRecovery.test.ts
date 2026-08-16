/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId, type MessageId } from '@/common/types/ids';
import {
  hasPermissionMessageForCallId,
  removePermissionMessage,
  removePermissionMessagesForTurn,
} from './usePendingConfirmationsRecovery';

const conversationId = parseConversationId('019b0000-0000-7000-8000-000000000001');
const completedTurnId = parseMessageId('019b0000-0000-7000-8000-000000000002');
const nextTurnId = parseMessageId('019b0000-0000-7000-8000-000000000003');

const permission = (
  id: string,
  callId: string,
  turnId?: MessageId
): TMessage => ({
  id,
  type: 'permission',
  position: 'left',
  conversation_id: conversationId,
  created_at: 1,
  ...(turnId ? { turn_id: turnId } : {}),
  content: {
    id: callId,
    call_id: callId,
    description: 'Allow tool execution',
    options: [],
  },
});

describe('pending confirmation message recovery', () => {
  test('deduplicates durable confirmations by their call id', () => {
    expect(hasPermissionMessageForCallId([permission('raised', 'call-1')], 'call-1')).toBe(true);
    expect(hasPermissionMessageForCallId([permission('raised', 'call-1')], 'call-2')).toBe(false);
  });

  test('removes permission cards by their call id', () => {
    const remaining = removePermissionMessage(
      [permission('shared', 'shared-call'), permission('unrelated', 'other-call')],
      { call_id: 'shared-call' }
    );

    expect(remaining.map((message) => message.id)).toEqual(['unrelated']);
  });

  test('turn completion removes only permission cards owned by that exact turn', () => {
    const remaining = removePermissionMessagesForTurn(
      [
        permission('completed-first', 'same-call', completedTurnId),
        permission('completed-second', 'other-call', completedTurnId),
        permission('next-turn', 'same-call', nextTurnId),
        permission('confirmation:turnless', 'same-call'),
      ],
      completedTurnId
    );

    expect(remaining.map((message) => message.id)).toEqual([
      'next-turn',
      'confirmation:turnless',
    ]);
  });
});
