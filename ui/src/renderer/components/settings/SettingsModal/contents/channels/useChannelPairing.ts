/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IChannelPairingRequest, IChannelUser } from '@/common/types/channel/channel';
import type { ChannelUserId } from '@/common/types/ids';
import { channel } from '@/common/adapter/ipcBridge';
import { Message } from '@arco-design/web-react';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ChannelPlatform, ChannelTarget } from './channelTarget';

/**
 * Shared pairing/authorized-user state machine for the channel config forms.
 *
 * Encapsulates the per-platform pending-pairing + authorized-user loads, the
 * `pairingRequested` / `userAuthorized` push subscriptions (scoped to a bot row
 * when a `channelTarget` addresses one), and the approve/reject/revoke actions.
 *
 * NOTE: WeixinConfigForm intentionally does NOT use this hook — its inbound
 * pairing flow carries sequence-guarded optimistic-mutation replay (see
 * weixinConfigState.ts) that this simple load/refresh model would regress.
 */
export function useChannelPairing(platformType: ChannelPlatform, channelTarget?: ChannelTarget) {
  const { t } = useTranslation();

  const [pairingLoading, setPairingLoading] = useState(false);
  const [usersLoading, setUsersLoading] = useState(false);
  const [pendingPairings, setPendingPairings] = useState<IChannelPairingRequest[]>([]);
  const [authorizedUsers, setAuthorizedUsers] = useState<IChannelUser[]>([]);

  const channelPluginId = channelTarget?.channelPluginId;

  // Load pending pairings for this platform (scoped to the addressed bot row).
  const loadPendingPairings = useCallback(async () => {
    setPairingLoading(true);
    try {
      const pairings = await channel.getPendingPairings.invoke();
      if (pairings) {
        setPendingPairings(
          pairings.filter((p) => p.platformType === platformType && (!channelPluginId || p.channel_plugin_id === channelPluginId))
        );
      }
    } catch (error) {
      console.error('[ChannelPairing] Failed to load pending pairings:', error);
    } finally {
      setPairingLoading(false);
    }
  }, [platformType, channelPluginId]);

  // Load authorized users for this platform (scoped to the addressed bot row).
  const loadAuthorizedUsers = useCallback(async () => {
    setUsersLoading(true);
    try {
      const users = await channel.getAuthorizedUsers.invoke();
      if (users) {
        setAuthorizedUsers(
          users.filter((u) => u.platformType === platformType && (!channelPluginId || u.channel_plugin_id === channelPluginId))
        );
      }
    } catch (error) {
      console.error('[ChannelPairing] Failed to load authorized users:', error);
    } finally {
      setUsersLoading(false);
    }
  }, [platformType, channelPluginId]);

  // Initial load
  useEffect(() => {
    void loadPendingPairings();
    void loadAuthorizedUsers();
  }, [loadPendingPairings, loadAuthorizedUsers]);

  // Listen for pairing requests
  useEffect(() => {
    const unsubscribe = channel.pairingRequested.on((request) => {
      if (request.platformType !== platformType) return;
      if (channelPluginId && request.channel_plugin_id !== channelPluginId) return;
      setPendingPairings((prev) => (prev.some((p) => p.code === request.code) ? prev : [request, ...prev]));
    });
    return () => unsubscribe();
  }, [platformType, channelPluginId]);

  // Listen for user authorization
  useEffect(() => {
    const unsubscribe = channel.userAuthorized.on((user) => {
      if (user.platformType !== platformType) return;
      if (channelPluginId && user.channel_plugin_id !== channelPluginId) return;
      setAuthorizedUsers((prev) => (prev.some((u) => u.channel_user_id === user.channel_user_id) ? prev : [user, ...prev]));
      setPendingPairings((prev) => prev.filter((p) => p.platformUserId !== user.platformUserId));
    });
    return () => unsubscribe();
  }, [platformType, channelPluginId]);

  const approvePairing = useCallback(
    async (code: string) => {
      try {
        await channel.approvePairing.invoke({ code });
        Message.success(t('settings.channels.pairingApproved', 'Pairing approved'));
        await loadPendingPairings();
        await loadAuthorizedUsers();
      } catch (error: unknown) {
        Message.error(error instanceof Error ? error.message : String(error));
      }
    },
    [loadPendingPairings, loadAuthorizedUsers, t]
  );

  const rejectPairing = useCallback(
    async (code: string) => {
      try {
        await channel.rejectPairing.invoke({ code });
        Message.info(t('settings.channels.pairingRejected', 'Pairing rejected'));
        await loadPendingPairings();
      } catch (error: unknown) {
        Message.error(error instanceof Error ? error.message : String(error));
      }
    },
    [loadPendingPairings, t]
  );

  const revokeUser = useCallback(
    async (channel_user_id: ChannelUserId) => {
      try {
        await channel.revokeUser.invoke({ channel_user_id });
        Message.success(t('settings.channels.userRevoked', 'User access revoked'));
        await loadAuthorizedUsers();
      } catch (error: unknown) {
        Message.error(error instanceof Error ? error.message : String(error));
      }
    },
    [loadAuthorizedUsers, t]
  );

  return {
    pendingPairings,
    authorizedUsers,
    pairingLoading,
    usersLoading,
    loadPendingPairings,
    loadAuthorizedUsers,
    approvePairing,
    rejectPairing,
    revokeUser,
  };
}
