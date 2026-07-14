/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { isOrdinaryWorkConversation } from './conversationListFilter';

describe('ordinary conversation list ownership', () => {
  test('retained execution attempt transcripts stay out of the ordinary list', () => {
    const transcript = {
      execution_step_id: 'step_1',
      extra: {},
    };

    expect(isOrdinaryWorkConversation(transcript as never)).toBe(false);
  });

  test('ordinary conversations remain visible', () => {
    const conversation = {
      execution_step_id: undefined,
      extra: {},
    };

    expect(isOrdinaryWorkConversation(conversation as never)).toBe(true);
  });
});
