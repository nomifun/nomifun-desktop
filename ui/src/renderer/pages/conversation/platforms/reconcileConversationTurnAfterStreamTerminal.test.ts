import { describe, expect, test } from 'bun:test';
import { BackendRequestError } from '@/common/adapter/httpBridge';
import type { TChatConversation } from '@/common/config/storage';
import type { ConversationId } from '@/common/types/ids';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { emitter } from '@/renderer/utils/emitter';
import { getConversationPauseNotice } from '../utils/conversationRuntime';
import {
  AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS,
  TERMINAL_RECONCILE_DELAYS_MS,
  reconcileConversationAuthoritativeRuntime,
  reconcileConversationTurnAfterAcceptedReplay,
  reconcileConversationTurnAfterStreamTerminal,
  terminalReconcileDelayForAttempt,
} from './reconcileConversationTurnAfterStreamTerminal';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000041');
const activeTurnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000042');
const idleConversation = {
  status: 'finished',
  runtime: { is_processing: false },
} as TChatConversation;
const busyConversation = {
  status: 'running',
  runtime: { is_processing: true, active_turn_id: activeTurnId },
} as TChatConversation;
const unknownConversation = {
  status: 'running',
  runtime: { is_processing: true },
} as TChatConversation;

describe('terminal stream runtime reconciliation', () => {
  test('a durable pause stops activity without settling or opening the queue', async () => {
    const paused = {
      status: 'running',
      extra: { execution_phase: 'paused' },
      runtime: { state: 'idle', is_processing: false, can_send_message: false, active_turn_id: activeTurnId },
    } as TChatConversation;
    let pauseCalls = 0;
    let idleCalls = 0;
    let unknownCalls = 0;
    const settled: ConversationId[] = [];
    const onSettled = (id: ConversationId) => { settled.push(id); };
    emitter.on('conversation.turn.settled', onSettled);
    try {
      const options = {
        isCurrent: () => true,
        onIdle: () => { idleCalls += 1; },
        onUnknown: () => { unknownCalls += 1; },
        onPaused: (conversation: TChatConversation) => {
          pauseCalls += 1;
          expect(conversation.runtime?.active_turn_id).toBe(activeTurnId);
        },
        delaysMs: [0], getConversation: async () => paused, retryForever: false,
      };
      expect(await reconcileConversationAuthoritativeRuntime(conversationId, options)).toBe(false);
      expect(pauseCalls).toBe(1);
      expect(idleCalls).toBe(0);
      expect(unknownCalls).toBe(0);
      expect(settled).toEqual([]);
    } finally { emitter.off('conversation.turn.settled', onSettled); }
  });

  test('each explicit resync immediately adopts its exact canonical pause snapshot', async () => {
    const unproven = {
      status: 'running',
      extra: { execution_phase: 'paused', execution_pause: {
        reason: 'EXECUTION_MODEL_RATE_LIMITED', cleanup_proven: false, paused_at_ms: 123,
      } },
      runtime: { state: 'idle', is_processing: false, can_send_message: false, active_turn_id: activeTurnId },
    } as TChatConversation;
    const proven = { ...unproven, extra: { ...unproven.extra, execution_pause: {
      ...unproven.extra!.execution_pause!, cleanup_proven: true,
    } } } as TChatConversation;
    const snapshots = [unproven, proven];
    const observed: TChatConversation[] = [];

    const options = {
      isCurrent: () => true,
      onIdle: () => { throw new Error('pause must not settle the turn'); },
      onPaused: (conversation: TChatConversation) => { observed.push(conversation); },
      delaysMs: [0, 0],
      getConversation: async () => snapshots.shift() ?? proven,
      retryForever: false,
      announceSettled: false,
    };
    expect(await reconcileConversationAuthoritativeRuntime(conversationId, options)).toBe(false);
    expect(observed).toEqual([unproven]);
    expect(snapshots).toHaveLength(1);
    expect(await reconcileConversationAuthoritativeRuntime(conversationId, options)).toBe(false);
    expect(observed).toEqual([unproven, proven]);
    expect(snapshots).toHaveLength(0);
  });

  test('an unproven canonical pause immediately surfaces cleanupRequired', async () => {
    const paused = {
      status: 'running',
      extra: { execution_phase: 'paused', execution_pause: {
        reason: 'EXECUTION_PAUSED', cleanup_proven: false, paused_at_ms: 456,
      } },
      runtime: { state: 'idle', is_processing: false, can_send_message: false, active_turn_id: activeTurnId },
    } as TChatConversation;
    let reads = 0;
    let observed: TChatConversation | undefined;

    expect(await reconcileConversationAuthoritativeRuntime(conversationId, {
      isCurrent: () => true,
      onIdle: () => { throw new Error('pause must not settle the turn'); },
      onPaused: conversation => { observed = conversation; },
      delaysMs: [0, 0, 0],
      getConversation: async () => { reads += 1; return paused; },
      retryForever: false,
      announceSettled: false,
    })).toBe(false);
    expect(reads).toBe(1);
    expect(getConversationPauseNotice(observed)).toMatchObject({ cleanupProven: false });
  });

  test('reconnect settles a turn whose terminal events were lost after turn.started', async () => {
    let running = true;
    let correlatedTurnId: typeof activeTurnId | undefined = activeTurnId;

    const result = await reconcileConversationAuthoritativeRuntime(conversationId, {
      isCurrent: () => true,
      onIdle: () => {
        running = false;
        correlatedTurnId = undefined;
      },
      onProcessing: (conversation) => {
        correlatedTurnId = conversation.runtime!.active_turn_id!;
      },
      delaysMs: [0],
      getConversation: async () => idleConversation,
      retryForever: false,
      announceSettled: false,
    });

    expect(result).toBe(true);
    expect(running).toBe(false);
    expect(correlatedTurnId).toBeUndefined();
  });

  test('reconnect adopts the same active turn and keeps polling until idle', async () => {
    const snapshots = [busyConversation, idleConversation];
    const adoptedTurnIds = [] as Array<typeof activeTurnId>;
    let running = true;

    const result = await reconcileConversationAuthoritativeRuntime(conversationId, {
      isCurrent: () => true,
      onIdle: () => {
        running = false;
      },
      onProcessing: (conversation) => {
        adoptedTurnIds.push(conversation.runtime!.active_turn_id!);
      },
      delaysMs: [0, 0],
      getConversation: async () => snapshots.shift() ?? idleConversation,
      retryForever: false,
      announceSettled: false,
    });

    expect(result).toBe(true);
    expect(adoptedTurnIds).toEqual([activeTurnId]);
    expect(running).toBe(false);
  });

  test('one BackendRequestError and one unknown projection recover without rejecting', async () => {
    const snapshots: Array<TChatConversation | Error> = [
      new BackendRequestError('network', 'connection reset'),
      unknownConversation,
      idleConversation,
    ];
    let unknownCalls = 0;
    let idleCalls = 0;

    const result = await reconcileConversationAuthoritativeRuntime(conversationId, {
      isCurrent: () => true,
      onIdle: () => {
        idleCalls += 1;
      },
      onUnknown: () => {
        unknownCalls += 1;
      },
      delaysMs: [0, 0, 0],
      getConversation: async () => {
        const snapshot = snapshots.shift() ?? idleConversation;
        if (snapshot instanceof Error) throw snapshot;
        return snapshot;
      },
      retryForever: false,
      announceSettled: false,
    });

    expect(result).toBe(true);
    expect(unknownCalls).toBe(1);
    expect(idleCalls).toBe(1);
    expect(snapshots).toHaveLength(0);
  });

  test('reconnect schedule performs its first authority read immediately', () => {
    expect(AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS[0]).toBe(0);
    expect(AUTHORITATIVE_RUNTIME_RESYNC_DELAYS_MS.at(-1)).toBe(16_000);
  });

  test('times out a hung read and advances to a later authoritative idle read', async () => {
    let reads = 0;
    let idleCalls = 0;
    const result = await reconcileConversationTurnAfterStreamTerminal(
      conversationId,
      () => true,
      () => {
        idleCalls += 1;
      },
      [0, 0],
      async () => {
        reads += 1;
        if (reads === 1) return new Promise<never>(() => {});
        return idleConversation;
      },
      5
    );

    expect(result).toBe(true);
    expect(reads).toBe(2);
    expect(idleCalls).toBe(1);
  });

  test('reuses the capped production delay after the initial schedule is exhausted', () => {
    expect(terminalReconcileDelayForAttempt(0)).toBe(TERMINAL_RECONCILE_DELAYS_MS[0]);
    expect(terminalReconcileDelayForAttempt(TERMINAL_RECONCILE_DELAYS_MS.length - 1)).toBe(16_000);
    expect(terminalReconcileDelayForAttempt(TERMINAL_RECONCILE_DELAYS_MS.length + 100)).toBe(16_000);
  });

  test('a forever retry stops when its generation is no longer current', async () => {
    let reads = 0;
    let idleCalls = 0;
    const result = await reconcileConversationTurnAfterStreamTerminal(
      conversationId,
      () => reads < 2,
      () => {
        idleCalls += 1;
      },
      [0],
      async () => {
        reads += 1;
        return busyConversation;
      },
      5,
      true
    );

    expect(result).toBe(false);
    expect(reads).toBe(2);
    expect(idleCalls).toBe(0);
  });

  test('an incomplete runtime projection never settles the current generation', async () => {
    let reads = 0;
    let idleCalls = 0;
    const result = await reconcileConversationTurnAfterStreamTerminal(
      conversationId,
      () => reads < 2,
      () => {
        idleCalls += 1;
      },
      [0],
      async () => {
        reads += 1;
        return unknownConversation;
      },
      5,
      true
    );

    expect(result).toBe(false);
    expect(reads).toBe(2);
    expect(idleCalls).toBe(0);
  });

  test('an accepted replay opens only after a running GET and settles on idle', async () => {
    const snapshots = [busyConversation, idleConversation];
    let processingCalls = 0;
    let idleCalls = 0;

    const result = await reconcileConversationTurnAfterAcceptedReplay(
      conversationId,
      () => true,
      () => {
        processingCalls += 1;
      },
      () => {
        idleCalls += 1;
      },
      [0, 0],
      async () => snapshots.shift() ?? idleConversation,
      5,
      false
    );

    expect(result).toBe(true);
    expect(processingCalls).toBe(1);
    expect(idleCalls).toBe(1);
  });

  test('a successful idle reconciliation announces conversation.turn.settled', async () => {
    const settled: ConversationId[] = [];
    const onSettled = (settledConversationId: ConversationId) => {
      settled.push(settledConversationId);
    };
    emitter.on('conversation.turn.settled', onSettled);

    try {
      const result = await reconcileConversationTurnAfterStreamTerminal(
        conversationId,
        () => true,
        () => {},
        [0],
        async () => idleConversation,
        5,
        false
      );

      expect(result).toBe(true);
      expect(settled).toEqual([conversationId]);
    } finally {
      emitter.off('conversation.turn.settled', onSettled);
    }
  });

  test('an exhausted reconciliation never announces conversation.turn.settled', async () => {
    const settled: ConversationId[] = [];
    const onSettled = (settledConversationId: ConversationId) => {
      settled.push(settledConversationId);
    };
    emitter.on('conversation.turn.settled', onSettled);

    try {
      const result = await reconcileConversationTurnAfterStreamTerminal(
        conversationId,
        () => true,
        () => {},
        [0],
        async () => busyConversation,
        5,
        false
      );

      expect(result).toBe(false);
      expect(settled).toEqual([]);
    } finally {
      emitter.off('conversation.turn.settled', onSettled);
    }
  });
});
