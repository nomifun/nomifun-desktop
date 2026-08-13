/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import { channel } from '@/common/adapter/ipcBridge';
import { Button, Input, Message } from '@arco-design/web-react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { buildEnablePluginRequest, findEnabledChannelStatus } from '@/renderer/components/channels/channelStatusSelection';
import { AuthorizedUserList, PendingPairingList } from './ChannelPairingLists';
import type { ChannelTarget } from './channelTarget';
import { useChannelPairing } from './useChannelPairing';

interface TwitchConfigFormProps {
  pluginStatus: IChannelPluginStatus | null;
  channelTarget?: ChannelTarget;
  onStatusChange: (status: IChannelPluginStatus | null) => void;
}

/** Twitch chat config: OAuth access token + the channel to join. */
const TwitchConfigForm: React.FC<TwitchConfigFormProps> = ({ pluginStatus, channelTarget, onStatusChange }) => {
  const { t } = useTranslation();
  const [token, setToken] = useState('');
  const [twitchChannel, setTwitchChannel] = useState('');
  const [testLoading, setTestLoading] = useState(false);

  const {
    pendingPairings,
    authorizedUsers,
    pairingLoading,
    usersLoading,
    loadPendingPairings,
    loadAuthorizedUsers,
    approvePairing,
    rejectPairing,
    revokeUser,
  } = useChannelPairing('twitch', channelTarget);

  const handleAutoEnable = async () => {
    const config = { credentials: { token: token.trim(), twitch_channel: twitchChannel.trim() } };
    const result = await channel.enablePlugin.invoke(buildEnablePluginRequest('twitch', channelTarget, config));
    if (!result.success) {
      throw new Error(result.error || t('nomi.settings.remoteEnableFailed', { defaultValue: 'Failed to enable channel' }));
    }
    Message.success(t('settings.twitch.pluginEnabled', 'Twitch bot enabled'));
    const plugins = await channel.getPluginStatus.invoke();
    if (plugins) {
      const row = findEnabledChannelStatus(plugins, {
        platform: 'twitch',
        enabledPluginId: result.plugin_id,
        companionId: channelTarget?.companionId,
        ownerDomain: channelTarget?.ownerDomain,
      });
      onStatusChange(row || null);
    }
  };

  const handleTestConnection = async () => {
    if (!token.trim() || !twitchChannel.trim()) {
      Message.warning(t('settings.twitch.credentialsRequired', 'Please enter an OAuth token and the channel to join'));
      return;
    }
    setTestLoading(true);
    try {
      const result = await channel.testPlugin.invoke({ plugin_type: 'twitch', token: token.trim() });
      if (result.success) {
        Message.success(t('settings.twitch.connectionSuccess', { defaultValue: 'Connected as {{username}}', username: result.bot_username || 'unknown' }));
        await handleAutoEnable();
      } else {
        Message.error(result.error || t('settings.twitch.connectionFailed', 'Connection failed'));
      }
    } catch (error: any) {
      Message.error(error.message || t('settings.twitch.connectionFailed', 'Connection failed'));
    } finally {
      setTestLoading(false);
    }
  };

  const credentialsLocked = !!pluginStatus?.connected;

  return (
    <div className='flex flex-col gap-16px'>
      <div className='flex flex-col gap-8px'>
        <span className='text-14px text-t-primary'>{t('settings.twitch.token', 'OAuth Access Token')}</span>
        <Input.Password value={token} onChange={setToken} placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : 'oauth token (chat:read + chat:write)'} visibilityToggle disabled={credentialsLocked} />
        <span className='text-14px text-t-primary mt-4px'>{t('settings.twitch.channel', 'Channel to join')}</span>
        <Input value={twitchChannel} onChange={setTwitchChannel} placeholder='mychannel' disabled={credentialsLocked} />
        <div className='text-12px text-t-tertiary'>{t('settings.twitch.tokensDesc', 'Generate an OAuth token with chat:read + chat:write scopes (e.g. via the Twitch Token Generator), and enter the channel name the bot should join. Twitch chat is text-only.')}</div>
        <div>
          <Button type='outline' loading={testLoading} onClick={handleTestConnection} disabled={credentialsLocked}>
            {t('settings.channels.testConnection', 'Test')}
          </Button>
        </div>
      </div>

      {pluginStatus?.enabled && (
        <PendingPairingList
          pairings={pendingPairings}
          loading={pairingLoading}
          onRefresh={loadPendingPairings}
          onApprove={approvePairing}
          onReject={rejectPairing}
        />
      )}

      {pluginStatus?.enabled && authorizedUsers.length > 0 && (
        <AuthorizedUserList
          users={authorizedUsers}
          loading={usersLoading}
          onRefresh={loadAuthorizedUsers}
          onRevoke={revokeUser}
        />
      )}
    </div>
  );
};

export default TwitchConfigForm;
