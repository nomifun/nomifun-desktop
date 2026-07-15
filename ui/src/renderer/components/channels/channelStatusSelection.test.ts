/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import { parseChannelId, parseCompanionId, parsePublicAgentId } from '@/common/types/ids';
import { findEnabledChannelStatus, retargetConfigAfterStatus, statusOwnedBy, statusIsUnbound } from './channelStatusSelection';

const CHANNEL_DEFAULT = parseChannelId('chn_0190f5fe-7c00-7a00-8000-000000000001');
const CHANNEL_OTHER = parseChannelId('chn_0190f5fe-7c00-7a00-8000-000000000002');
const CHANNEL_TARGET = parseChannelId('chn_0190f5fe-7c00-7a00-8000-000000000003');
const CHANNEL_UNBOUND = parseChannelId('chn_0190f5fe-7c00-7a00-8000-000000000004');
const CHANNEL_EXISTING = parseChannelId('chn_0190f5fe-7c00-7a00-8000-000000000005');
const CHANNEL_X = parseChannelId('chn_0190f5fe-7c00-7a00-8000-000000000006');
const COMPANION_A = parseCompanionId('companion_0190f5fe-7c00-7a00-8000-000000000001');
const COMPANION_B = parseCompanionId('companion_0190f5fe-7c00-7a00-8000-000000000002');
const COMPANION_OTHER = parseCompanionId('companion_0190f5fe-7c00-7a00-8000-000000000003');
const COMPANION_TARGET = parseCompanionId('companion_0190f5fe-7c00-7a00-8000-000000000004');
const PUBLIC_A = parsePublicAgentId('pubagent_0190f5fe-7c00-7a00-8000-000000000001');
const PUBLIC_OTHER = parsePublicAgentId('pubagent_0190f5fe-7c00-7a00-8000-000000000002');
const PUBLIC_TARGET = parsePublicAgentId('pubagent_0190f5fe-7c00-7a00-8000-000000000003');

const row = (patch: Partial<IChannelPluginStatus>): IChannelPluginStatus => ({
  id: CHANNEL_DEFAULT,
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
      row({ id: CHANNEL_DEFAULT, enabled: false, connected: false, hasToken: false }),
      row({ id: CHANNEL_OTHER, companionId: COMPANION_OTHER }),
      row({ id: CHANNEL_TARGET, companionId: COMPANION_TARGET }),
    ];

    expect(
      findEnabledChannelStatus(statuses, {
        platform: 'qqbot',
        enabledPluginId: CHANNEL_TARGET,
        companionId: COMPANION_OTHER,
      })?.id
    ).toBe(CHANNEL_TARGET);
  });

  test('falls back to platform plus companion binding for create-mode enables', () => {
    const statuses = [
      row({ id: CHANNEL_UNBOUND, companionId: undefined, publicAgentId: null }),
      row({ id: CHANNEL_TARGET, companionId: COMPANION_TARGET }),
    ];

    expect(
      findEnabledChannelStatus(statuses, {
        platform: 'qqbot',
        companionId: COMPANION_TARGET,
      })?.id
    ).toBe(CHANNEL_TARGET);
  });

  test('falls back to platform plus public agent binding', () => {
    const statuses = [
      row({ id: CHANNEL_OTHER, publicAgentId: PUBLIC_OTHER }),
      row({ id: CHANNEL_TARGET, publicAgentId: PUBLIC_TARGET }),
    ];

    expect(
      findEnabledChannelStatus(statuses, {
        platform: 'qqbot',
        publicAgentId: PUBLIC_TARGET,
      })?.id
    ).toBe(CHANNEL_TARGET);
  });
});

describe('retargetConfigAfterStatus', () => {
  test('moves a create-mode modal onto the resolved row by id (owner-agnostic)', () => {
    expect(
      retargetConfigAfterStatus({ platform: 'qqbot' }, row({ id: CHANNEL_TARGET, companionId: COMPANION_TARGET }))
    ).toEqual({ platform: 'qqbot', channelId: CHANNEL_TARGET });
  });

  test('leaves an existing-row modal, a platform mismatch, or null status untouched', () => {
    expect(
      retargetConfigAfterStatus(
        { platform: 'qqbot', channelId: CHANNEL_EXISTING },
        row({ id: CHANNEL_TARGET, companionId: COMPANION_TARGET })
      )
    ).toEqual({ platform: 'qqbot', channelId: CHANNEL_EXISTING });
    expect(retargetConfigAfterStatus({ platform: 'qqbot' }, row({ id: CHANNEL_X, type: 'telegram' }))).toEqual({
      platform: 'qqbot',
    });
    expect(retargetConfigAfterStatus({ platform: 'qqbot' }, null)).toEqual({ platform: 'qqbot' });
  });
});

describe('statusOwnedBy / statusIsUnbound', () => {
  test('statusOwnedBy matches the right canonical owner side', () => {
    expect(statusOwnedBy(row({ companionId: COMPANION_A }), { companionId: COMPANION_A })).toBe(true);
    expect(statusOwnedBy(row({ companionId: COMPANION_A }), { companionId: COMPANION_B })).toBe(false);
    expect(statusOwnedBy(row({ publicAgentId: PUBLIC_A }), { publicAgentId: PUBLIC_A })).toBe(true);
    // publicAgent owner takes precedence in the query
    expect(statusOwnedBy(row({ companionId: COMPANION_A }), { publicAgentId: PUBLIC_A })).toBe(false);
  });

  test('statusIsUnbound is true only when neither owner is set', () => {
    expect(statusIsUnbound(row({ companionId: undefined, publicAgentId: undefined }))).toBe(true);
    expect(statusIsUnbound(row({ companionId: COMPANION_A }))).toBe(false);
    expect(statusIsUnbound(row({ publicAgentId: PUBLIC_A }))).toBe(false);
  });
});
