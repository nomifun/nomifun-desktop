import { describe, expect, test } from 'bun:test';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { MAX_NOMI_PENDING_POST_PROCESSES } from './nomiMessageBuffer';
import {
  createNomiPostProcessState,
  getNomiPostProcessInFlightWaiters,
  isNomiBackendFinalTextAuthoritative,
  isNomiPostProcessBufferAssociated,
  isNomiPostProcessRequestCurrent,
  rememberNomiPostProcessObservation,
  rememberNomiPostProcessPending,
  shouldHandleNomiTerminalPostProcess,
  takeNomiPostProcessObservationsForBuffer,
  tryRememberNomiInFlightPostProcess,
  type NomiInFlightPostProcess,
} from './nomiPostProcessState';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000001');
const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000002');
const terminalId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000003');
const targetMessageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000004');
const otherMessageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000005');
const generatedMessageId = (sequence: number) =>
  parseMessageId(`0190f5fe-7c00-7a00-8001-${sequence.toString(16).padStart(12, '0')}`);

const request = {
  conversationId,
  terminalId,
  targetMessageId,
  turnId,
  allowTurnFallback: false,
  generation: 1,
  turnStartGeneration: 1,
};

describe('Nomi post-process state associations', () => {
  test('an unresolved legacy terminal associates a late fragment by exact turn', () => {
    const state = createNomiPostProcessState();
    rememberNomiPostProcessPending(state, {
      ...request,
      targetMessageId: undefined,
      allowTurnFallback: true,
    });

    expect(isNomiPostProcessBufferAssociated(state, conversationId, 'late-segment', turnId)).toBe(
      true
    );
  });

  test('a targeted pending request does not capture an unrelated segment in the same turn', () => {
    const state = createNomiPostProcessState();
    rememberNomiPostProcessPending(state, request);

    expect(isNomiPostProcessBufferAssociated(state, conversationId, otherMessageId, turnId)).toBe(
      false
    );
    expect(isNomiPostProcessBufferAssociated(state, conversationId, targetMessageId, turnId)).toBe(
      true
    );
    // The terminal wire id remains a valid association even when the visible
    // text segment has a distinct id.
    expect(isNomiPostProcessBufferAssociated(state, conversationId, terminalId, turnId)).toBe(true);
  });

  test('a completed observation is not retried by reconnect-style pending scans', () => {
    const state = createNomiPostProcessState();
    const observation: NomiInFlightPostProcess = {
      ...request,
      targetMessageId,
      bufferVersion: 3,
    };
    rememberNomiPostProcessObservation(state, observation);

    expect(state.pending.size).toBe(0);
    expect(state.observed.size).toBe(1);
    expect(isNomiPostProcessBufferAssociated(state, conversationId, targetMessageId, turnId)).toBe(
      true
    );
    expect(isNomiPostProcessBufferAssociated(state, conversationId, otherMessageId, turnId)).toBe(
      false
    );
  });

  test('in-flight post-process jobs are capped without evicting running work', () => {
    const state = createNomiPostProcessState();
    for (let index = 0; index < MAX_NOMI_PENDING_POST_PROCESSES; index += 1) {
      const inFlight: NomiInFlightPostProcess = {
        ...request,
        terminalId: generatedMessageId(index + 10),
        targetMessageId: generatedMessageId(index + 1_000),
        bufferVersion: 1,
      };
      expect(tryRememberNomiInFlightPostProcess(state, inFlight)).toBe(true);
    }

    const oldestTerminalId = generatedMessageId(10);
    const overflow: NomiInFlightPostProcess = {
      ...request,
      terminalId: generatedMessageId(10_000),
      targetMessageId: generatedMessageId(10_001),
      bufferVersion: 1,
    };
    rememberNomiPostProcessPending(state, overflow);

    expect(tryRememberNomiInFlightPostProcess(state, overflow)).toBe(false);
    expect(state.inFlight.size).toBe(MAX_NOMI_PENDING_POST_PROCESSES);
    expect(state.inFlight.has(oldestTerminalId)).toBe(true);
    expect(state.pending.get(overflow.terminalId)).toBe(overflow);
    expect(getNomiPostProcessInFlightWaiters(state)).toEqual([overflow]);
  });

  test('ordinary retryable failures are not mistaken for released-slot waiters', () => {
    const state = createNomiPostProcessState();
    const failedRequest = {
      ...request,
      terminalId: generatedMessageId(20_000),
      targetMessageId: generatedMessageId(20_001),
    };
    rememberNomiPostProcessPending(state, failedRequest);

    expect(getNomiPostProcessInFlightWaiters(state)).toEqual([]);
  });

  test('a late fragment promotes an exact completed observation exactly once', () => {
    const state = createNomiPostProcessState();
    rememberNomiPostProcessObservation(state, {
      ...request,
      targetMessageId,
      bufferVersion: 3,
    });

    const promoted = takeNomiPostProcessObservationsForBuffer(
      state,
      conversationId,
      targetMessageId,
      turnId
    );
    expect(promoted).toHaveLength(1);
    expect(promoted[0].terminalId).toBe(terminalId);
    expect(state.observed.size).toBe(0);
    expect(
      takeNomiPostProcessObservationsForBuffer(state, conversationId, targetMessageId, turnId)
    ).toHaveLength(0);
  });

  test('a matching request remains current after the active root has settled', () => {
    expect(
      isNomiPostProcessRequestCurrent(request, {
        mounted: true,
        conversationId,
        generation: 1,
        turnStartGeneration: 1,
        rootTurnId: null,
        lastSettledTurnId: turnId,
      })
    ).toBe(true);
  });

  test('a request from a foreign turn cannot borrow a newer active root', () => {
    expect(
      isNomiPostProcessRequestCurrent(request, {
        mounted: true,
        conversationId,
        generation: 1,
        turnStartGeneration: 1,
        rootTurnId: otherMessageId,
        lastSettledTurnId: turnId,
      })
    ).toBe(false);
  });

  test('a turnless legacy request remains exact-generation fenced', () => {
    expect(
      isNomiPostProcessRequestCurrent(
        { ...request, turnId: undefined },
        {
          mounted: true,
          conversationId,
          generation: 1,
          turnStartGeneration: 1,
          rootTurnId: turnId,
        }
      )
    ).toBe(true);
  });

  test('authoritative metadata on an ordinary frame is not terminal authority', () => {
    // Keep this contract at the helper boundary so a malformed future wire
    // frame cannot discard a local fallback before a real terminal arrives.
    expect(
      shouldHandleNomiTerminalPostProcess(
        {
          type: 'content',
          data: { content: 'text' },
          msgId: terminalId,
          finalTextAuthoritative: true,
        },
        {
          rootTurnId: turnId,
          lastSettledTurnId: null,
          hasBuffer: () => false,
          isAssociated: () => false,
        }
      )
    ).toBe(false);
  });

  test('only an explicit true marker claims backend final-text authority', () => {
    expect(isNomiBackendFinalTextAuthoritative(true)).toBe(true);
    expect(isNomiBackendFinalTextAuthoritative(false)).toBe(false);
    expect(isNomiBackendFinalTextAuthoritative(undefined)).toBe(false);
  });

  test('an explicit false marker leaves a matching finish on the legacy path', () => {
    expect(
      shouldHandleNomiTerminalPostProcess(
        {
          type: 'finish',
          data: {},
          msgId: terminalId,
          turnId,
          finalTextAuthoritative: false,
          finalTextMsgId: targetMessageId,
        },
        {
          rootTurnId: turnId,
          lastSettledTurnId: null,
          hasBuffer: () => false,
          isAssociated: () => false,
        }
      )
    ).toBe(true);
  });

  test('an explicit false marker never makes an error a completed post-process job', () => {
    expect(
      shouldHandleNomiTerminalPostProcess(
        {
          type: 'error',
          data: {},
          msgId: terminalId,
          turnId,
          finalTextAuthoritative: false,
        },
        {
          rootTurnId: turnId,
          lastSettledTurnId: null,
          hasBuffer: () => false,
          isAssociated: () => false,
        }
      )
    ).toBe(false);
  });
});
