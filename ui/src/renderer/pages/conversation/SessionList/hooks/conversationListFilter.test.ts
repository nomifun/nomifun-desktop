/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseExecutionStepId } from '@/common/types/ids';

import { isOrdinaryWorkConversation } from './conversationListFilter';

describe('ordinary conversation list ownership', () => {
  test('retained execution attempt transcripts stay out of the ordinary list', () => {
    const transcript = {
      execution_step_id: parseExecutionStepId('0190f5fe-7c00-7a00-8000-000000000001'),
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

  test('robot sessions never enter the ordinary work list', () => {
    // A robot thread is a long-lived companion conversation owned by a device.
    // It is excluded EXPLICITLY rather than incidentally via `companion_id`:
    // that marker is what the companion group already keys on, and relying on it
    // would silently break the day a robot thread stops carrying it.
    const robotSession = {
      execution_step_id: undefined,
      extra: { robot_session: true, robot_id: 'aa:bb:cc:dd:ee:ff' },
    };
    expect(isOrdinaryWorkConversation(robotSession as never)).toBe(false);

    const robotIdOnly = { execution_step_id: undefined, extra: { robot_id: 'aa:bb:cc:dd:ee:ff' } };
    expect(isOrdinaryWorkConversation(robotIdOnly as never)).toBe(false);
  });
});
