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

/**
 * Preference row component
 */
const PreferenceRow: React.FC<{
  label: string;
  description?: React.ReactNode;
  extra?: React.ReactNode;
  children: React.ReactNode;
}> = ({ label, description, extra, children }) => (
  <div className='flex items-center justify-between gap-24px py-12px'>
    <div className='flex-1'>
      <div className='flex items-center gap-8px'>
        <span className='text-14px text-t-primary'>{label}</span>
        {extra}
      </div>
      {description && <div className='text-12px text-t-tertiary mt-2px'>{description}</div>}
    </div>
    <div className='flex items-center'>{children}</div>
  </div>
);

/**
 * Section header component
 */
const SectionHeader: React.FC<{ title: string; action?: React.ReactNode }> = ({ title, action }) => (
  <div className='flex items-center justify-between mb-12px'>
    <h3 className='text-14px font-500 text-t-primary m-0'>{title}</h3>
    {action}
  </div>
);

interface TelegramConfigFormProps {
  pluginStatus: IChannelPluginStatus | null;
  /** 多机器人模式下寻址的渠道行；缺省 = 全局设置页 legacy 单行行为。 */
  channelTarget?: ChannelTarget;
  onStatusChange: (status: IChannelPluginStatus | null) => void;
  onTokenChange?: (token: string) => void;
}

