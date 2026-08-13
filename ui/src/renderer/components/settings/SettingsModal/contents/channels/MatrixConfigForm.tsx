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

interface MatrixConfigFormProps {
  pluginStatus: IChannelPluginStatus | null;
  channelTarget?: ChannelTarget;
  onStatusChange: (status: IChannelPluginStatus | null) => void;
}

/**
 * Matrix channel config. Needs the homeserver URL, the bot's user id (mxid)
 * and an access token. v1 supports unencrypted rooms only (no E2EE — see the
 * design spec: matrix-sdk's crypto stack conflicts with the workspace deps).
 */
const MatrixConfigForm: React.FC<MatrixConfigFormProps> = ({ pluginStatus, channelTarget, onStatusChange }) => {
  const { t } = useTranslation();
  const [homeserver, setHomeserver] = useState('');
  const [userId, setUserId] = useState('');
  const [accessToken, setAccessToken] = useState('');
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
  } = useChannelPairing('matrix', channelTarget);

  const handleAutoEnable = async () => {
    const config = { credentials: { access_token: accessToken.trim(), homeserver_url: homeserver.trim(), user_id: userId.trim() } };
    const result = await channel.enablePlugin.invoke(buildEnablePluginRequest('matrix', channelTarget, config));
    if (!result.success) {
      throw new Error(result.error || t('nomi.settings.remoteEnableFailed', { defaultValue: 'Failed to enable channel' }));
    }
    Message.success(t('settings.matrix.pluginEnabled', 'Matrix bot enabled'));
    const plugins = await channel.getPluginStatus.invoke();
    if (plugins) {
      const row = findEnabledChannelStatus(plugins, {
        platform: 'matrix',
        enabledPluginId: result.plugin_id,
        companionId: channelTarget?.companionId,
        ownerDomain: channelTarget?.ownerDomain,
      });
      onStatusChange(row || null);
    }
  };

  const handleTestConnection = async () => {
    if (!homeserver.trim() || !userId.trim() || !accessToken.trim()) {
      Message.warning(t('settings.matrix.credentialsRequired', 'Please enter homeserver URL, user id and access token'));
      return;
    }
    setTestLoading(true);
    try {
      const result = await channel.testPlugin.invoke({ plugin_type: 'matrix', token: accessToken.trim(), extra_config: { homeserver_url: homeserver.trim(), user_id: userId.trim() } });
      if (result.success) {
        Message.success(t('settings.matrix.connectionSuccess', { defaultValue: 'Connected as {{username}}', username: result.bot_username || userId.trim() }));
        await handleAutoEnable();
      } else {
        Message.error(result.error || t('settings.matrix.connectionFailed', 'Connection failed'));
      }
    } catch (error: any) {
      Message.error(error.message || t('settings.matrix.connectionFailed', 'Connection failed'));
    } finally {
      setTestLoading(false);
    }
  };

  const credentialsLocked = !!pluginStatus?.connected;

  return (
    <div className='flex flex-col gap-16px'>
      <div className='flex flex-col gap-8px'>
        <span className='text-14px text-t-primary'>{t('settings.matrix.homeserver', 'Homeserver URL')}</span>
        <Input value={homeserver} onChange={setHomeserver} placeholder='https://matrix.org' disabled={credentialsLocked} />
        <span className='text-14px text-t-primary mt-4px'>{t('settings.matrix.userId', 'Bot User ID (mxid)')}</span>
        <Input value={userId} onChange={setUserId} placeholder='@mybot:matrix.org' disabled={credentialsLocked} />
        <span className='text-14px text-t-primary mt-4px'>{t('settings.matrix.accessToken', 'Access Token')}</span>
        <Input.Password value={accessToken} onChange={setAccessToken} placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : 'syt_...'} visibilityToggle disabled={credentialsLocked} />
        <div className='text-12px text-t-tertiary'>{t('settings.matrix.tokensDesc', 'Create a bot user on your homeserver and obtain its access token (e.g. via the login API). v1 supports unencrypted rooms only.')}</div>
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

export default MatrixConfigForm;
