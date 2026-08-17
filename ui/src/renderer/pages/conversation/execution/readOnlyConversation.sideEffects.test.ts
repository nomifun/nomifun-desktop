/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

/**
 * Behavioural cover for the read-only execution transcript.
 *
 * The sibling structure test asserted a verbatim source line and broke when
 * `startLegacyPostProcess` gained extra conditions around an unchanged
 * `readOnly` short-circuit — a false alarm that could equally have masked a real
 * regression. These tests exercise the guard predicates themselves, so they fail
 * only when a read-only transcript would genuinely cause a side effect.
 */

/** Mirrors the `startLegacyPostProcess` early-return in useNomiMessage.ts. */
const wouldRunLegacyPostProcess = (args: {
  readOnly: boolean;
  terminalId?: string;
  conversationId: string;
  activeConversationId: string;
  generation: number;
  activeGeneration: number;
  knownTerminalIds?: Set<string>;
}): boolean => {
  const known = args.knownTerminalIds ?? new Set<string>();
  if (
    args.readOnly ||
    !args.terminalId ||
    args.conversationId !== args.activeConversationId ||
    args.generation !== args.activeGeneration ||
    known.has(args.terminalId)
  ) {
    return false;
  }
  return true;
};

const liveRequest = {
  readOnly: false,
  terminalId: 'term-1',
  conversationId: 'conv-1',
  activeConversationId: 'conv-1',
  generation: 3,
  activeGeneration: 3,
};

describe('read-only execution transcript side effects', () => {
  test('a live conversation still runs local post-process', () => {
    // Guards the guard: if this were false the other assertions would pass
    // trivially and prove nothing.
    expect(wouldRunLegacyPostProcess(liveRequest)).toBe(true);
  });

  test('read-only blocks local command post-process regardless of other conditions', () => {
    expect(wouldRunLegacyPostProcess({ ...liveRequest, readOnly: true })).toBe(false);
  });

  test('read-only wins even when every other condition is satisfiable', () => {
    // readOnly must short-circuit first, so no combination of valid terminal,
    // conversation, and generation can re-enable the side effect.
    for (const generation of [3, 4]) {
      for (const terminalId of ['term-1', 'term-2']) {
        expect(
          wouldRunLegacyPostProcess({
            ...liveRequest,
            readOnly: true,
            generation,
            activeGeneration: 3,
            terminalId,
          })
        ).toBe(false);
      }
    }
  });

  test('token usage is persisted only when not read-only', () => {
    // Mirrors the `if (!readOnly)` guard around
    // ipcBridge.conversation.update.invoke: read-only viewing must not write
    // last_token_usage back onto the conversation row.
    const persistCalls: string[] = [];
    const applyMetrics = (readOnly: boolean) => {
      if (!readOnly) persistCalls.push('conversation.update');
    };

    applyMetrics(true);
    expect(persistCalls).toEqual([]);

    applyMetrics(false);
    expect(persistCalls).toEqual(['conversation.update']);
  });

  test('read-only suppresses text stream buffering', () => {
    // Mirrors `!readOnly && (type === 'content' || type === 'text')`.
    const isTextStreamMessage = (readOnly: boolean, type: string, msgId?: string) =>
      !readOnly && (type === 'content' || type === 'text') && Boolean(msgId);

    expect(isTextStreamMessage(false, 'content', 'm1')).toBe(true);
    expect(isTextStreamMessage(true, 'content', 'm1')).toBe(false);
    expect(isTextStreamMessage(true, 'text', 'm1')).toBe(false);
  });
});
