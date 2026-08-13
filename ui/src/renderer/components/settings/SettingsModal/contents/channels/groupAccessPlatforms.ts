/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ChannelPlatform } from './channelTarget';

/**
 * Platforms whose adapters can reliably distinguish a direct message from an
 * IM group/channel and enforce the shared mention gate. Telegram is excluded
 * until its adapter has structured @-mention parsing; Matrix is excluded until
 * its sync adapter consumes `m.direct` / membership state. WeChat, Nostr and
 * Twitch do not expose the group-chat product contract managed here.
 */
export const GROUP_ACCESS_PLATFORMS = [
  'lark',
  'dingtalk',
  'wecom',
  'qqbot',
  'discord',
  'slack',
  'mattermost',
] as const satisfies readonly ChannelPlatform[];

const GROUP_ACCESS_PLATFORM_SET: ReadonlySet<ChannelPlatform> = new Set(GROUP_ACCESS_PLATFORMS);

export function supportsGroupAccess(platform: ChannelPlatform): boolean {
  return GROUP_ACCESS_PLATFORM_SET.has(platform);
}
