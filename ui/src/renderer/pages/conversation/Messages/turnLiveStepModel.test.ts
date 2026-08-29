/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { planTurnLiveStep } from './turnLiveStepModel';

const disclosure = (
  running: boolean,
  processItems: Array<{ id: string; state: 'completed' | 'running' | 'failed' | 'canceled' }>
) => ({ running, processItems });

describe('planTurnLiveStep', () => {
  test('hidden while the conversation is not processing', () => {
    expect(
      planTurnLiveStep({
        isProcessing: false,
        disclosure: disclosure(true, [{ id: 'a', state: 'running' }]),
        hasStreamingReplyText: false,
      })
    ).toBeNull();
  });

  test('hidden when the tail turn has settled', () => {
    expect(
      planTurnLiveStep({
        isProcessing: true,
        disclosure: disclosure(false, [{ id: 'a', state: 'completed' }]),
        hasStreamingReplyText: false,
      })
    ).toBeNull();
  });

  test('latest running item is the current step', () => {
    expect(
      planTurnLiveStep({
        isProcessing: true,
        disclosure: disclosure(true, [
          { id: 'a', state: 'completed' },
          { id: 'b', state: 'running' },
        ]),
        hasStreamingReplyText: false,
      })
    ).toEqual({ kind: 'item', itemId: 'b', state: 'running' });
  });

  test('streaming reply text without running items composes the reply', () => {
    expect(
      planTurnLiveStep({
        isProcessing: true,
        disclosure: disclosure(true, [{ id: 'a', state: 'completed' }]),
        hasStreamingReplyText: true,
      })
    ).toEqual({ kind: 'composing', state: 'running' });
  });

  test('fresh turn with no process rows analyzes the request', () => {
    expect(
      planTurnLiveStep({ isProcessing: true, disclosure: disclosure(true, []), hasStreamingReplyText: false })
    ).toEqual({ kind: 'analyzing', state: 'running' });
  });

  test('gap between steps prepares the next action', () => {
    expect(
      planTurnLiveStep({
        isProcessing: true,
        disclosure: disclosure(true, [{ id: 'a', state: 'completed' }]),
        hasStreamingReplyText: false,
      })
    ).toEqual({ kind: 'preparing', state: 'running' });
  });

  test('hidden without a tail disclosure', () => {
    expect(planTurnLiveStep({ isProcessing: true, hasStreamingReplyText: false })).toBeNull();
  });
});
