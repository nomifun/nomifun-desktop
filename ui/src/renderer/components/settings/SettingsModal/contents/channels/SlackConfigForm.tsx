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

interface SlackConfigFormProps {
  pluginStatus: IChannelPluginStatus | null;
  channelTarget?: ChannelTarget;
  onStatusChange: (status: IChannelPluginStatus | null) => void;
}

/**
 * Slack channel config (Socket Mode). Needs two tokens: a bot token (`xoxb-`)
 * and an app-level token (`xapp-`, scope `connections:write`).
 */
const SlackConfigForm: React.FC<SlackConfigFormProps> = ({ pluginStatus, channelTarget, onStatusChange }) => {
  const { t } = useTranslation();
  const [botToken, setBotToken] = useState('');
  const [appToken, setAppToken] = useState('');
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
  } = useChannelPairing('slack', channelTarget);

  const handleAutoEnable = async () => {
    const config = { credentials: { token: botToken.trim(), app_token: appToken.trim() } };
    const result = await channel.enablePlugin.invoke(buildEnablePluginRequest('slack', channelTarget, config));
    if (!result.success) {
      throw new Error(result.error || t('nomi.settings.remoteEnableFailed', { defaultValue: 'Failed to enable channel' }));
    }
    Message.success(t('settings.slack.pluginEnabled', 'Slack bot enabled'));
    const plugins = await channel.getPluginStatus.invoke();
    if (plugins) {
      const row = findEnabledChannelStatus(plugins, {
        platform: 'slack',
        enabledPluginId: result.plugin_id,
        companionId: channelTarget?.companionId,
        ownerDomain: channelTarget?.ownerDomain,
      });
      onStatusChange(row || null);
    }
  };

  const handleTestConnection = async () => {
    if (!botToken.trim() || !appToken.trim()) {
      Message.warning(t('settings.slack.credentialsRequired', 'Please enter both the bot token and the app-level token'));
      return;
    }
    setTestLoading(true);
    try {
      const result = await channel.testPlugin.invoke({ plugin_type: 'slack', token: botToken.trim(), extra_config: { app_token: appToken.trim() } });
      if (result.success) {
        Message.success(t('settings.slack.connectionSuccess', { defaultValue: 'Connected! Bot: {{username}}', username: result.bot_username || 'unknown' }));
        await handleAutoEnable();
      } else {
        Message.error(result.error || t('settings.slack.connectionFailed', 'Connection failed'));
      }
    } catch (error: any) {
      Message.error(error.message || t('settings.slack.connectionFailed', 'Connection failed'));
    } finally {
      setTestLoading(false);
    }
  };

  const credentialsLocked = !!pluginStatus?.connected;

  return (
    <div className='flex flex-col gap-16px'>
      <div className='flex flex-col gap-8px'>
        <span className='text-14px text-t-primary'>{t('settings.slack.botToken', 'Bot Token (xoxb-)')}</span>
        <Input.Password value={botToken} onChange={setBotToken} placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : 'xoxb-...'} visibilityToggle disabled={credentialsLocked} />
        <span className='text-14px text-t-primary mt-4px'>{t('settings.slack.appToken', 'App-Level Token (xapp-)')}</span>
        <Input.Password value={appToken} onChange={setAppToken} placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : 'xapp-...'} visibilityToggle disabled={credentialsLocked} />
        <div className='text-12px text-t-tertiary'>{t('settings.slack.tokensDesc', 'In Slack API → your app: enable Socket Mode (gives the xapp- app token with connections:write), and install the app to get the xoxb- bot token.')}</div>
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

export default SlackConfigForm;
