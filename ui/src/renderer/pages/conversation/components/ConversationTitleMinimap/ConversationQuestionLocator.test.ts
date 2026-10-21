/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { getDotDistanceLevel, pickActiveQuestionIndex } from './ConversationQuestionLocator';
import { buildTurnPreview } from './minimapUtils';
import type { IMessageText } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';

test('question navigation skips queued inputs and retains a current-task steering question', () => {
  const conversation = parseConversationId('0190f5fe-7c00-7a00-8000-000000000981');
  const message = (suffix: string, content: string, position: 'left' | 'right', placement?: 'queued' | 'steering'): IMessageText => ({
    id: suffix, msg_id: parseMessageId(`0190f5fe-7c00-7a00-8000-${suffix.padStart(12, '0')}`), conversation_id: conversation,
    type: 'text', position, content: { content, ...(placement ? { input_delivery: { state: 'pending', placement } } : {}) },
  });
  const preview = buildTurnPreview([
    message('1', 'Original question', 'right'),
    message('2', 'Later queued question', 'right', 'queued'),
    message('3', 'Original answer', 'left'),
    message('4', 'Correct this task', 'right', 'steering'),
    message('5', 'Corrected answer', 'left'),
  ]);
  expect(preview.map(turn => turn.question)).toEqual(['Original question', 'Correct this task']);
  expect(preview.map(turn => turn.index)).toEqual([1, 2]);
  expect(preview[0].answer).toBe('Original answer');
});

describe('pickActiveQuestionIndex', () => {
  test('chooses the latest question above the viewport anchor', () => {
    expect(pickActiveQuestionIndex([-260, -24, 180, 420], 140)).toBe(1);
  });

  test('chooses the first question when every question is below the anchor', () => {
    expect(pickActiveQuestionIndex([180, 420], 140)).toBe(0);
  });

  test('returns -1 when no question anchors are available', () => {
    expect(pickActiveQuestionIndex([], 140)).toBe(-1);
  });
});

describe('getDotDistanceLevel', () => {
  test('marks the active question as level 0', () => {
    expect(getDotDistanceLevel(3, 3)).toBe(0);
  });

  test('marks direct neighbors as level 1', () => {
    expect(getDotDistanceLevel(2, 3)).toBe(1);
    expect(getDotDistanceLevel(4, 3)).toBe(1);
  });

  test('marks second neighbors as level 2 and farther dots as level 3', () => {
    expect(getDotDistanceLevel(1, 3)).toBe(2);
    expect(getDotDistanceLevel(5, 3)).toBe(2);
    expect(getDotDistanceLevel(0, 3)).toBe(3);
    expect(getDotDistanceLevel(8, 3)).toBe(3);
  });
});
