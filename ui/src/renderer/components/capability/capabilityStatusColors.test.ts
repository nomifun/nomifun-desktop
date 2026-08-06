/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ISshLinkPhase } from '@/common/adapter/ipcBridge';

import { CAPABILITY_COLORS } from './CapabilityIcon';
import { SSH_STATUS_COLOR } from './capabilityStatusColors';

/** The seven phases the backend's SshLinkPhase can serialize. */
const ALL_PHASES: ISshLinkPhase[] = [
  'idle',
  'connecting',
  'connected',
  'degraded',
  'reconnecting',
  'dropped',
  'closed',
];

describe('SSH link phase colours', () => {
  test('covers every phase with nothing extra', () => {
    expect(Object.keys(SSH_STATUS_COLOR).sort()).toEqual([...ALL_PHASES].sort());
  });

  test('a working link is green and a lost link is red', () => {
    expect(SSH_STATUS_COLOR.connected).toBe(CAPABILITY_COLORS.active);
    expect(SSH_STATUS_COLOR.dropped).toBe(CAPABILITY_COLORS.danger);
  });

  test('a recycling shell and a backing-off dial are both "armed", not green', () => {
    // degraded = transport is fine, the remote shell is being recycled;
    // reconnecting = the ladder is running. Neither may read as connected.
    expect(SSH_STATUS_COLOR.degraded).toBe(CAPABILITY_COLORS.armed);
    expect(SSH_STATUS_COLOR.reconnecting).toBe(CAPABILITY_COLORS.armed);
    expect(SSH_STATUS_COLOR.connecting).toBe(CAPABILITY_COLORS.idle);
    expect(SSH_STATUS_COLOR.idle).toBe(CAPABILITY_COLORS.off);
    expect(SSH_STATUS_COLOR.closed).toBe(CAPABILITY_COLORS.off);
  });

  test('every colour comes from the shared palette, never a literal', () => {
    const palette = new Set<string>(Object.values(CAPABILITY_COLORS));
    for (const phase of ALL_PHASES) {
      expect(palette.has(SSH_STATUS_COLOR[phase])).toBe(true);
    }
  });
});
