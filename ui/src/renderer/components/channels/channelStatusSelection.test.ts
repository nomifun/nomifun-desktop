/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import { parseChannelPluginId, parseCompanionId } from '@/common/types/ids';
import {
  buildEnablePluginRequest,
  findEnabledChannelStatus,
  retargetConfigAfterStatus,
  statusInOwnerDomain,
  statusOwnedBy,
  statusIsUnbound,
} from './channelStatusSelection';

const CHANNEL_DEFAULT = parseChannelPluginId('0190f5fe-7c00-7a00-8000-000000000011');
const CHANNEL_OTHER = parseChannelPluginId('0190f5fe-7c00-7a00-8000-000000000012');
const CHANNEL_TARGET = parseChannelPluginId('0190f5fe-7c00-7a00-8000-000000000013');
const CHANNEL_UNBOUND = parseChannelPluginId('0190f5fe-7c00-7a00-8000-000000000014');
const CHANNEL_EXISTING = parseChannelPluginId('0190f5fe-7c00-7a00-8000-000000000015');
const CHANNEL_X = parseChannelPluginId('0190f5fe-7c00-7a00-8000-000000000016');
const COMPANION_A = parseCompanionId('0190f5fe-7c00-7a00-8000-000000000001');
const COMPANION_B = parseCompanionId('0190f5fe-7c00-7a00-8000-000000000002');
const COMPANION_OTHER = parseCompanionId('0190f5fe-7c00-7a00-8000-000000000003');
const COMPANION_TARGET = parseCompanionId('0190f5fe-7c00-7a00-8000-000000000004');

const row = (patch: Partial<IChannelPluginStatus>): IChannelPluginStatus => ({
  plugin_id: CHANNEL_DEFAULT,
  type: 'qqbot',
  name: 'QQ Bot',
  enabled: true,
  connected: true,
  activeUsers: 0,
  hasToken: true,
  owner_domain: 'companion',
  ...patch,
});

describe('findEnabledChannelStatus', () => {
  test('uses the backend returned channel id before owner fallback', () => {
    const statuses = [
      row({ plugin_id: CHANNEL_DEFAULT, enabled: false, connected: false, hasToken: false }),
      row({ plugin_id: CHANNEL_OTHER, companionId: COMPANION_OTHER }),
      row({ plugin_id: CHANNEL_TARGET, companionId: COMPANION_TARGET }),
    ];

    expect(
      findEnabledChannelStatus(statuses, {
        platform: 'qqbot',
        enabledPluginId: CHANNEL_TARGET,
        companionId: COMPANION_OTHER,
      })?.plugin_id
    ).toBe(CHANNEL_TARGET);
  });

  test('falls back to platform plus companion binding for create-mode enables', () => {
    const statuses = [
      row({ plugin_id: CHANNEL_UNBOUND, companionId: undefined }),
      row({ plugin_id: CHANNEL_TARGET, companionId: COMPANION_TARGET }),
    ];

    expect(
      findEnabledChannelStatus(statuses, {
        platform: 'qqbot',
        companionId: COMPANION_TARGET,
      })?.plugin_id
    ).toBe(CHANNEL_TARGET);
  });

  test('falls back to the customer-service domain when the query targets it without a companion', () => {
    const statuses = [
      row({ plugin_id: CHANNEL_UNBOUND, owner_domain: 'companion' }),
      row({ plugin_id: CHANNEL_TARGET, owner_domain: 'customer_service' }),
    ];

    expect(
      findEnabledChannelStatus(statuses, {
        platform: 'qqbot',
        ownerDomain: 'customer_service',
      })?.plugin_id
    ).toBe(CHANNEL_TARGET);
    // Without the domain hint an ownerless query still resolves nothing.
    expect(findEnabledChannelStatus(statuses, { platform: 'qqbot' })).toBeNull();
  });

});

