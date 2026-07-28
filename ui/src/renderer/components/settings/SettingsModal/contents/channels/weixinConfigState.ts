/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  IChannelPairingRequest,
  IChannelPluginStatus,
  IChannelUser,
} from '@/common/types/channel/channel';
import type { ChannelPluginId } from '@/common/types/ids';

type WeixinRuntimeStatus = Pick<IChannelPluginStatus, 'enabled' | 'connected' | 'status'>;

export type WeixinEnableConfig = Record<string, unknown> & {
  credentials: {
    account_id: string;
    bot_token: string;
    /**
     * `PluginCredentials.extra` is `serde(flatten)`, so the login endpoint's
     * negotiated API origin belongs directly in `credentials`.
     */
    baseUrl?: string;
  };
};

export type WeixinPairingMutation =
  | {
      sequence: number;
      type: 'upsert';
      request: IChannelPairingRequest;
    }
  | {
      sequence: number;
      type: 'remove-user';
      platformUserId: string;
      channelPluginId?: ChannelPluginId;
    };

export type WeixinAuthorizedUserMutation = {
  sequence: number;
  user: IChannelUser;
};

/**
 * Credentials being present only means the plugin can be started. The channel
 * is connected exclusively when the enabled runtime itself reports Running.
 */
export function isWeixinRuntimeConnected(
  status: WeixinRuntimeStatus | null | undefined
): boolean {
  return Boolean(status?.enabled && status.connected && status.status === 'running');
}

export function buildWeixinEnableConfig(
  accountId: string,
  botToken: string,
  baseUrl?: string
): WeixinEnableConfig {
  const normalizedBaseUrl = baseUrl?.trim();
  return {
    credentials: {
      account_id: accountId,
      bot_token: botToken,
      ...(normalizedBaseUrl ? { baseUrl: normalizedBaseUrl } : {}),
    },
  };
}

/**
 * Resolve only the entity created/updated by the enable response. Owner/type
 * fallbacks can select a different WeChat bot in multi-plugin mode.
 */
export function findWeixinPluginStatusById(
  statuses: readonly IChannelPluginStatus[],
  pluginId: ChannelPluginId
): IChannelPluginStatus | null {
  return (
    statuses.find((status) => status.plugin_id === pluginId && status.type === 'weixin') ??
    null
  );
}

/**
 * Apply pairing events that arrived after an HTTP snapshot request started.
 * This lets the snapshot remain authoritative while preventing a slow response
 * from erasing a newer WebSocket upsert/removal.
 */
export function applyWeixinPairingMutations(
  snapshot: readonly IChannelPairingRequest[],
  mutations: readonly WeixinPairingMutation[]
): IChannelPairingRequest[] {
  let result = [...snapshot];

  for (const mutation of [...mutations].sort((left, right) => left.sequence - right.sequence)) {
    if (mutation.type === 'upsert') {
      result = [
        mutation.request,
        ...result.filter((pairing) => pairing.code !== mutation.request.code),
      ];
      continue;
    }

    result = result.filter((pairing) => {
      if (pairing.platformUserId !== mutation.platformUserId) return true;
      return (
        mutation.channelPluginId !== undefined &&
        pairing.channel_plugin_id !== mutation.channelPluginId
      );
    });
  }

  return result;
}

/**
 * Replay authorization events that arrived after an HTTP user-list snapshot
 * started so a stale response cannot erase a newly approved user.
 */
export function applyWeixinAuthorizedUserMutations(
  snapshot: readonly IChannelUser[],
  mutations: readonly WeixinAuthorizedUserMutation[]
): IChannelUser[] {
  let result = [...snapshot];

  for (const mutation of [...mutations].sort(
    (left, right) => left.sequence - right.sequence
  )) {
    result = [
      mutation.user,
      ...result.filter(
        (user) => user.channel_user_id !== mutation.user.channel_user_id
      ),
    ];
  }

  return result;
}
