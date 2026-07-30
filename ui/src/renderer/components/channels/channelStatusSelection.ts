/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ChannelOwnerDomain, IChannelPluginStatus } from '@/common/types/channel/channel';
import type { ChannelPluginId, CompanionId } from '@/common/types/ids';
import type { ChannelPlatform, ChannelTarget } from '@/renderer/components/settings/SettingsModal/contents/channels/channelTarget';

export interface EnabledChannelStatusQuery {
  platform: ChannelPlatform;
  enabledPluginId?: ChannelPluginId;
  companionId?: CompanionId;
  /** 客服域 create-mode 回退寻址：无 companion 归属时按域匹配。 */
  ownerDomain?: ChannelOwnerDomain;
}

export type ChannelConfigTarget = { platform: ChannelPlatform; channelPluginId?: ChannelPluginId } | null;

export interface ChannelOwnerQuery {
  companionId?: CompanionId;
}

const nonEmptyOwnerId = <T extends string>(value: T | null | undefined): T | undefined =>
  value == null || value.length === 0 ? undefined : value;

export function findEnabledChannelStatus(
  statuses: IChannelPluginStatus[],
  query: EnabledChannelStatusQuery
): IChannelPluginStatus | null {
  const enabledPluginId = query.enabledPluginId;
  if (enabledPluginId != null) {
    const byId = statuses.find((status) => status.plugin_id === enabledPluginId);
    if (byId) return byId;
  }

  const companionId = nonEmptyOwnerId(query.companionId);
  return (
    statuses.find((status) => {
      if (status.type !== query.platform) return false;
      if (companionId) return nonEmptyOwnerId(status.companionId) === companionId;
      // 客服域 create-mode：无 companion 归属，按所有权域匹配（精确解析仍以
      // enabledPluginId 优先，这里只是兜底）。
      if (query.ownerDomain === 'customer_service') return statusInOwnerDomain(status, 'customer_service');
      return false;
    }) ?? null
  );
}

/**
 * When the config modal is in create mode (no channelPluginId), move it onto the
 * entity the caller just resolved. The caller — findEnabledChannelStatus (by the
 * backend-returned business ID) or the owner-scoped adopt effect — already
 * guarantees `status` is the intended entity, so we retarget by business ID rather than
 * re-checking owner equality, which was fragile against id normalization /
 * binding-commit-lag skew and left the toggle stuck OFF after a real success.
 */
export function retargetConfigAfterStatus(
  current: ChannelConfigTarget,
  status: IChannelPluginStatus | null
): ChannelConfigTarget {
  if (!current || current.channelPluginId || !status || status.type !== current.platform) return current;
  return { platform: current.platform, channelPluginId: status.plugin_id };
}

/** Trimmed owner check: does this row currently belong to the given owner? */
export function statusOwnedBy(status: IChannelPluginStatus, owner: ChannelOwnerQuery): boolean {
  const companionId = nonEmptyOwnerId(owner.companionId);
  if (companionId) return nonEmptyOwnerId(status.companionId) === companionId;
  return false;
}

/** A row with no companion owner (a free, bindable bot). */
export function statusIsUnbound(status: IChannelPluginStatus): boolean {
  return !nonEmptyOwnerId(status.companionId);
}

/**
 * 所有权分域判定。过渡期后端可能尚未透出 `owner_domain` —— 缺省按 companion
 * 处理（与 DB `DEFAULT 'companion'` 一致），既有伙伴侧行为不变。
 */
export function statusInOwnerDomain(
  status: IChannelPluginStatus,
  domain: ChannelOwnerDomain
): boolean {
  return (status.owner_domain ?? 'companion') === domain;
}

/** `channel.enablePlugin` 请求体（见 ipcBridge enablePlugin 的寻址契约注释）。 */
export interface EnablePluginRequest {
  plugin_id?: ChannelPluginId;
  plugin_type: ChannelPlatform;
  companion_id?: CompanionId;
  owner_domain?: ChannelOwnerDomain;
  config: Record<string, unknown>;
}

/**
 * enable/create 请求体的唯一构造点，让所有平台表单转发同一份所有权契约：
 * - companion 域（缺省）：照旧转发 `companion_id` 绑宠；
 * - customer_service 域：创建时打上 `owner_domain`，且绝不携带 companion 绑定
 *   （两域互斥，后端触发器兜底）；已有行（带 plugin_id）按 id 寻址即可，
 *   不再重复发送 owner_domain（域不可变）。
 */
export function buildEnablePluginRequest(
  platform: ChannelPlatform,
  channelTarget: ChannelTarget | undefined,
  config: Record<string, unknown>
): EnablePluginRequest {
  if (!channelTarget) return { plugin_type: platform, config };
  const csDomain = channelTarget.ownerDomain === 'customer_service';
  return {
    ...(channelTarget.channelPluginId ? { plugin_id: channelTarget.channelPluginId } : {}),
    plugin_type: platform,
    ...(!csDomain && channelTarget.companionId ? { companion_id: channelTarget.companionId } : {}),
    ...(csDomain && !channelTarget.channelPluginId ? { owner_domain: 'customer_service' as const } : {}),
    config,
  };
}
