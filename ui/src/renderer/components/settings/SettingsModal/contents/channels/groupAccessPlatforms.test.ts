/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  buildSetGroupAccessRequest,
  normalizeGroupAccessMode,
} from '@/common/types/channel/channel';
import type { ChannelPluginId } from '@/common/types/ids';
import type { ChannelPlatform } from './channelTarget';
import { GROUP_ACCESS_PLATFORMS, supportsGroupAccess } from './groupAccessPlatforms';

const PLUGIN_ID = '0190f5fe-7c00-7a00-8000-000000000001' as ChannelPluginId;

describe('group-chat access UI contract', () => {
  test('exposes the policy only for adapters with reliable group and mention semantics', () => {
    expect(GROUP_ACCESS_PLATFORMS).toEqual([
      'lark',
      'dingtalk',
      'wecom',
      'qqbot',
      'discord',
      'slack',
      'mattermost',
    ]);

    const excluded: ChannelPlatform[] = ['telegram', 'matrix', 'weixin', 'nostr', 'twitch'];
    expect(GROUP_ACCESS_PLATFORMS.every(supportsGroupAccess)).toBe(true);
    expect(excluded.some(supportsGroupAccess)).toBe(false);
  });

  test('keeps all three wire values and fails closed for missing or future values', () => {
    expect(normalizeGroupAccessMode('all_members')).toBe('all_members');
    expect(normalizeGroupAccessMode('allowlist')).toBe('allowlist');
    expect(normalizeGroupAccessMode('disabled')).toBe('disabled');
    expect(normalizeGroupAccessMode(undefined)).toBe('disabled');
    expect(normalizeGroupAccessMode('future_mode')).toBe('disabled');
  });

  test('builds the exact independent settings endpoint payload', () => {
    expect(buildSetGroupAccessRequest(PLUGIN_ID, 'all_members')).toEqual({
      plugin_id: PLUGIN_ID,
      group_access_mode: 'all_members',
    });
    expect(buildSetGroupAccessRequest(PLUGIN_ID, 'invalid')).toEqual({
      plugin_id: PLUGIN_ID,
      group_access_mode: 'disabled',
    });
  });
});
