/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { TFunction } from 'i18next';
import React from 'react';

import { CAPABILITY_COLORS } from '@/renderer/components/capability/CapabilityIcon';
import { renderIdmmCapabilityIcon } from '@/renderer/components/capability/idmmCapabilityIcon';

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

  test('uses the shared smart-decision icon for IDMM markers', () => {
    const idmmItem = buildSessionCapabilityItems(t, { idmmState: 'armed' }).find((item) => item.key === 'idmm');
    const sharedIcon = renderIdmmCapabilityIcon({ size: 13 });

    expect(React.isValidElement(idmmItem?.icon)).toBe(true);
    expect((idmmItem?.icon as React.ReactElement).type).toBe(sharedIcon.type);
  });
});