const TelegramConfigForm: React.FC<TelegramConfigFormProps> = ({
  pluginStatus,
  channelTarget,
  onStatusChange,
  onTokenChange,
}) => {
  const { t } = useTranslation();

  const [telegramToken, setTelegramToken] = useState('');
  const [testLoading, setTestLoading] = useState(false);
  const [, setTokenTested] = useState(false);
  const [, setTestedBotUsername] = useState<string | null>(null);

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
  } = useChannelPairing('telegram', channelTarget);

  // Test Telegram connection
  const handleTestConnection = async () => {
    if (!telegramToken.trim()) {
      Message.warning(t('settings.channels.tokenRequired', 'Please enter a bot token'));
      return;
    }

    setTestLoading(true);
    setTokenTested(false);
    setTestedBotUsername(null);
    try {
      // testPlugin returns { success, botUsername?, error? } directly
      const result = await channel.testPlugin.invoke({
        plugin_type: 'telegram',
        token: telegramToken.trim(),
      });

      if (result.success) {
        setTokenTested(true);
        setTestedBotUsername(result.bot_username || null);
        Message.success(
          t('settings.channels.connectionSuccess', {
            defaultValue: 'Connected! Bot: @{{username}}',
            username: result.bot_username || 'unknown',
          })
        );

        // Auto-enable bot after successful test
        await handleAutoEnable();
      } else {
        setTokenTested(false);
        Message.error(result.error || t('settings.channels.connectionFailed', 'Connection failed'));
      }
    } catch (error: any) {
      setTokenTested(false);
      Message.error(error.message || t('settings.channels.connectionFailed', 'Connection failed'));
    } finally {
      setTestLoading(false);
    }
  };

  // Auto-enable plugin after successful test
  const handleAutoEnable = async () => {
    try {
      const config = { credentials: { token: telegramToken.trim() } };
      const result = await channel.enablePlugin.invoke(buildEnablePluginRequest('telegram', channelTarget, config));
      if (!result.success) {
        throw new Error(result.error || t('nomi.settings.remoteEnableFailed', { defaultValue: 'Failed to enable channel' }));
      }

      Message.success(t('settings.channels.pluginEnabled', 'Telegram bot enabled'));
      const plugins = await channel.getPluginStatus.invoke();
      if (plugins) {
        // Multi-plugin model: resolve by the backend-returned business UUID, or by owner scope after create.
        const telegramPlugin = findEnabledChannelStatus(plugins, {
          platform: 'telegram',
          enabledPluginId: result.plugin_id,
          companionId: channelTarget?.companionId,
          ownerDomain: channelTarget?.ownerDomain,
        });
        onStatusChange(telegramPlugin || null);
      }
    } catch (error: unknown) {
      console.error('[ChannelSettings] Auto-enable failed:', error);
      Message.error(error instanceof Error ? error.message : String(error));
    }
  };

  // Reset token tested state when token changes
  const handleTokenChange = (value: string) => {
    setTelegramToken(value);
    setTokenTested(false);
    setTestedBotUsername(null);
    onTokenChange?.(value);
  };

  // Row-scoped credential lock — see LarkConfigForm for the rationale (was a
  // global per-platform `authorizedUsers.length > 0`, which froze a second
  // companion's create form). Lock only when THIS bot row is live.
  const credentialsLocked = !!pluginStatus?.connected;

  return (
    <div className='flex flex-col gap-24px'>
      <PreferenceRow
        label={t('settings.channels.botToken', 'Bot Token')}
        description={t(
          'settings.channels.botTokenDesc',
          'Open Telegram, find @BotFather and send /newbot to get your Bot Token.'
        )}
      >
        <div className='flex items-center gap-8px'>
          {credentialsLocked ? (
            <Tooltip
              content={t(
                'settings.channels.tokenLocked',
                'Please close the Channel and delete all authorized users before modifying the configuration'
              )}
            >
              <span>
                <Input.Password
                  value={telegramToken}
                  onChange={handleTokenChange}
                  placeholder={
                    pluginStatus?.hasToken ? '••••••••••••••••' : '123456:ABC-DEF...'
                  }
                  style={{ width: 240 }}
                  visibilityToggle
                  disabled={credentialsLocked}
                />
              </span>
            </Tooltip>
          ) : (
            <Input.Password
              value={telegramToken}
              onChange={handleTokenChange}
              placeholder={
                pluginStatus?.hasToken ? '••••••••••••••••' : '123456:ABC-DEF...'
              }
              style={{ width: 240 }}
              visibilityToggle
              disabled={credentialsLocked}
            />
          )}
          {credentialsLocked ? (
            <Tooltip
              content={t(
                'settings.channels.tokenLocked',
                'Please close the Channel and delete all authorized users before modifying the configuration'
              )}
            >
              <span>
                <Button
                  type='outline'
                  loading={testLoading}
                  onClick={handleTestConnection}
                  disabled={credentialsLocked}
                >
                  {t('settings.channels.testConnection', 'Test')}
                </Button>
              </span>
            </Tooltip>
          ) : (
            <Button
              type='outline'
              loading={testLoading}
              onClick={handleTestConnection}
              disabled={credentialsLocked}
            >
              {t('settings.channels.testConnection', 'Test')}
            </Button>
          )}
        </div>
      </PreferenceRow>


      {/* Next Steps Guide - show when bot is enabled and no authorized users yet */}
      {pluginStatus?.enabled && pluginStatus?.connected && authorizedUsers.length === 0 && (
        <div className='bg-[rgba(var(--primary-rgb),0.08)] rd-12px p-16px border border-[rgba(var(--primary-rgb),0.2)]'>
          <SectionHeader title={t('settings.channels.nextSteps', 'Next Steps')} />
          <div className='text-14px text-t-secondary space-y-8px'>
            <p className='m-0'>
              <strong>1.</strong> {t('settings.channels.step1', 'Open Telegram and search for your bot')}
              {pluginStatus.botUsername && (
                <span className='ml-4px'>
                  <code className='bg-fill-2 px-6px py-2px rd-4px'>@{pluginStatus.botUsername}</code>
                </span>
              )}
            </p>
            <p className='m-0'>
              <strong>2.</strong>{' '}
              {t('settings.channels.step2', 'Send any message or click /start to initiate pairing')}
            </p>
            <p className='m-0'>
              <strong>3.</strong>{' '}
              {t(
                'settings.channels.step3',
                'A pairing request will appear below. Click "Approve" to authorize the user.'
              )}
            </p>
            <p className='m-0'>
              <strong>4.</strong>{' '}
              {t('settings.channels.step4', 'Once approved, you can start chatting with Gemini through Telegram!')}
            </p>
          </div>
        </div>
      )}

      {/* Pending Pairings - show when bot is enabled and no authorized users yet */}
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

      {/* Authorized Users - show when there are authorized users */}
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

export default TelegramConfigForm;
