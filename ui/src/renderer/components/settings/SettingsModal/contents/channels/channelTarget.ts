/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 渠道行寻址目标（多机器人模型）。
 *
 * Addresses one channel row (`channel_plugins` row) for the per-owner
 * multi-bot flows:
 * - `channelId` present → update that canonical `chn_` row;
 * - `channelId` absent → create mode: the first enable creates a new row of
 *   the form's platform bound to its owner (backend rejects with 409 when the
 *   same bot is already bound to another owner).
 *
 * The bind owner is EITHER a desktop companion (`companionId`) OR a public
 * agent (`publicAgentId`) — exactly one is set; they are mutually exclusive
 * (a channel bot serves a single object). The enable call forwards whichever
 * one is present as `companion_id` / `public_agent_id`.
 *
 * Forms that receive no `channelTarget` create an unbound row by platform.
 */
import type { ChannelId, CompanionId, PublicAgentId } from '@/common/types/ids';

export interface ChannelTarget {
  channelId?: ChannelId;
  companionId?: CompanionId;
  publicAgentId?: PublicAgentId;
}

/** Builtin IM platforms a companion can connect (the channel config forms cover this set). */
export type ChannelPlatform = 'telegram' | 'lark' | 'dingtalk' | 'weixin' | 'wecom' | 'discord' | 'slack' | 'matrix' | 'mattermost' | 'twitch' | 'nostr' | 'qqbot';
