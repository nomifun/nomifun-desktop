/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const hooksSource = readFileSync(new URL('./hooks.ts', import.meta.url), 'utf8');
const emitterSource = readFileSync(
  new URL('../../../utils/emitter.ts', import.meta.url),
  'utf8'
);
const reconcileSource = readFileSync(
  new URL('../platforms/reconcileConversationTurnAfterStreamTerminal.ts', import.meta.url),
  'utf8'
);

describe('turn settle transcript refetch', () => {
  test('emitter declares the local conversation.turn.settled event', () => {
    expect(emitterSource.includes("'conversation.turn.settled': [ConversationId]")).toBe(true);
  });

  test('idle reconciliation announces the local settle signal', () => {
    // The HTTP GET-poll fallback is the only authority left when every WS
    // frame was lost; it must also trigger a transcript reload, not just
    // lower the spinner.
    expect(
      reconcileSource.includes("emitter.emit('conversation.turn.settled', conversationId)")
    ).toBe(true);
  });

  test('useMessageLstCache reloads messages when the current conversation settles', () => {
    expect(hooksSource.includes("addEventListener('conversation.turn.settled'")).toBe(true);
    expect(
      hooksSource.includes('[useMessageLstCache] Failed to refresh messages after turn settle:')
    ).toBe(true);
  });
});
