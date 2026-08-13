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

interface MattermostConfigFormProps {
  pluginStatus: IChannelPluginStatus | null;
  channelTarget?: ChannelTarget;
  onStatusChange: (status: IChannelPluginStatus | null) => void;
}

/** Mattermost channel config (self-hosted): server URL + bot access token. */
const MattermostConfigForm: React.FC<MattermostConfigFormProps> = ({ pluginStatus, channelTarget, onStatusChange }) => {
  const { t } = useTranslation();
  const [serverUrl, setServerUrl] = useState('');
  const [token, setToken] = useState('');
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
  } = useChannelPairing('mattermost', channelTarget);

  const handleAutoEnable = async () => {
    const config = { credentials: { token: token.trim(), server_url: serverUrl.trim() } };
    const result = await channel.enablePlugin.invoke(buildEnablePluginRequest('mattermost', channelTarget, config));
    if (!result.success) {
      throw new Error(result.error || t('nomi.settings.remoteEnableFailed', { defaultValue: 'Failed to enable channel' }));
    }
    Message.success(t('settings.mattermost.pluginEnabled', 'Mattermost bot enabled'));
    const plugins = await channel.getPluginStatus.invoke();
    if (plugins) {
      const row = findEnabledChannelStatus(plugins, {
        platform: 'mattermost',
        enabledPluginId: result.plugin_id,
        companionId: channelTarget?.companionId,
        ownerDomain: channelTarget?.ownerDomain,
      });
      onStatusChange(row || null);
    }
  };

  const handleTestConnection = async () => {
    if (!serverUrl.trim() || !token.trim()) {
      Message.warning(t('settings.mattermost.credentialsRequired', 'Please enter the server URL and bot token'));
      return;
    }
    setTestLoading(true);
    try {
      const result = await channel.testPlugin.invoke({ plugin_type: 'mattermost', token: token.trim(), extra_config: { server_url: serverUrl.trim() } });
      if (result.success) {
        Message.success(t('settings.mattermost.connectionSuccess', { defaultValue: 'Connected! Bot: {{username}}', username: result.bot_username || 'unknown' }));
        await handleAutoEnable();
      } else {
        Message.error(result.error || t('settings.mattermost.connectionFailed', 'Connection failed'));
      }
    } catch (error: any) {
      Message.error(error.message || t('settings.mattermost.connectionFailed', 'Connection failed'));
    } finally {
      setTestLoading(false);
    }
  };

  const credentialsLocked = !!pluginStatus?.connected;

  return (
    <div className='flex flex-col gap-16px'>
      <div className='flex flex-col gap-8px'>
        <span className='text-14px text-t-primary'>{t('settings.mattermost.serverUrl', 'Server URL')}</span>
        <Input value={serverUrl} onChange={setServerUrl} placeholder='https://mattermost.example.com' disabled={credentialsLocked} />
        <span className='text-14px text-t-primary mt-4px'>{t('settings.mattermost.botToken', 'Bot Token')}</span>
        <Input.Password value={token} onChange={setToken} placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : 'token'} visibilityToggle disabled={credentialsLocked} />
        <div className='text-12px text-t-tertiary'>{t('settings.mattermost.tokensDesc', 'In Mattermost System Console → Integrations → Bot Accounts, create a bot and copy its access token. Interactive buttons are not available (they need a public callback URL).')}</div>
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

export default MattermostConfigForm;
