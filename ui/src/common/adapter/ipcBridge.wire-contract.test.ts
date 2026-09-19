/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { fromApiTurnCompletedEvent } from './ipcBridge';
import { isAuthoritativeCompletionRuntimeIdle } from '@/renderer/pages/conversation/platforms/authoritativeTurnLifecyclePolicy';

const source = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');
const CONVERSATION_ID = '0190f5fe-7c00-7a00-8000-000000000001';
const MESSAGE_ID = '0190f5fe-7c00-7a00-8000-000000000002';

describe('ipc bridge wire ID contracts', () => {
  test('revoke user uses channel_user_id and not user_id', () => {
    expect(source.includes('revokeUser: httpPost<void, { channel_user_id:')).toBe(true);
    expect(source.includes("'/api/channel/users/revoke'")).toBe(true);
    expect(source.includes('revokeUser: httpPost<void, { user_id:')).toBe(false);
  });

  test('group access maps fail-closed status and posts the independent policy payload', () => {
    expect(source.includes('groupAccessMode: normalizeGroupAccessMode(raw.group_access_mode)')).toBe(
      true
    );
    expect(source.includes('setGroupAccess: httpPost<void, SetGroupAccessRequest>')).toBe(true);
    expect(source.includes("'/api/channel/settings/group-access'")).toBe(true);
  });

  test('channel disable and delete reject HTTP-200 business failures', () => {
    expect(source.includes('function requireSuccessfulChannelResponse')).toBe(true);
    expect(source.includes('disablePlugin: withResponseMap(')).toBe(true);
    expect(source.includes('deletePlugin: withResponseMap(')).toBe(true);
    expect(source.includes("throw new Error(raw.error || raw.message || 'Channel operation failed')")).toBe(
      true
    );
  });

  test('turn.completed last_message uses message_id and rejects generic id', () => {
    expect(source.includes('message_id?: MessageId;')).toBe(true);
    expect(source.includes('last_message legacy field "id" is not accepted')).toBe(true);
    expect(source.includes('          id: rawLast.id')).toBe(false);
  });

  test('maps message_id and rejects last_message.id at runtime', () => {
    const mapped = fromApiTurnCompletedEvent({
      conversation_id: CONVERSATION_ID,
      turn_id: MESSAGE_ID,
      last_message: {
        message_id: MESSAGE_ID,
        content: 'done',
        created_at: 1,
      },
    });
    expect(mapped.last_message.message_id).toBe(MESSAGE_ID);
    expect(mapped.runtime.is_processing).toBe(true);

    let rejected = false;
    try {
      fromApiTurnCompletedEvent({
        conversation_id: CONVERSATION_ID,
        turn_id: MESSAGE_ID,
        last_message: {
          id: MESSAGE_ID,
          content: 'legacy',
          created_at: 1,
        },
      });
    } catch {
      rejected = true;
    }
    expect(rejected).toBe(true);
  });

  test('does not expose the legacy mutable knowledge-writeback bridge', () => {
    expect(source.includes('retryKnowledgeWriteback:')).toBe(false);
    expect(source.includes('/knowledge-writeback/retry')).toBe(false);
  });

  test('maps exact active_turn_id from a turn lifecycle runtime snapshot', () => {
    const mapped = fromApiTurnCompletedEvent({
      conversation_id: CONVERSATION_ID,
      turn_id: MESSAGE_ID,
      runtime: {
        state: 'idle',
        is_processing: false,
        active_turn_id: MESSAGE_ID,
      },
      last_message: {
        message_id: MESSAGE_ID,
        content: 'done',
        created_at: 1,
      },
    });

    expect(mapped.runtime.active_turn_id).toBe(MESSAGE_ID);
  });

  test('does not default a missing terminal runtime field to released', () => {
    const mapped = fromApiTurnCompletedEvent({
      conversation_id: CONVERSATION_ID,
      turn_id: MESSAGE_ID,
      runtime: {
        state: 'idle',
      },
    });

    expect(mapped.runtime.is_processing).toBe(true);
  });

  test('accepts the canonical relay terminal only with explicit idle runtime authority', () => {
    const mapped = fromApiTurnCompletedEvent({
      conversation_id: CONVERSATION_ID,
      turn_id: MESSAGE_ID,
      status: 'finished',
      state: 'ai_waiting_input',
      can_send_message: true,
      runtime: {
        state: 'idle',
        can_send_message: true,
        has_runtime: false,
        runtime_status: 'finished',
        is_processing: false,
        active_turn_id: null,
      },
    });

    expect(mapped.turn_id).toBe(MESSAGE_ID);
    expect(isAuthoritativeCompletionRuntimeIdle(mapped.runtime)).toBe(true);
  });
});
