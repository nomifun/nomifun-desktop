/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import { channel } from '@/common/adapter/ipcBridge';
import { Button, Input, Message, Tooltip } from '@arco-design/web-react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { buildEnablePluginRequest, findEnabledChannelStatus } from '@/renderer/components/channels/channelStatusSelection';
import { AuthorizedUserList, PendingPairingList } from './ChannelPairingLists';
import type { ChannelTarget } from './channelTarget';
import { useChannelPairing } from './useChannelPairing';

/** Preference row */
const PreferenceRow: React.FC<{
  label: string;
  description?: React.ReactNode;
  children: React.ReactNode;
}> = ({ label, description, children }) => (
  <div className='flex items-center justify-between gap-24px py-12px'>
    <div className='flex-1'>
      <span className='text-14px text-t-primary'>{label}</span>
      {description && <div className='text-12px text-t-tertiary mt-2px'>{description}</div>}
    </div>
    <div className='flex items-center'>{children}</div>
  </div>
);

interface DiscordConfigFormProps {
  pluginStatus: IChannelPluginStatus | null;
  /** 多机器人模式下寻址的渠道行；缺省 = 全局设置页 legacy 单行行为。 */
  channelTarget?: ChannelTarget;
  onStatusChange: (status: IChannelPluginStatus | null) => void;
  onTokenChange?: (token: string) => void;
}

const DiscordConfigForm: React.FC<DiscordConfigFormProps> = ({ pluginStatus, channelTarget, onStatusChange, onTokenChange }) => {
  const { t } = useTranslation();

  const [discordToken, setDiscordToken] = useState('');
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
  } = useChannelPairing('discord', channelTarget);

  const handleAutoEnable = async () => {
    try {
      const config = { credentials: { token: discordToken.trim() } };
      const result = await channel.enablePlugin.invoke(buildEnablePluginRequest('discord', channelTarget, config));
      if (!result.success) {
        throw new Error(result.error || t('nomi.settings.remoteEnableFailed', { defaultValue: 'Failed to enable channel' }));
      }
      Message.success(t('settings.discord.pluginEnabled', 'Discord bot enabled'));
      const plugins = await channel.getPluginStatus.invoke();
      if (plugins) {
        const discordPlugin = findEnabledChannelStatus(plugins, {
          platform: 'discord',
          enabledPluginId: result.plugin_id,
          companionId: channelTarget?.companionId,
          ownerDomain: channelTarget?.ownerDomain,
        });
        onStatusChange(discordPlugin || null);
      }
    } catch (error: unknown) {
      console.error('[ChannelSettings] Auto-enable failed:', error);
      Message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const handleTestConnection = async () => {
    if (!discordToken.trim()) {
      Message.warning(t('settings.discord.tokenRequired', 'Please enter a bot token'));
      return;
    }
    setTestLoading(true);
    try {
      const result = await channel.testPlugin.invoke({ plugin_type: 'discord', token: discordToken.trim() });
      if (result.success) {
        Message.success(t('settings.discord.connectionSuccess', { defaultValue: 'Connected! Bot: {{username}}', username: result.bot_username || 'unknown' }));
        await handleAutoEnable();
      } else {
        Message.error(result.error || t('settings.discord.connectionFailed', 'Connection failed'));
      }
    } catch (error: any) {
      Message.error(error.message || t('settings.discord.connectionFailed', 'Connection failed'));
    } finally {
      setTestLoading(false);
    }
  };

  const handleTokenChange = (value: string) => {
    setDiscordToken(value);
    onTokenChange?.(value);
  };

  // Row-scoped credential lock — lock only when THIS bot row is live.
  const credentialsLocked = !!pluginStatus?.connected;

  return (
    <div className='flex flex-col gap-24px'>
      <PreferenceRow label={t('settings.discord.botToken', 'Bot Token')} description={t('settings.discord.botTokenDesc', 'Create an application at the Discord Developer Portal, add a Bot, and copy its token.')}>
        <div className='flex items-center gap-8px'>
          <Input.Password value={discordToken} onChange={handleTokenChange} placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : 'MTxxxxxxxx.Gxxxxx.xxxxxxxx'} style={{ width: 240 }} visibilityToggle disabled={credentialsLocked} />
          {credentialsLocked ? (
            <Tooltip content={t('settings.discord.tokenLocked', 'Disable this channel before modifying the configuration')}>
              <span>
                <Button type='outline' loading={testLoading} onClick={handleTestConnection} disabled={credentialsLocked}>
                  {t('settings.channels.testConnection', 'Test')}
                </Button>
              </span>
            </Tooltip>
          ) : (
            <Button type='outline' loading={testLoading} onClick={handleTestConnection} disabled={credentialsLocked}>
              {t('settings.channels.testConnection', 'Test')}
            </Button>
          )}
        </div>
      </PreferenceRow>

      {/* Privileged-intent reminder: Discord requires the Message Content Intent. */}
      <div className='bg-[rgba(var(--primary-rgb),0.08)] rd-12px p-12px border border-solid border-[rgba(var(--primary-rgb),0.2)] text-12px text-t-secondary'>
        {t('settings.discord.intentNote', 'In the Developer Portal → Bot → Privileged Gateway Intents, enable "Message Content Intent", otherwise the bot cannot read message text. Invite the bot to your server (or DM it) to start.')}
      </div>

      {/* Pending Pairings */}
      {pluginStatus?.enabled && authorizedUsers.length === 0 && (
        <PendingPairingList
          pairings={pendingPairings}
          loading={pairingLoading}
          onRefresh={loadPendingPairings}
          onApprove={approvePairing}
          onReject={rejectPairing}
          showCopyButton
        />
      )}

      {/* Authorized Users */}
      {pluginStatus?.enabled && authorizedUsers.length > 0 && (
        <AuthorizedUserList
          users={authorizedUsers}
          loading={usersLoading}
          onRefresh={loadAuthorizedUsers}
          onRevoke={revokeUser}
          showMeta
        />
      )}
    </div>
  );
};

export default DiscordConfigForm;
