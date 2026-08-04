/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useEffect, useState } from 'react';

import { channel } from '@/common/adapter/ipcBridge';
import type { CompanionId } from '@/common/types/ids';
import { statusInOwnerDomain, statusOwnedBy } from '@/renderer/components/channels/channelStatusSelection';

/**
 * 本伙伴机器人上待审批的配对请求数 —— 远程控制页唯一「有人在等你」的信号。
 *
 * Count of pending pairing requests that belong to a channel plugin owned by
 * THIS companion. Pairings carry the plugin's business UUID, so the plugin
 * roster is what maps them onto a companion; both are refetched on the same
 * live events the settings surfaces listen to. Read-only: approving/rejecting
 * still happens inside the platform config form.
 */
export const usePairingAttention = (companionId: CompanionId | null): number => {
  const [pending, setPending] = useState(0);

  useEffect(() => {
    // A companion switch must not carry the previous companion's count (and its
    // attention dot) into the new one while the refetch is still in flight.
    setPending(0);
    if (!companionId) return;
    let alive = true;

    const refresh = async () => {
      try {
        const [plugins, pairings] = await Promise.all([
          channel.getPluginStatus.invoke(),
          channel.getPendingPairings.invoke(),
        ]);
        if (!alive) return;
        // 渠道所有权分域：只数 companion 域里归本宠的机器人（客服域 bot 自闭环）。
        const mine = new Set(
          (plugins ?? [])
            .filter((plugin) => statusInOwnerDomain(plugin, 'companion') && statusOwnedBy(plugin, { companionId }))
            .map((plugin) => plugin.plugin_id)
        );
        setPending(
          (pairings ?? []).filter((pairing) => pairing.channel_plugin_id && mine.has(pairing.channel_plugin_id)).length
        );
      } catch (error) {
        console.error('[RemoteTab] Failed to count pending pairings:', error);
      }
    };

    void refresh();
    const unsubs = [
      channel.pairingRequested.on(() => void refresh()),
      channel.userAuthorized.on(() => void refresh()),
      channel.pluginStatusChanged.on(() => void refresh()),
    ];
    return () => {
      alive = false;
      unsubs.forEach((unsubscribe) => unsubscribe());
    };
  }, [companionId]);

  return pending;
};
