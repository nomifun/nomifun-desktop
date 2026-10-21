import { describe, expect, test } from 'bun:test';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { warmupConversationForPassiveMount } from '../utils/warmupConversation';
import {
  classifyAuthoritativeTurnStart,
  resolveVerifiedAuthoritativeTurnStart,
} from './authoritativeTurnLifecyclePolicy';
import {
  getNomiHydrationLifecycleFence,
  shouldApplyNomiStreamEventToTurn,
} from './nomi/nomiLifecycleFence';
const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000071');
const completedTurnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000072');

const finishedSnapshot = {
  id: conversationId,
  status: 'finished',
  runtime: {
    state: 'idle',
    can_send_message: true,
    is_processing: false,
  },
} as TChatConversation;

describe('Finished conversation remount authority', () => {
  test('late old started/stream/completed plus remount stays idle and sends no POST', async () => {
    let warmupPostCount = 0;

    // Passive mount first performs an authoritative GET. Finished is a
    // terminal snapshot and cannot trigger runtime preparation.
    expect(
      await warmupConversationForPassiveMount(conversationId, {
        getConversation: async () => finishedSnapshot,
        warmup: async () => {
          warmupPostCount += 1;
        },
      })
    ).toBe(false);

    const nomiFence = getNomiHydrationLifecycleFence(false);

    // Delayed old output cannot raise the active runtime's lifecycle fence.
    expect(
      shouldApplyNomiStreamEventToTurn({
        eventTurnId: completedTurnId,
        activeTurnId: null,
        turnClosed: nomiFence.turnClosed,
        awaitingBackendTurn: false,
      })
    ).toBe(false);
    // A delayed old start remains behind exact active_turn_id verification.
    expect(
      classifyAuthoritativeTurnStart({
        turnId: completedTurnId,
        activeTurnId: null,
        cancelledTurnIds: new Set(),
        rejectUnannouncedStart: false,
        awaitingBackendTurn: false,
        verifyUnannouncedStartRuntime: true,
      })
    ).toBe('verify_runtime');
    expect(
      resolveVerifiedAuthoritativeTurnStart({
        turnId: completedTurnId,
        runtimeIsProcessing: finishedSnapshot.runtime?.is_processing === true,
        eventActiveTurnId: completedTurnId,
        runtimeActiveTurnId: finishedSnapshot.runtime?.active_turn_id,
      })
    ).toBe('ignore');

    expect(warmupPostCount).toBe(0);
  });
});
