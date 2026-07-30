/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 渠道实体寻址目标（多机器人模型）。
 *
 * Addresses one channel plugin entity for the per-owner
 * multi-bot flows:
 * - `channelPluginId` present → update that canonical UUIDv7 entity;
 * - `channelPluginId` absent → create mode: the first enable creates a new entity of
 *   the form's platform bound to its owner (backend rejects with 409 when the
 *   same bot is already bound to another owner).
 *
 * The bind owner is a desktop companion (`companionId`); the enable call
 * forwards it as `companion_id`. (Customer-service bindings are owned by the
 * customer-service domain — PUT /api/customer-service/agents/{id}/bindings —
 * and never ride the enable call.)
 *
 * Forms that receive no `channelTarget` create an unbound row by platform.
 */
import type { ChannelOwnerDomain } from '@/common/types/channel/channel';
import type { ChannelPluginId, CompanionId } from '@/common/types/ids';

export interface ChannelTarget {
  channelPluginId?: ChannelPluginId;
  companionId?: CompanionId;
  /**
   * 目标所有权域。`'customer_service'` 时创建请求带 `owner_domain` 且绝不携带
   * `companion_id`（两域互斥）；缺省 = companion 域（伙伴侧既有行为不变）。
   */
  ownerDomain?: ChannelOwnerDomain;
}

/** Builtin IM platforms a companion can connect (the channel config forms cover this set). */
export type ChannelPlatform = 'telegram' | 'lark' | 'dingtalk' | 'weixin' | 'wecom' | 'discord' | 'slack' | 'matrix' | 'mattermost' | 'twitch' | 'nostr' | 'qqbot';
