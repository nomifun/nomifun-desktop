/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { parseProviderId } from '@/common/types/ids';

import type { CreativeChatSessionReference } from '../../../domain';
import type { CreativeStudioAgentMessage } from '../../../agent';
import {
  classifyCreativeCanvasAgentHistory,
  createCreativeCanvasAgentSession,
  creativeCanvasAgentModelSelection,
  creativeCanvasAgentSessionWithAuthoritativeHistory,
  creativeCanvasAgentSessionWithPendingTurn,
  creativeCanvasAgentSessionWithoutPendingTurn,
  replaceCreativeCanvasAgentSession,
} from './model';

const sessionId = '0190f5fe-7c00-7a00-8000-000000000201';
const idempotencyKey = '0190f5fe-7c00-7a00-8000-000000000202';
const providerId = '0190f5fe-7c00-7a00-8000-000000000203';
const userMessageId = '0190f5fe-7c00-7a00-8000-000000000204';
const assistantMessageId = '0190f5fe-7c00-7a00-8000-000000000205';
const model = { providerId: parseProviderId(providerId), model: 'nomi-chat' };
const history: CreativeStudioAgentMessage[] = [
  { id: userMessageId, role: 'user', status: 'complete', text: '制作一个分镜' },
  { id: assistantMessageId, role: 'assistant', status: 'complete', text: '先确认画面目标。' },
];

const captureError = (run: () => void): Error | null => {
  try {
    run();
    return null;
  } catch (error) {
    return error instanceof Error ? error : new Error(String(error));
  }
};

describe('Creative Canvas Agent document model', () => {
  test('creates a session and persists the first pending turn with an immutable model', () => {
    const empty = createCreativeCanvasAgentSession(sessionId, 10);
    const pending = creativeCanvasAgentSessionWithPendingTurn({
      session: empty,
      model,
      idempotencyKey,
      prompt: '  制作一个分镜  ',
      now: 20,
    });

    expect(empty).toEqual({
      id: sessionId,
      title: '新对话',
      messageIds: [],
      model: null,
      pendingTurn: null,
      createdAt: 10,
      updatedAt: 10,
    });
    expect(pending.title).toBe('制作一个分镜');
    expect(pending.model).toEqual({ providerId, model: 'nomi-chat' });
    expect(pending.pendingTurn).toEqual({
      idempotencyKey,
      prompt: '制作一个分镜',
      createdAt: 20,
    });
    expect(creativeCanvasAgentModelSelection(pending.model)).toEqual(model);

    const differentModelError = captureError(() =>
      creativeCanvasAgentSessionWithPendingTurn({
        session: { ...pending, pendingTurn: null },
        model: { ...model, model: 'other-chat' },
        idempotencyKey,
        prompt: '继续',
        now: 30,
      })
    );
    expect(differentModelError?.message.includes('immutable')).toBe(true);
  });

  test('accepts only the current durable prefix or one completed pending pair', () => {
    const pending = creativeCanvasAgentSessionWithPendingTurn({
      session: createCreativeCanvasAgentSession(sessionId, 10),
      model,
      idempotencyKey,
      prompt: '制作一个分镜',
      now: 20,
    });

    expect(classifyCreativeCanvasAgentHistory(pending, [])).toBe('current');
    expect(classifyCreativeCanvasAgentHistory(pending, history)).toBe(
      'completed-pending-turn'
    );
    const invalidProjection = captureError(() =>
      classifyCreativeCanvasAgentHistory(pending, history.slice(0, 1))
    );
    expect(invalidProjection?.message.includes('invalid pending-turn')).toBe(true);
  });

  test('reconciles authoritative ids, clears fences, and replaces only the matching session', () => {
    const pending = creativeCanvasAgentSessionWithPendingTurn({
      session: createCreativeCanvasAgentSession(sessionId, 10),
      model,
      idempotencyKey,
      prompt: '制作一个分镜',
      now: 20,
    });
    const completed = creativeCanvasAgentSessionWithAuthoritativeHistory(
      pending,
      history,
      30
    );
    const other: CreativeChatSessionReference = {
      ...createCreativeCanvasAgentSession(
        '0190f5fe-7c00-7a00-8000-000000000206',
        11
      ),
      title: '另一个会话',
    };

    expect(completed.messageIds).toEqual([userMessageId, assistantMessageId]);
    expect(completed.pendingTurn).toBeNull();
    expect(creativeCanvasAgentSessionWithoutPendingTurn(pending, 31).pendingTurn).toBeNull();
    expect(replaceCreativeCanvasAgentSession([pending, other], completed)).toEqual([
      completed,
      other,
    ]);
  });
});
