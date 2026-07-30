/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 客服渠道自闭环的纯函数面：客服域机器人挑选池、绑定态推导、
 * 创建弹窗的"新出现 bot"快照差分（创建即自动绑定的探测点）。
 *
 * 渠道所有权分域：客服的 bot 与桌面伙伴的 bot 彻底互斥、不共享挑选池
 * （`channel_plugins.owner_domain`，后端触发器兜底）。
 */

import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import type { ChannelPluginId, CsAgentId } from '@/common/types/ids';
import { statusInOwnerDomain } from '@/renderer/components/channels/channelStatusSelection';

/** 客服域机器人（客服自闭环挑选池；绝不含伙伴域 bot）。 */
export function selectCsChannelBots(
  statuses: readonly IChannelPluginStatus[]
): IChannelPluginStatus[] {
  return statuses.filter((status) => statusInOwnerDomain(status, 'customer_service'));
}

export type CsBotBindingState =
  | { kind: 'boundToThis' }
  | { kind: 'boundToOther'; csAgentId: CsAgentId }
  | { kind: 'unbound' };

/** 绑定态：绑本客服 / 绑其他客服（同域可换绑）/ 未绑定。 */
export function csBotBindingState(
  pluginId: ChannelPluginId,
  csAgentId: CsAgentId,
  ownerByBot: ReadonlyMap<ChannelPluginId, CsAgentId>
): CsBotBindingState {
  const owner = ownerByBot.get(pluginId);
  if (owner == null) return { kind: 'unbound' };
  if (owner === csAgentId) return { kind: 'boundToThis' };
  return { kind: 'boundToOther', csAgentId: owner };
}

/**
 * 创建弹窗打开后新出现的客服域 bot。用快照差分而不是表单回调解析，
 * 因为各平台表单的 create-mode 状态解析是启发式的；这里以业务 UUID
 * 差集精确锁定"这个弹窗创建出来的那一行"。
 */
export function findNewlyCreatedCsBot(
  statuses: readonly IChannelPluginStatus[],
  platform: string,
  knownIds: ReadonlySet<ChannelPluginId>
): IChannelPluginStatus | null {
  return (
    statuses.find(
      (status) =>
        statusInOwnerDomain(status, 'customer_service') &&
        status.type === platform &&
        !knownIds.has(status.plugin_id)
    ) ?? null
  );
}
