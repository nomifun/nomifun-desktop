/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import { findEnabledChannelStatus, retargetConfigAfterStatus, statusOwnedBy, statusIsUnbound } from './channelStatusSelection';

const row = (patch: Partial<IChannelPluginStatus>): IChannelPluginStatus => ({
  id: 'achn_default',
  type: 'qqbot',
  name: 'QQ Bot',
  enabled: true,
  connected: true,
  activeUsers: 0,
  hasToken: true,
  ...patch,
});

describe('findEnabledChannelStatus', () => {
  test('uses the backend returned channel id before owner fallback', () => {
    const statuses = [
      row({ id: 'qqbot', enabled: false, connected: false, hasToken: false }),
      row({ id: 'achn_other', companionId: 'companion_other' }),
      row({ id: 'achn_target', companionId: 'companion_target' }),
    ];

    expect(
      findEnabledChannelStatus(statuses, {
        platform: 'qqbot',
        enabledPluginId: 'achn_target',
        companionId: 'companion_other',
      })?.id
    ).toBe('achn_target');
  });

  test('falls back to platform plus companion binding for create-mode enables', () => {
    const statuses = [
      row({ id: 'achn_unbound', companionId: undefined, publicAgentId: null }),
      row({ id: 'achn_target', companionId: 'companion_target' }),
    ];

    expect(
      findEnabledChannelStatus(statuses, {
        platform: 'qqbot',
        companionId: 'companion_target',
      })?.id
    ).toBe('achn_target');
  });

  test('falls back to platform plus public agent binding', () => {
    const statuses = [
      row({ id: 'achn_other', publicAgentId: 'pub_other' }),
      row({ id: 'achn_target', publicAgentId: 'pub_target' }),
    ];

    expect(
      findEnabledChannelStatus(statuses, {
        platform: 'qqbot',
        publicAgentId: 'pub_target',
      })?.id
    ).toBe('achn_target');
  });
});

describe('retargetConfigAfterStatus', () => {
  test('moves a create-mode modal onto the resolved row by id (owner-agnostic)', () => {
    expect(
      retargetConfigAfterStatus({ platform: 'qqbot' }, row({ id: 'achn_target', companionId: 'companion_target' }))
    ).toEqual({ platform: 'qqbot', channelId: 'achn_target' });
    // The caller already resolved the correct row, so an owner-id skew must NOT
    // block the retarget (this was the stuck-toggle bug).
    expect(
      retargetConfigAfterStatus({ platform: 'qqbot' }, row({ id: 'achn_target', companionId: ' companion_target ' }))
    ).toEqual({ platform: 'qqbot', channelId: 'achn_target' });
  });

  test('leaves an existing-row modal, a platform mismatch, or null status untouched', () => {
    expect(
      retargetConfigAfterStatus(
        { platform: 'qqbot', channelId: 'achn_existing' },
        row({ id: 'achn_target', companionId: 'companion_target' })
      )
    ).toEqual({ platform: 'qqbot', channelId: 'achn_existing' });
    expect(retargetConfigAfterStatus({ platform: 'qqbot' }, row({ id: 'achn_x', type: 'telegram' }))).toEqual({
      platform: 'qqbot',
    });
    expect(retargetConfigAfterStatus({ platform: 'qqbot' }, null)).toEqual({ platform: 'qqbot' });
  });
});

describe('statusOwnedBy / statusIsUnbound', () => {
  test('statusOwnedBy trims and matches the right owner side', () => {
    expect(statusOwnedBy(row({ companionId: 'companion_a' }), { companionId: ' companion_a ' })).toBe(true);
    expect(statusOwnedBy(row({ companionId: 'companion_a' }), { companionId: 'companion_b' })).toBe(false);
    expect(statusOwnedBy(row({ publicAgentId: 'pub_a' }), { publicAgentId: 'pub_a' })).toBe(true);
    // publicAgent owner takes precedence in the query
    expect(statusOwnedBy(row({ companionId: 'companion_a' }), { publicAgentId: 'pub_a' })).toBe(false);
  });

  test('statusIsUnbound is true only when neither owner is set', () => {
    expect(statusIsUnbound(row({ companionId: undefined, publicAgentId: undefined }))).toBe(true);
    expect(statusIsUnbound(row({ companionId: '   ', publicAgentId: undefined }))).toBe(true);
    expect(statusIsUnbound(row({ companionId: 'companion_a' }))).toBe(false);
    expect(statusIsUnbound(row({ publicAgentId: 'pub_a' }))).toBe(false);
  });
});
