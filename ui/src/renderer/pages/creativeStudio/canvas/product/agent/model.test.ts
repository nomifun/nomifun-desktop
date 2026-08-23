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
      modelInput: '制作一个分镜',
      skillIds: [],
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

  test('strips selector presentation metadata before persisting the model identity', () => {
    const selectorModel = {
      ...model,
      providerName: 'Display-only provider',
      platform: 'custom',
      task: 'chat',
      traits: [],
      protocol: 'openai.chat_text',
    };
    const pending = creativeCanvasAgentSessionWithPendingTurn({
      session: createCreativeCanvasAgentSession(sessionId, 10),
      model: selectorModel,
      idempotencyKey,
      prompt: '制作一个分镜',
      now: 20,
    });

    expect(pending.model).toEqual(model);
    expect(Object.keys(pending.model ?? {})).toEqual(['providerId', 'model']);
  });

  test('copies exact planning input while deriving the title only from the user prompt', () => {
    const skillIds = ['canvas.inspect', 'asset-search_v2'];
    const pending = creativeCanvasAgentSessionWithPendingTurn({
      session: createCreativeCanvasAgentSession(sessionId, 10),
      model,
      idempotencyKey,
      prompt: '  用户可见标题  ',
      modelInput: 'System planning envelope\n\n用户可见标题',
      skillIds,
      now: 20,
    });
    skillIds.push('mutated.after.creation');

    expect(pending.title).toBe('用户可见标题');
    expect(pending.pendingTurn).toEqual({
      idempotencyKey,
      prompt: '用户可见标题',
      modelInput: 'System planning envelope\n\n用户可见标题',
      skillIds: ['canvas.inspect', 'asset-search_v2'],
      createdAt: 20,
    });
  });

  test('strictly rejects invalid explicit planning input and accepts exact boundaries', () => {
    const boundary = creativeCanvasAgentSessionWithPendingTurn({
      session: createCreativeCanvasAgentSession(sessionId, 10),
      model,
      idempotencyKey,
      prompt: '边界',
      modelInput: 'x'.repeat(262_144),
      skillIds: ['a'.repeat(128)],
      now: 20,
    });
    expect(boundary.pendingTurn?.modelInput.length).toBe(262_144);
    expect(boundary.pendingTurn?.skillIds).toEqual(['a'.repeat(128)]);

    const invalidCases: Array<{
      modelInput?: unknown;
      skillIds?: unknown;
      message: string;
    }> = [
      { modelInput: '', message: 'model input' },
      { modelInput: ' padded ', message: 'model input' },
      { modelInput: 'x'.repeat(262_145), message: 'model input' },
      { modelInput: null, message: 'model input' },
      { skillIds: null, message: 'skill ids' },
      {
        skillIds: Array.from({ length: 9 }, (_, index) => `skill-${index}`),
        message: 'at most 8',
      },
      { skillIds: ['a'.repeat(129)], message: 'skill id 0' },
      { skillIds: [' canvas.read'], message: 'skill id 0' },
      { skillIds: ['canvas/read'], message: 'skill id 0' },
      { skillIds: ['canvas.检查'], message: 'skill id 0' },
      { skillIds: ['canvas.read', 'canvas.read'], message: 'unique' },
    ];

    for (const invalid of invalidCases) {
      const error = captureError(() =>
        creativeCanvasAgentSessionWithPendingTurn({
          session: createCreativeCanvasAgentSession(sessionId, 10),
          model,
          idempotencyKey,
          prompt: '继续',
          ...(invalid.modelInput === undefined
            ? {}
            : { modelInput: invalid.modelInput as string }),
          ...(invalid.skillIds === undefined
            ? {}
            : { skillIds: invalid.skillIds as readonly string[] }),
          now: 20,
        })
      );
      expect(error?.message.includes(invalid.message)).toBe(true);
    }
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
