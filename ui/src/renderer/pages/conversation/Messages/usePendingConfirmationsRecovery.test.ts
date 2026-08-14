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

const genericPermission = (
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

const acpPermission = (
  id: string,
  callId: string,
  turnId?: MessageId
): TMessage => ({
  id,
  type: 'acp_permission',
  position: 'left',
  conversation_id: conversationId,
  created_at: 1,
  ...(turnId ? { turn_id: turnId } : {}),
  content: {
    session_id: 'session-1',
    options: [],
    tool_call: {
      tool_call_id: callId,
      title: 'Run command',
    },
  },
});

describe('pending confirmation message recovery', () => {
  test('deduplicates durable confirmations against both permission wire shapes', () => {
    expect(hasPermissionMessageForCallId([genericPermission('generic', 'call-1')], 'call-1')).toBe(
      true
    );
    expect(hasPermissionMessageForCallId([acpPermission('acp', 'call-2')], 'call-2')).toBe(true);
  });

  test('removes both permission wire shapes by their shared call id', () => {
    const remaining = removePermissionMessage(
      [
        genericPermission('generic', 'shared-call'),
        acpPermission('acp', 'shared-call'),
        acpPermission('unrelated', 'other-call'),
      ],
      { call_id: 'shared-call' }
    );

    expect(remaining.map((message) => message.id)).toEqual(['unrelated']);
  });

  test('turn completion removes only permission cards owned by that exact turn', () => {
    const remaining = removePermissionMessagesForTurn(
      [
        genericPermission('completed-generic', 'same-call', completedTurnId),
        acpPermission('completed-acp', 'other-call', completedTurnId),
        acpPermission('next-turn', 'same-call', nextTurnId),
        genericPermission('confirmation:turnless', 'same-call'),
      ],
      completedTurnId
    );

    expect(remaining.map((message) => message.id)).toEqual([
      'next-turn',
      'confirmation:turnless',
    ]);
  });
});