describe('retargetConfigAfterStatus', () => {
  test('moves a create-mode modal onto the resolved row by id (owner-agnostic)', () => {
    expect(
      retargetConfigAfterStatus(
        { platform: 'qqbot' },
        row({ plugin_id: CHANNEL_TARGET, companionId: COMPANION_TARGET }),
      ),
    ).toEqual({ platform: 'qqbot', channelPluginId: CHANNEL_TARGET });
  });

  test('leaves an existing-row modal, a platform mismatch, or null status untouched', () => {
    expect(
      retargetConfigAfterStatus(
        { platform: 'qqbot', channelPluginId: CHANNEL_EXISTING },
        row({ plugin_id: CHANNEL_TARGET, companionId: COMPANION_TARGET })
      )
    ).toEqual({ platform: 'qqbot', channelPluginId: CHANNEL_EXISTING });
    expect(
      retargetConfigAfterStatus(
        { platform: 'qqbot' },
        row({ plugin_id: CHANNEL_X, type: 'telegram' }),
      ),
    ).toEqual({
      platform: 'qqbot',
    });
    expect(retargetConfigAfterStatus({ platform: 'qqbot' }, null)).toEqual({ platform: 'qqbot' });
  });
});

describe('statusOwnedBy / statusIsUnbound', () => {
  test('statusOwnedBy matches the right canonical owner side', () => {
    expect(statusOwnedBy(row({ companionId: COMPANION_A }), { companionId: COMPANION_A })).toBe(true);
    expect(statusOwnedBy(row({ companionId: COMPANION_A }), { companionId: COMPANION_B })).toBe(false);
  });

  test('statusIsUnbound is true only when no owner is set', () => {
    expect(statusIsUnbound(row({ companionId: undefined }))).toBe(true);
    expect(statusIsUnbound(row({ companionId: COMPANION_A }))).toBe(false);
  });
});

describe('statusInOwnerDomain', () => {
  test('splits rows by ownership domain, defaulting missing wire values to companion', () => {
    expect(statusInOwnerDomain(row({ owner_domain: 'companion' }), 'companion')).toBe(true);
    expect(statusInOwnerDomain(row({ owner_domain: 'customer_service' }), 'customer_service')).toBe(true);
    expect(statusInOwnerDomain(row({ owner_domain: 'customer_service' }), 'companion')).toBe(false);
    // Transitional payloads without the column behave as companion-domain rows.
    expect(
      statusInOwnerDomain(row({ owner_domain: undefined as unknown as 'companion' }), 'companion')
    ).toBe(true);
  });
});

describe('buildEnablePluginRequest', () => {
  test('without a channel target it creates a bare row by platform', () => {
    expect(buildEnablePluginRequest('qqbot', undefined, { a: 1 })).toEqual({
      plugin_type: 'qqbot',
      config: { a: 1 },
    });
  });

  test('companion-domain targets forward companion_id and never owner_domain', () => {
    expect(
      buildEnablePluginRequest('telegram', { companionId: COMPANION_A }, {})
    ).toEqual({ plugin_type: 'telegram', companion_id: COMPANION_A, config: {} });
    expect(
      buildEnablePluginRequest('telegram', { channelPluginId: CHANNEL_EXISTING, companionId: COMPANION_A }, {})
    ).toEqual({
      plugin_id: CHANNEL_EXISTING,
      plugin_type: 'telegram',
      companion_id: COMPANION_A,
      config: {},
    });
  });

  test('customer-service create-mode stamps owner_domain and never carries a companion binding', () => {
    expect(
      buildEnablePluginRequest(
        'telegram',
        { ownerDomain: 'customer_service', companionId: COMPANION_A },
        { credentials: { token: 't' } }
      )
    ).toEqual({
      plugin_type: 'telegram',
      owner_domain: 'customer_service',
      config: { credentials: { token: 't' } },
    });
  });

  test('customer-service edit-mode addresses the row by id without re-sending owner_domain', () => {
    expect(
      buildEnablePluginRequest(
        'telegram',
        { channelPluginId: CHANNEL_EXISTING, ownerDomain: 'customer_service' },
        {}
      )
    ).toEqual({ plugin_id: CHANNEL_EXISTING, plugin_type: 'telegram', config: {} });
  });
});
