/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import type { IAutoWorkState } from '@/common/adapter/ipcBridge';
import { parseConversationId } from '@/common/types/ids';

import {
  applyAutoWorkStateToSessionCapabilities,
  capabilityKey,
  getSessionCapabilitySnapshot,
  resetSessionCapabilitiesForTest,
} from './useSessionCapabilities';

const autoWorkState = (overrides: Partial<IAutoWorkState> = {}): IAutoWorkState => ({
  kind: 'conversation',
  target_id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000007'),
  enabled: true,
  running: false,
  run_state: 'idle',
  completed_count: 0,
  ...overrides,
});

describe('SessionList capability snapshot', () => {
  const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000007');

  test('applies an enabled AutoWork state returned from the control save flow', () => {
    resetSessionCapabilitiesForTest();

    applyAutoWorkStateToSessionCapabilities(autoWorkState());

    const snapshot = getSessionCapabilitySnapshot();
    expect(snapshot.autowork.get(capabilityKey('conversation', conversationId))).toBe('idle');
  });

  test('removes AutoWork state when the control save flow disables it', () => {
    resetSessionCapabilitiesForTest();
    applyAutoWorkStateToSessionCapabilities(autoWorkState());

    applyAutoWorkStateToSessionCapabilities(autoWorkState({ enabled: false, run_state: 'off' }));

    const snapshot = getSessionCapabilitySnapshot();
    expect(snapshot.autowork.has(capabilityKey('conversation', conversationId))).toBe(false);
  });

  test('does not subscribe to the retired independent decision layer', () => {
    const hook = readFileSync(new URL('./useSessionCapabilities.ts', import.meta.url), 'utf8');
    const projection = readFileSync(new URL('../utils/sessionCapabilityItems.tsx', import.meta.url), 'utf8');

    expect(hook.includes('ipcBridge.idmm')).toBe(false);
    expect(projection.includes('idmmState')).toBe(false);
  });
});
