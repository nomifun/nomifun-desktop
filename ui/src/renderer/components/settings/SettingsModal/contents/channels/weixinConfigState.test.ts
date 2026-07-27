/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import type {
  IChannelPairingRequest,
  IChannelPluginStatus,
} from '@/common/types/channel/channel';
import { parseChannelPluginId, parseChannelUserId } from '@/common/types/ids';
import {
  applyWeixinAuthorizedUserMutations,
  applyWeixinPairingMutations,
  buildWeixinEnableConfig,
  findWeixinPluginStatusById,
  isWeixinRuntimeConnected,
} from './weixinConfigState';

const CHANNEL_A = parseChannelPluginId('0190f5fe-7c00-7a00-8000-000000000011');
const CHANNEL_B = parseChannelPluginId('0190f5fe-7c00-7a00-8000-000000000012');

const status = (patch: Partial<IChannelPluginStatus> = {}): IChannelPluginStatus => ({
  plugin_id: CHANNEL_A,
  type: 'weixin',
  name: 'WeChat',
  enabled: true,
  connected: true,
  status: 'running',
  activeUsers: 0,
  hasToken: true,
  ...patch,
});

const pairing = (
  code: string,
  platformUserId = `user-${code}`
): IChannelPairingRequest => ({
  code,
  platformUserId,
  platformType: 'weixin',
  requestedAt: 1,
  expiresAt: 2,
  channel_plugin_id: CHANNEL_A,
});

const user = (
  id: string,
  authorizedAt: number
): import('@/common/types/channel/channel').IChannelUser => ({
  channel_user_id: parseChannelUserId(id),
  platformUserId: `platform-${id}`,
  platformType: 'weixin',
  authorizedAt,
  channel_plugin_id: CHANNEL_A,
});

describe('WeChat runtime connection semantics', () => {
  test('requires enabled, connected, and the exact running state', () => {
    expect(isWeixinRuntimeConnected(status())).toBe(true);

    expect(isWeixinRuntimeConnected(status({ enabled: false }))).toBe(false);
    expect(isWeixinRuntimeConnected(status({ connected: false }))).toBe(false);
    expect(isWeixinRuntimeConnected(status({ status: 'starting' }))).toBe(false);
    expect(isWeixinRuntimeConnected(status({ status: 'error' }))).toBe(false);
    expect(isWeixinRuntimeConnected(status({ status: undefined }))).toBe(false);
    expect(isWeixinRuntimeConnected(null)).toBe(false);
  });

  test('never treats stored credentials as a live connection', () => {
    expect(
      isWeixinRuntimeConnected(
        status({
          enabled: true,
          connected: false,
          status: 'error',
          hasToken: true,
        })
      )
    ).toBe(false);
  });
});

describe('WeChat login configuration', () => {
  test('keeps the negotiated baseUrl as a flattened credentials field', () => {
    expect(
      buildWeixinEnableConfig(
        'account-1',
        'token-1',
        '  https://api.weixin.qq.com/custom/  '
      )
    ).toEqual({
      credentials: {
        account_id: 'account-1',
        bot_token: 'token-1',
        baseUrl: 'https://api.weixin.qq.com/custom/',
      },
    });
  });

  test('omits an absent baseUrl instead of serializing a blank override', () => {
    expect(buildWeixinEnableConfig('account-1', 'token-1', '   ')).toEqual({
      credentials: {
        account_id: 'account-1',
        bot_token: 'token-1',
      },
    });
  });
});

describe('WeChat plugin and pairing identity', () => {
  test('selects the status by the exact enable-response plugin id', () => {
    const statuses = [
      status({ plugin_id: CHANNEL_A }),
      status({ plugin_id: CHANNEL_B }),
    ];

    expect(findWeixinPluginStatusById(statuses, CHANNEL_B)?.plugin_id).toBe(CHANNEL_B);
  });

  test('replays a newer WebSocket pairing over a slow HTTP snapshot', () => {
    const snapshot = [pairing('old')];
    const livePairing = pairing('live');

    expect(
      applyWeixinPairingMutations(snapshot, [
        { sequence: 2, type: 'upsert', request: livePairing },
      ]).map((request) => request.code)
    ).toEqual(['live', 'old']);
  });

  test('applies later authorization removals after pairing upserts', () => {
    const livePairing = pairing('live', 'user-live');

    expect(
      applyWeixinPairingMutations([], [
        { sequence: 3, type: 'remove-user', platformUserId: 'user-live' },
        { sequence: 2, type: 'upsert', request: livePairing },
      ])
    ).toEqual([]);
  });

  test('authorization removes only the approved bot pairing in a global view', () => {
    const firstBot = pairing('first', 'same-user');
    const secondBot = {
      ...pairing('second', 'same-user'),
      channel_plugin_id: CHANNEL_B,
    };

    expect(
      applyWeixinPairingMutations([firstBot, secondBot], [
        {
          sequence: 3,
          type: 'remove-user',
          platformUserId: 'same-user',
          channelPluginId: CHANNEL_A,
        },
      ]).map((request) => request.code)
    ).toEqual(['second']);
  });

  test('replays a newer authorization over a stale HTTP user snapshot', () => {
    const existing = user('0190f5fe-7c00-7a00-8000-000000000021', 1);
    const newlyAuthorized = user('0190f5fe-7c00-7a00-8000-000000000022', 2);

    expect(
      applyWeixinAuthorizedUserMutations([existing], [
        { sequence: 4, user: newlyAuthorized },
      ]).map((authorized) => authorized.channel_user_id)
    ).toEqual([newlyAuthorized.channel_user_id, existing.channel_user_id]);
  });

  test('authorization replay replaces duplicate users by stable id', () => {
    const stale = user('0190f5fe-7c00-7a00-8000-000000000023', 1);
    const current = user('0190f5fe-7c00-7a00-8000-000000000023', 2);

    expect(
      applyWeixinAuthorizedUserMutations([stale], [
        { sequence: 5, user: current },
      ])
    ).toEqual([current]);
  });
});

describe('WeChat pairing reconnect recovery', () => {
  test('reloads durable pairings and users after WebSocket reconnect', () => {
    const formSource = readFileSync(
      new URL('./WeixinConfigForm.tsx', import.meta.url),
      'utf8'
    );

    expect(formSource.includes('channel.reconnected.on')).toBe(true);
    expect(formSource.includes('void loadPendingPairings()')).toBe(true);
    expect(formSource.includes('void loadAuthorizedUsers()')).toBe(true);
  });
});
