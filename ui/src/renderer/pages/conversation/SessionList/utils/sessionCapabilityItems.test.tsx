/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { TFunction } from 'i18next';
import { CAPABILITY_COLORS } from '@/renderer/components/capability/CapabilityIcon';
import { AUTOWORK_STATUS_COLOR } from '@/renderer/components/capability/capabilityStatusColors';

import { buildSessionCapabilityItems, type SessionCronStatus } from './sessionCapabilityItems';

const t = ((key: string) => key) as TFunction;

describe('buildSessionCapabilityItems', () => {
  test.each(['active', 'paused', 'unread'] as SessionCronStatus[])(
    'uses the brand-lit colour for bound cron sessions in %s state',
    (cronStatus) => {
      const cronItem = buildSessionCapabilityItems(t, { cronStatus }).find((item) => item.key === 'cron');

      expect(cronItem?.color).toBe(CAPABILITY_COLORS.brand);
    }
  );

  test('keeps cron error state in the danger colour', () => {
    const cronItem = buildSessionCapabilityItems(t, { cronStatus: 'error' }).find((item) => item.key === 'cron');

    expect(cronItem?.color).toBe(CAPABILITY_COLORS.danger);
  });

  // The sidebar icon and conversation-header control read the same shared
  // state→colour map so the two surfaces never drift.
  test('colours AutoWork icon from the shared map (active→active, idle→idle)', () => {
    const active = buildSessionCapabilityItems(t, { autoworkState: 'active' }).find((i) => i.key === 'autowork');
    const idle = buildSessionCapabilityItems(t, { autoworkState: 'idle' }).find((i) => i.key === 'autowork');
    expect(active?.color).toBe(AUTOWORK_STATUS_COLOR.active);
    expect(idle?.color).toBe(AUTOWORK_STATUS_COLOR.idle);
  });

});
