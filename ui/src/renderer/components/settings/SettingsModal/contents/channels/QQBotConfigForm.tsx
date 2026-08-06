/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import { channel } from '@/common/adapter/ipcBridge';
import { findEnabledChannelStatus, buildEnablePluginRequest } from '@/renderer/components/channels/channelStatusSelection';
import { Button, Input, Message } from '@arco-design/web-react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AuthorizedUserList, PendingPairingList } from './ChannelPairingLists';
import type { ChannelTarget } from './channelTarget';
import { useChannelPairing } from './useChannelPairing';

interface QQBotConfigFormProps {
  pluginStatus: IChannelPluginStatus | null;
  channelTarget?: ChannelTarget;
  onStatusChange: (status: IChannelPluginStatus | null) => void;
  onCredentialsChange?: (credentials: { appId: string; clientSecret: string }) => void;
}

/** QQ Bot config: AppID + ClientSecret (OAuth2 client-credentials). */
const QQBotConfigForm: React.FC<QQBotConfigFormProps> = ({
  pluginStatus,
  channelTarget,
  onStatusChange,
  onCredentialsChange,
}) => {
  const { t } = useTranslation();
  const [appId, setAppId] = useState('');
  const [clientSecret, setClientSecret] = useState('');
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
  } = useChannelPairing('qqbot', channelTarget);

  // Single source of truth: mirror the typed credentials up to the parent
  // (PlatformConfigBody's qqbotCredentialsRef) whenever they change, so the
  // shared 「启用渠道」 switch can enable with as-yet-unsaved credentials.
  useEffect(() => {
    onCredentialsChange?.({ appId, clientSecret });
  }, [appId, clientSecret, onCredentialsChange]);

  const handleAutoEnable = async () => {
    const config = { credentials: { client_id: appId.trim(), client_secret: clientSecret.trim() } };
    const result = await channel.enablePlugin.invoke(buildEnablePluginRequest('qqbot', channelTarget, config));
    if (!result.success) {
      throw new Error(
        result.error ||
          t('nomi.settings.remoteEnableFailed', { defaultValue: 'Failed to enable channel' })
      );
    }
    Message.success(t('settings.qqbot.pluginEnabled', 'QQ bot enabled'));
    const plugins = await channel.getPluginStatus.invoke();
    if (plugins) {
      // Prefer the business UUID returned by the backend. This survives
      // create-mode creation and identity reuse, then falls back to owner scope.
      const row = findEnabledChannelStatus(plugins, {
        platform: 'qqbot',
        enabledPluginId: result.plugin_id,
        companionId: channelTarget?.companionId,
        ownerDomain: channelTarget?.ownerDomain,
      });
      // Only report a resolved plugin — feeding the parent `null` would skip its
      // optimistic merge + retarget (the adopt effect + next refresh still heal).
      if (row) onStatusChange(row);
    }
  };

  const handleTestConnection = async () => {
    if (!appId.trim() || !clientSecret.trim()) {
      Message.warning(t('settings.qqbot.credentialsRequired', 'Please enter the AppID and ClientSecret'));
      return;
    }
    setTestLoading(true);
    try {
      const result = await channel.testPlugin.invoke({ plugin_type: 'qqbot', token: appId.trim(), extra_config: { app_secret: clientSecret.trim() } });
      if (result.success) {
        Message.success(t('settings.qqbot.connectionSuccess', { defaultValue: 'Connected! AppID: {{appId}}', appId: result.bot_username || appId.trim() }));
        await handleAutoEnable();
      } else {
        Message.error(result.error || t('settings.qqbot.connectionFailed', 'Connection failed'));
      }
    } catch (error: any) {
      Message.error(error.message || t('settings.qqbot.connectionFailed', 'Connection failed'));
    } finally {
      setTestLoading(false);
    }
  };

  const credentialsLocked = !!pluginStatus?.connected;

  return (
    <div className='flex flex-col gap-16px'>
      <div className='flex flex-col gap-8px'>
        <span className='text-14px text-t-primary'>{t('settings.qqbot.appId', 'AppID')}</span>
        <Input value={appId} onChange={setAppId} placeholder='102xxxxxx' disabled={credentialsLocked} />
        <span className='text-14px text-t-primary mt-4px'>{t('settings.qqbot.clientSecret', 'ClientSecret')}</span>
        <Input.Password value={clientSecret} onChange={setClientSecret} placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : 'client secret'} visibilityToggle disabled={credentialsLocked} />
        <div className='text-12px text-t-tertiary'>{t('settings.qqbot.tokensDesc', 'Create a bot on the QQ Open Platform (q.qq.com) and copy its AppID + ClientSecret.')}</div>
        <div>
          <Button type='outline' loading={testLoading} onClick={handleTestConnection} disabled={credentialsLocked}>
            {t('settings.channels.testConnection', 'Test')}
          </Button>
        </div>
      </div>

      {/* Intent reminder: GROUP_AND_C2C needs console approval. */}
      <div className='bg-[rgba(var(--primary-rgb),0.08)] rd-12px p-12px border border-solid border-[rgba(var(--primary-rgb),0.2)] text-12px text-t-secondary'>
        {t('settings.qqbot.intentNote', 'In the QQ Open Platform console → bot management → permissions, apply for the "GROUP_AND_C2C" intent, otherwise the bot cannot receive group/private messages.')}
      </div>

      {pluginStatus?.enabled && authorizedUsers.length === 0 && (
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

export default QQBotConfigForm;
