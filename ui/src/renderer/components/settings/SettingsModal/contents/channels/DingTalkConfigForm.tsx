/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IChannelPluginStatus } from '@/common/types/channel/channel';
import { channel } from '@/common/adapter/ipcBridge';
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

interface DingTalkConfigFormProps {
  pluginStatus: IChannelPluginStatus | null;
  /** 多机器人模式下寻址的渠道行；缺省 = 全局设置页 legacy 单行行为。 */
  channelTarget?: ChannelTarget;
  onStatusChange: (status: IChannelPluginStatus | null) => void;
}

const DINGTALK_DEV_DOCS_URL = 'https://github.com/nomifun/nomifun-app/wiki/DingTalk-Bot-Setup-Guide';

const DingTalkConfigForm: React.FC<DingTalkConfigFormProps> = ({
  pluginStatus,
  channelTarget,
  onStatusChange,
}) => {
  const { t } = useTranslation();

  // DingTalk credentials
  const [clientId, setClientId] = useState('');
  const [clientSecret, setClientSecret] = useState('');

  const [testLoading, setTestLoading] = useState(false);
  const [_credentialsTested, setCredentialsTested] = useState(false);
  const [touched, setTouched] = useState({ clientId: false, clientSecret: false });

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
  } = useChannelPairing('dingtalk', channelTarget);

  // Test DingTalk connection
  const handleTestConnection = async () => {
    setTouched({ clientId: true, clientSecret: true });

    if (!clientId.trim() || !clientSecret.trim()) {
      Message.warning(t('settings.dingtalk.credentialsRequired', 'Please enter Client ID and Client Secret'));
      return;
    }

    setTestLoading(true);
    setCredentialsTested(false);
    try {
      // testPlugin returns { success, botUsername?, error? } directly
      const result = await channel.testPlugin.invoke({
        plugin_type: 'dingtalk',
        token: clientId.trim(),
        extra_config: {
          app_secret: clientSecret.trim(),
        },
      });

      if (result.success) {
        setCredentialsTested(true);
        Message.success(t('settings.dingtalk.connectionSuccess', 'Connected to DingTalk API!'));
        await handleAutoEnable();
      } else {
        setCredentialsTested(false);
        Message.error(result.error || t('settings.dingtalk.connectionFailed', 'Connection failed'));
      }
    } catch (error: any) {
      setCredentialsTested(false);
      Message.error(error.message || t('settings.dingtalk.connectionFailed', 'Connection failed'));
    } finally {
      setTestLoading(false);
    }
  };

  // Auto-enable plugin after successful test
  const handleAutoEnable = async () => {
    try {
      const config = {
        credentials: {
          client_id: clientId.trim(),
          client_secret: clientSecret.trim(),
        },
      };
      const result = await channel.enablePlugin.invoke(buildEnablePluginRequest('dingtalk', channelTarget, config));
      if (!result.success) {
        throw new Error(result.error || t('nomi.settings.remoteEnableFailed', { defaultValue: 'Failed to enable channel' }));
      }

      Message.success(t('settings.dingtalk.pluginEnabled', 'DingTalk bot enabled'));
      const plugins = await channel.getPluginStatus.invoke();
      if (plugins) {
        // Multi-plugin model: resolve by the backend-returned business UUID, or by owner scope after create.
        const dingtalkPlugin = findEnabledChannelStatus(plugins, {
          platform: 'dingtalk',
          enabledPluginId: result.plugin_id,
          companionId: channelTarget?.companionId,
          ownerDomain: channelTarget?.ownerDomain,
        });
        onStatusChange(dingtalkPlugin || null);
      }
    } catch (error: unknown) {
      console.error('[DingTalkConfig] Auto-enable failed:', error);
      Message.error(
        (error instanceof Error ? error.message : String(error)) ||
          t('settings.dingtalk.enableFailed', 'Failed to enable DingTalk plugin')
      );
    }
  };

  // Reset credentials tested state when credentials change
  const handleCredentialsChange = () => {
    setCredentialsTested(false);
  };

  // Row-scoped credential lock — see LarkConfigForm for the rationale (was a
  // global per-platform `authorizedUsers.length > 0`, which froze a second
  // companion's create form). Lock only when THIS bot row is live.
  const credentialsLocked = !!pluginStatus?.connected;

  return (
    <div className='flex flex-col gap-24px'>
      {/* Client ID */}
      <PreferenceRow
        label={t('settings.dingtalk.clientId', 'Client ID')}
        description={
          <span>
            <a
              className='text-primary hover:underline cursor-pointer text-12px'
              href={DINGTALK_DEV_DOCS_URL}
              onClick={(e) => {
                e.preventDefault();
                openExternalUrl(DINGTALK_DEV_DOCS_URL).catch(console.error);
              }}
            >
              {t('settings.dingtalk.devConsoleLink', 'DingTalk Open Platform')}
            </a>{' '}
            {t('settings.dingtalk.clientIdDescSuffix', 'to get your Client ID')}
          </span>
        }
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
                value={clientId}
                onChange={(value) => {
                  setClientId(value);
                  handleCredentialsChange();
                }}
                onBlur={() => setTouched((prev) => ({ ...prev, clientId: true }))}
                placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : 'dingxxxxxxxxxx'}
                style={{ width: 240 }}
                status={touched.clientId && !clientId.trim() && !pluginStatus?.hasToken ? 'error' : undefined}
                disabled={credentialsLocked}
              />
            </span>
          </Tooltip>
        ) : (
          <Input
            value={clientId}
            onChange={(value) => {
              setClientId(value);
              handleCredentialsChange();
            }}
            onBlur={() => setTouched((prev) => ({ ...prev, clientId: true }))}
            placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : 'dingxxxxxxxxxx'}
            style={{ width: 240 }}
            status={touched.clientId && !clientId.trim() && !pluginStatus?.hasToken ? 'error' : undefined}
            disabled={credentialsLocked}
          />
        )}
      </PreferenceRow>

      {/* Client Secret */}
      <PreferenceRow
        label={t('settings.dingtalk.clientSecret', 'Client Secret')}
        description={
          <span>
            <a
              className='text-primary hover:underline cursor-pointer text-12px'
              href={DINGTALK_DEV_DOCS_URL}
              onClick={(e) => {
                e.preventDefault();
                openExternalUrl(DINGTALK_DEV_DOCS_URL).catch(console.error);
              }}
            >
              {t('settings.dingtalk.devConsoleLink', 'DingTalk Open Platform')}
            </a>{' '}
            {t('settings.dingtalk.clientSecretDescSuffix', 'to get Client Secret')}
          </span>
        }
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
                value={clientSecret}
                onChange={(value) => {
                  setClientSecret(value);
                  handleCredentialsChange();
                }}
                onBlur={() => setTouched((prev) => ({ ...prev, clientSecret: true }))}
                placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : 'xxxxxxxxxxxxxxxxxx'}
                style={{ width: 240 }}
                status={touched.clientSecret && !clientSecret.trim() && !pluginStatus?.hasToken ? 'error' : undefined}
                visibilityToggle
                disabled={credentialsLocked}
              />
            </span>
          </Tooltip>
        ) : (
          <Input.Password
            value={clientSecret}
            onChange={(value) => {
              setClientSecret(value);
              handleCredentialsChange();
            }}
            onBlur={() => setTouched((prev) => ({ ...prev, clientSecret: true }))}
            placeholder={pluginStatus?.hasToken ? '••••••••••••••••' : 'xxxxxxxxxxxxxxxxxx'}
            style={{ width: 240 }}
            status={touched.clientSecret && !clientSecret.trim() && !pluginStatus?.hasToken ? 'error' : undefined}
            visibilityToggle
            disabled={credentialsLocked}
          />
        )}
      </PreferenceRow>

      {/* Test Connection Button */}
      {!pluginStatus?.connected && (
        <div className='flex justify-end'>
          {pluginStatus?.hasToken && !clientId.trim() && !clientSecret.trim() ? (
            <span className='text-12px text-t-tertiary mr-12px self-center'>
              {t('settings.dingtalk.credentialsSaved', 'Credentials already configured. Enter new values to update.')}
            </span>
          ) : null}
          <Button
            type='primary'
            loading={testLoading}
            onClick={handleTestConnection}
            disabled={pluginStatus?.hasToken && !clientId.trim() && !clientSecret.trim()}
          >
            {t('settings.dingtalk.testAndConnect', 'Test & Connect')}
          </Button>
        </div>
      )}


      {/* Connection Status */}
      {pluginStatus?.enabled && authorizedUsers.length === 0 && (
        <div
          className={`rd-12px p-16px border ${pluginStatus?.connected ? 'bg-green-50 dark:bg-green-900/20 border-green-200 dark:border-green-800' : pluginStatus?.error ? 'bg-red-50 dark:bg-red-900/20 border-red-200 dark:border-red-800' : 'bg-yellow-50 dark:bg-yellow-900/20 border-yellow-200 dark:border-yellow-800'}`}
        >
          <SectionHeader
            title={t('settings.dingtalk.connectionStatus', 'Connection Status')}
            action={
              <span
                className={`text-12px px-8px py-2px rd-4px ${pluginStatus?.connected ? 'bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300' : pluginStatus?.error ? 'bg-red-100 text-red-700 dark:bg-red-900 dark:text-red-300' : 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900 dark:text-yellow-300'}`}
              >
                {pluginStatus?.connected
                  ? t('settings.dingtalk.statusConnected', 'Connected')
                  : pluginStatus?.error
                    ? t('settings.dingtalk.statusError', 'Error')
                    : t('settings.dingtalk.statusConnecting', 'Connecting...')}
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
                <strong>1.</strong> {t('settings.dingtalk.step1', 'Open DingTalk and find your bot application')}
              </p>
              <p className='m-0'>
                <strong>2.</strong> {t('settings.dingtalk.step2', 'Send any message to initiate pairing')}
              </p>
              <p className='m-0'>
                <strong>3.</strong>{' '}
                {t(
                  'settings.dingtalk.step3',
                  'A pairing request will appear below. Click "Approve" to authorize the user.'
                )}
              </p>
              <p className='m-0'>
                <strong>4.</strong>{' '}
                {t(
                  'settings.dingtalk.step4',
                  'Once approved, you can start chatting with the AI agent through DingTalk!'
                )}
              </p>
            </div>
          )}
          {!pluginStatus?.connected && !pluginStatus?.error && (
            <div className='text-14px text-t-secondary'>
              {t('settings.dingtalk.waitingConnection', 'Connection is being established. Please wait...')}
            </div>
          )}
        </div>
      )}

      {/* Pending Pairings */}
      {pluginStatus?.enabled && (
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

export default DingTalkConfigForm;
