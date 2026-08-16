/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('execution transcript capability boundary', () => {
  test('marks every projected platform chat as read-only', () => {
    const source = readSource(new URL('./ReadOnlyConversationView.tsx', import.meta.url));
    // One readOnly / hideSendBox prop per projected platform arm. Only the nomi
    // arm remains, so each prop must appear exactly once — a second occurrence
    // would mean an unaudited second surface was reintroduced.
    expect(source.match(/readOnly/g)?.length ?? 0).toBe(1);
    expect(source.match(/hideSendBox/g)?.length ?? 0).toBe(1);
  });

  test('disables Nomi persistence and local command side effects', () => {
    const chatSource = readSource(new URL('../platforms/nomi/NomiChat.tsx', import.meta.url));
    const messageSource = readSource(new URL('../platforms/nomi/useNomiMessage.ts', import.meta.url));
    const recoverySource = readSource(new URL('../Messages/usePendingConfirmationsRecovery.ts', import.meta.url));

    expect(chatSource.includes('usePendingConfirmationsRecovery(conversation_id, { enabled: !readOnly })')).toBe(true);
    expect(chatSource.includes('readOnly,')).toBe(true);
    expect(recoverySource.includes('if (!enabled || !conversation_id) return;')).toBe(true);
    // The post-process side effect (spawning terminal follow-up work) must stay
    // gated on readOnly. `37fb2407` renamed the callback that carried this guard
    // from `processCompletedAssistantMessage` to `startLegacyPostProcess`, so
    // pin the guard's leading conjunct rather than the old callback's argument.
    expect(messageSource.includes('readOnly ||')).toBe(true);
    expect(messageSource.includes('if (!readOnly) {')).toBe(true);
    expect(messageSource.includes('ipcBridge.conversation.update.invoke')).toBe(true);
  });

  test('removes every confirmation action from transcript rendering', () => {
    const permissionSource = readSource(new URL('../Messages/components/MessagePermission.tsx', import.meta.url));
    const toolGroupSource = readSource(new URL('../Messages/components/MessageToolGroup.tsx', import.meta.url));

    expect(permissionSource.includes('useConversationContextSafe()?.readOnly === true')).toBe(true);
    expect(permissionSource.includes('if (readOnly || hasResponded || !selected) return;')).toBe(true);
    expect(permissionSource.includes('!readOnly && !hasResponded')).toBe(true);

    expect(toolGroupSource.includes("!readOnly && content.status === 'Confirming'")).toBe(true);
    expect(toolGroupSource.includes('if (readOnly) return;')).toBe(true);
  });
});
