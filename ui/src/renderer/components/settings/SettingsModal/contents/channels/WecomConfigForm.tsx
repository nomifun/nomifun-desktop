/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import { channel, type IWebUIStatus } from '@/common/adapter/ipcBridge';
import { openExternalUrl } from '@/renderer/utils/platform';
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
  required?: boolean;
  children: React.ReactNode;
}> = ({ label, description, extra, required, children }) => (
  <div className='flex items-center justify-between gap-24px py-12px'>
    <div className='flex-1'>
      <div className='flex items-center gap-8px'>
        <span className='text-14px text-t-primary'>
          {label}
          {required && <span className='text-red-500 ml-2px'>*</span>}
        </span>
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

interface WecomConfigFormProps {
  pluginStatus: IChannelPluginStatus | null;
  /** 多机器人模式下寻址的渠道行；缺省 = 全局设置页 legacy 单行行为。 */
  channelTarget?: ChannelTarget;
  onStatusChange: (status: IChannelPluginStatus | null) => void;
  webuiStatus: IWebUIStatus | null;
}

const WECOM_DEV_DOCS_URL = 'https://developer.work.weixin.qq.com/document/path/101463';

const WecomConfigForm: React.FC<WecomConfigFormProps> = ({
  pluginStatus,
  channelTarget,
  onStatusChange,
  webuiStatus,
}) => {
  const { t } = useTranslation();

  const [botId, setBotId] = useState('');
  const [secret, setSecret] = useState('');

  const [saveLoading, setSaveLoading] = useState(false);
  const [touched, setTouched] = useState({ botId: false, secret: false });
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
  } = useChannelPairing('wecom', channelTarget);

  const handleSaveAndEnable = async () => {
    setTouched({ botId: true, secret: true });
    const id = botId.trim();
    const sec = secret.trim();
    if (!id || !sec) {
      Message.warning(t('settings.wecom.credentialsRequired', 'Please enter Bot ID and Secret'));
      return;
    }

    setSaveLoading(true);
    try {
      const config = {
        credentials: {
          bot_id: id,
          secret: sec,
        },
      };
      const result = await channel.enablePlugin.invoke(buildEnablePluginRequest('wecom', channelTarget, config));
      if (!result.success) {
        throw new Error(result.error || t('nomi.settings.remoteEnableFailed', { defaultValue: 'Failed to enable channel' }));
      }

      Message.success(t('settings.wecom.pluginEnabled', 'WeCom channel enabled'));
      const plugins = await channel.getPluginStatus.invoke();
      if (plugins) {
        // Multi-plugin model: resolve by the backend-returned business UUID, or by owner scope after create.
        const wecomPlugin = findEnabledChannelStatus(plugins, {
          platform: 'wecom',
          enabledPluginId: result.plugin_id,
          companionId: channelTarget?.companionId,
          ownerDomain: channelTarget?.ownerDomain,
        });
        onStatusChange(wecomPlugin || null);
      }
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      console.error('[WecomConfig] Save failed:', error);
      Message.error(message || t('settings.wecom.enableFailed', 'Failed to enable WeCom channel'));
    } finally {
      setSaveLoading(false);
    }
  };

  const handleCredentialsChange = () => {
    /* reserved for future “dirty” indicators */
  };

  // Row-scoped credential lock — see LarkConfigForm for the rationale (was a
  // global per-platform `authorizedUsers.length > 0`, which froze a second
  // companion's create form). Lock only when THIS bot row is live.
  const credentialsLocked = !!pluginStatus?.connected;

  return (
    <div className='flex flex-col gap-24px'>
      <div className='text-12px leading-relaxed p-10px rd-8px bg-[rgba(var(--orange-6),0.08)] border border-solid border-[rgba(var(--orange-6),0.3)] text-t-secondary'>
        <div className='font-500 text-t-primary mb-6px'>
          {t('settings.wecom.wsTitle', 'WeCom WebSocket connection')}
        </div>
        <div className='mt-6px'>
          {t(
            'settings.wecom.wsHint',
            'Use the WeCom Intelligent Bot “Long Connection (WebSocket)” mode. No callback URL / domain / public IP required.'
          )}
        </div>
        <div className='mt-4px'>
          <a
            className='text-primary hover:underline cursor-pointer text-12px'
            href={WECOM_DEV_DOCS_URL}
            onClick={(e) => {
              e.preventDefault();
              openExternalUrl(WECOM_DEV_DOCS_URL).catch(console.error);
            }}
          >
            {t('settings.wecom.devDocLink', 'WeCom developer documentation')}
          </a>
        </div>
      </div>

      <PreferenceRow
        label={t('settings.wecom.botId', 'Bot ID')}
        description={t('settings.wecom.botIdDesc', 'Bot ID from WeCom Intelligent Bot (Long Connection mode)')}
        required
      >
        {credentialsLocked ? (
          <Tooltip
            content={t(
              'settings.channels.tokenLocked',
              'Please close the Channel and delete all authorized users before modifying'
            )}
          >
            <span>
              <Input
                value={botId}
                onChange={(value) => {
                  setBotId(value);
                  handleCredentialsChange();
                }}
                onBlur={() => setTouched((prev) => ({ ...prev, botId: true }))}
                placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : ''}
                style={{ width: 260 }}
                status={touched.botId && !botId.trim() && !pluginStatus?.hasToken ? 'error' : undefined}
                disabled={credentialsLocked}
              />
            </span>
          </Tooltip>
        ) : (
          <Input
            value={botId}
            onChange={(value) => {
              setBotId(value);
              handleCredentialsChange();
            }}
            onBlur={() => setTouched((prev) => ({ ...prev, botId: true }))}
            placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : ''}
            style={{ width: 260 }}
            status={touched.botId && !botId.trim() && !pluginStatus?.hasToken ? 'error' : undefined}
            disabled={credentialsLocked}
          />
        )}
      </PreferenceRow>

      <PreferenceRow
        label={t('settings.wecom.secret', 'Secret')}
        description={t('settings.wecom.secretDesc', 'Secret from WeCom Intelligent Bot (Long Connection mode)')}
        required
      >
        {credentialsLocked ? (
          <Tooltip
            content={t(
              'settings.channels.tokenLocked',
              'Please close the Channel and delete all authorized users before modifying'
            )}
          >
            <span>
              <Input.Password
                value={secret}
                onChange={(value) => {
                  setSecret(value);
                  handleCredentialsChange();
                }}
                onBlur={() => setTouched((prev) => ({ ...prev, secret: true }))}
                placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : ''}
                style={{ width: 260 }}
                status={touched.secret && !secret.trim() && !pluginStatus?.hasToken ? 'error' : undefined}
                visibilityToggle
                disabled={credentialsLocked}
              />
            </span>
          </Tooltip>
        ) : (
          <Input.Password
            value={secret}
            onChange={(value) => {
              setSecret(value);
              handleCredentialsChange();
            }}
            onBlur={() => setTouched((prev) => ({ ...prev, secret: true }))}
            placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : ''}
            style={{ width: 260 }}
            status={touched.secret && !secret.trim() && !pluginStatus?.hasToken ? 'error' : undefined}
            visibilityToggle
            disabled={credentialsLocked}
          />
        )}
      </PreferenceRow>

      {!pluginStatus?.connected && (
        <div className='flex justify-end'>
          {pluginStatus?.hasToken && !botId.trim() && !secret.trim() ? (
            <span className='text-12px text-t-tertiary mr-12px self-center'>
              {t('settings.wecom.credentialsSaved', 'Credentials already configured. Enter new values to update.')}
            </span>
          ) : null}
          <Button
            type='primary'
            loading={saveLoading}
            onClick={() => void handleSaveAndEnable()}
            disabled={pluginStatus?.hasToken && !botId.trim() && !secret.trim()}
          >
            {t('settings.wecom.saveAndEnable', 'Save & Enable')}
          </Button>
        </div>
      )}


      {/* Connection Status */}
      {pluginStatus?.enabled && authorizedUsers.length === 0 && (
        <div
          className={`rd-12px p-16px border ${pluginStatus?.connected ? 'bg-green-50 dark:bg-green-900/20 border-green-200 dark:border-green-800' : pluginStatus?.error ? 'bg-red-50 dark:bg-red-900/20 border-red-200 dark:border-red-800' : 'bg-yellow-50 dark:bg-yellow-900/20 border-yellow-200 dark:border-yellow-800'}`}
        >
          <SectionHeader
            title={t('settings.wecom.connectionStatus', 'Connection Status')}
            action={
              <span
                className={`text-12px px-8px py-2px rd-4px ${pluginStatus?.connected ? 'bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300' : pluginStatus?.error ? 'bg-red-100 text-red-700 dark:bg-red-900 dark:text-red-300' : 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900 dark:text-yellow-300'}`}
              >
                {pluginStatus?.connected
                  ? t('settings.wecom.statusConnected', 'Connected')
                  : pluginStatus?.error
                    ? t('settings.wecom.statusError', 'Error')
                    : t('settings.wecom.statusConnecting', 'Connecting...')}
              </span>
            }
          />
          {pluginStatus?.error && (
            <div className='text-14px text-red-600 dark:text-red-400 mb-12px'>{pluginStatus.error}</div>
          )}
          {pluginStatus?.connected && (
            <div className='text-14px text-t-secondary space-y-8px'>
              <p className='m-0 font-500'>{t('settings.channels.nextSteps', 'Next Steps')}:</p>
              <p className='m-0'>
                <strong>1.</strong> {t('settings.wecom.step1', 'Open WeCom and find your bot application')}
              </p>
              <p className='m-0'>
                <strong>2.</strong> {t('settings.wecom.step2', 'Send any message to initiate pairing')}
              </p>
              <p className='m-0'>
                <strong>3.</strong>{' '}
                {t(
                  'settings.wecom.step3',
                  'A pairing request will appear below. Click "Approve" to authorize the user.'
                )}
              </p>
              <p className='m-0'>
                <strong>4.</strong>{' '}
                {t(
                  'settings.wecom.step4',
                  'Once approved, you can start chatting with the AI agent through WeCom!'
                )}
              </p>
            </div>
          )}
          {!pluginStatus?.connected && !pluginStatus?.error && (
            <div className='text-14px text-t-secondary'>
              {t('settings.wecom.waitingConnection', 'Connection is being established. Please wait...')}
            </div>
          )}
        </div>
      )}

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

export default WecomConfigForm;
