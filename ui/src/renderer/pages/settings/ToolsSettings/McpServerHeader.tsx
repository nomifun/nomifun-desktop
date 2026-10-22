import type { McpServerId } from '@/common/types/ids';
import type { IMcpServer } from '@/common/config/storage';
import { Button, Dropdown, Menu, Popover, Switch, Tooltip } from '@arco-design/web-react';
import { Check, CloseSmall, Info, LoadingOne, Refresh, Write, DeleteFour, Setting, Login } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { McpOAuthStatus } from '@/renderer/hooks/mcp/useMcpOAuth';
import type { McpOAuthAttempt } from '@/common/types/mcpAuth';
import FeedbackButton from '@/renderer/components/base/FeedbackButton';
import { iconColors } from '@/renderer/styles/colors';
import { getMcpConfigurationFields, supportsMcpOAuthLogin } from '@/renderer/hooks/mcp/mcpAuthConfig';
import { useMcpDisplayName } from '@/renderer/services/mcp/useMcpDisplayName';
import styles from './McpServerList.module.css';

interface McpServerHeaderProps {
  server: IMcpServer;
  isTestingConnection: boolean;
  oauthStatus?: McpOAuthStatus;
  oauthAttempt?: McpOAuthAttempt;
  isLoggingIn?: boolean;
  isTogglingEnabled?: boolean;
  onTestConnection: (server: IMcpServer) => void;
  onEditServer: (server: IMcpServer) => void;
  onDeleteServer: (serverId: McpServerId) => void;
  onOAuthLogin?: (server: IMcpServer) => void;
  onOAuthCancel?: (server: IMcpServer) => void;
  onOAuthLogout?: (server: IMcpServer) => void;
  onOAuthConfigure?: (server: IMcpServer) => void;
  onOAuthContinue?: (server: IMcpServer) => void;
  onToggleEnabled: (server: IMcpServer) => void;
}

const getStatusIcon = (
  last_test_status?: IMcpServer['last_test_status'],
  isTestingConnection?: boolean
) => {
  if (isTestingConnection || last_test_status === 'testing') {
    return <LoadingOne size='18' fill={iconColors.primary} />;
  }

  if (last_test_status === 'error') {
    return <CloseSmall size='18' fill={iconColors.danger} />;
  }

  if (last_test_status === 'connected') {
    return <Check size='18' fill={iconColors.success} />;
  }

  return <Info theme='outline' size='18' fill={iconColors.secondary} />;
};

const formatStatusTimestamp = (timestamp?: number): string | null => {
  if (!timestamp) {
    return null;
  }

  return new Date(timestamp).toLocaleString();
};

const getStatusPopoverContent = (
  server: IMcpServer,
  t?: (key: string, options?: Record<string, unknown>) => string
) => {
  if (server.last_test_status !== 'error' && server.last_test_status !== 'connected') {
    return null;
  }

  if (server.last_test_status === 'connected') {
    const checkedAt = formatStatusTimestamp(server.last_connected || server.updated_at);
    return (
      <div className='max-w-300px space-y-2 text-13px leading-20px'>
        <div className='font-medium text-t-primary'>
          {t?.('settings.mcpCheckPassedSummary') || 'Manual check passed'}
        </div>
        {checkedAt ? (
          <div className='text-12px leading-18px text-t-secondary'>{`${t?.('settings.mcpCheckedAtLabel') || 'Checked at:'} ${checkedAt}`}</div>
        ) : null}
        <div className='text-12px leading-18px text-t-secondary opacity-80'>
          {t?.('settings.mcpCheckPurposeHint') ||
            'Used to verify whether the MCP configuration is available. It does not represent the real-time status in the current conversation.'}
        </div>
      </div>
    );
  }

  const checkedAt = formatStatusTimestamp(server.updated_at);

  const reasonText =
    t?.('settings.mcpInlineFailureHint') ||
    'The last availability check failed. The cause may be a missing runtime, a prerequisite service, networking, authentication, or protocol compatibility—not necessarily the JSON format. Test again to see the current diagnostic.';

  return (
    <div className='max-w-300px space-y-2 text-13px leading-20px'>
      <div className='font-medium text-t-primary'>{t?.('settings.mcpCheckFailedSummary') || 'Manual check failed'}</div>
      <div className='text-t-primary'>{reasonText}</div>
      {checkedAt ? (
        <div className='text-12px leading-18px text-t-secondary'>{`${t?.('settings.mcpCheckedAtLabel') || 'Checked at:'} ${checkedAt}`}</div>
      ) : null}
    </div>
  );
};

const getStatusText = (
  last_test_status?: IMcpServer['last_test_status'],
  isTestingConnection?: boolean,
  t?: (key: string, options?: Record<string, unknown>) => string
) => {
  if (isTestingConnection || last_test_status === 'testing') {
    return t?.('settings.mcpTesting') || 'testing';
  }

  if (last_test_status === 'error') {
    return t?.('settings.mcpCheckFailedSimple') || 'Failed';
  }

  if (last_test_status === 'connected') {
    return t?.('settings.mcpCheckPassedSimple') || 'Manual check passed';
  }

  return t?.('settings.mcpDisconnected') || 'Not tested';
};

const McpServerHeader: React.FC<McpServerHeaderProps> = ({
  server,
  isTestingConnection,
  oauthStatus,
  oauthAttempt,
  isLoggingIn,
  isTogglingEnabled,
  onTestConnection,
  onEditServer,
  onDeleteServer,
  onOAuthLogin,
  onOAuthCancel,
  onOAuthLogout,
  onOAuthConfigure,
  onOAuthContinue,
  onToggleEnabled,
}) => {
  const { t } = useTranslation();
  const getDisplayName = useMcpDisplayName();
  const displayName = getDisplayName(server);

  const configurationFields = getMcpConfigurationFields(server.transport);
  const needsConfiguration = configurationFields.length > 0;
  const oauthCapable = supportsMcpOAuthLogin(server.transport);
  const auth = oauthStatus?.auth;
  const clientRequired = auth?.state === 'configuration_required';
  const needsLogin = oauthCapable && auth && ['required', 'reauthorization_required', 'configuration_required', 'failed'].includes(auth.state);
  const statusText = needsConfiguration
    ? t('settings.mcpConfigurationRequiredShort', {
        defaultValue: 'Configuration required',
      })
    : getStatusText(server.last_test_status, isTestingConnection, t);
  const statusIcon = needsConfiguration ? (
    <span className='text-16px font-bold leading-none text-orange-500'>!</span>
  ) : (
    getStatusIcon(server.last_test_status, isTestingConnection)
  );
  const statusPopoverContent = needsConfiguration ? (
    <div className='max-w-300px space-y-2 text-13px leading-20px'>
      <div className='font-medium text-t-primary'>
        {t('settings.mcpConfigurationRequiredShort', {
          defaultValue: 'Configuration required',
        })}
      </div>
      <div className='text-12px leading-18px text-t-secondary'>
        {t('settings.mcpConfigurationFieldsHint', {
          fields: configurationFields.join(', '),
          defaultValue: `Complete these fields before testing: ${configurationFields.join(', ')}`,
        })}
      </div>
    </div>
  ) : (
    getStatusPopoverContent(server, t)
  );

  const isError = !needsConfiguration && server.last_test_status === 'error';

  return (
    <div className={styles.header}>
      <div
        className={styles.summary}
        onClick={(event) => {
          if ((event.target as Element).closest('button')) event.stopPropagation();
        }}
        onKeyDown={(event) => {
          if ((event.target as Element).closest('button')) event.stopPropagation();
        }}
      >
        <span className={styles.name} title={displayName}>{displayName}</span>
        {statusPopoverContent ? (
          <Popover content={statusPopoverContent} trigger='hover' position='top'>
            <span role='img' aria-label={statusText} className={styles.status}>
              {statusIcon}
              <span aria-hidden='true'>{statusText}</span>
            </span>
          </Popover>
        ) : (
          <Tooltip content={statusText} position='top'>
            <span role='img' aria-label={statusText} className={styles.status}>
              {statusIcon}
              <span aria-hidden='true'>{statusText}</span>
            </span>
          </Tooltip>
        )}
        {isError && <FeedbackButton className='!h-24px !px-6px !py-0 [&_svg]:!pt-0' />}
        {oauthCapable && auth?.state === 'authorized' && !isLoggingIn && (
          <span data-testid='mcp-auth-authorized' className='flex-none text-12px text-t-secondary'>
            {t('settings.mcpAuthenticated')}
          </span>
        )}
        {isLoggingIn && (
          <span className='flex-none text-12px text-t-secondary'>
            {t(oauthAttempt?.status === 'exchanging' ? 'settings.mcpOAuthExchanging' : 'settings.mcpOAuthWaiting')}
          </span>
        )}
        {needsConfiguration && !server.builtin && (
          <Button
            size='mini'
            type='outline'
            status='warning'
            icon={<Write size={'14'} />}
            className={styles.stateAction}
            title={statusText}
            onClick={() => onEditServer(server)}
          >
            {t('settings.mcpConfigure', { defaultValue: 'Configure' })}
          </Button>
        )}
        {!needsConfiguration && needsLogin && !isLoggingIn && onOAuthLogin && (
          <Button
            size='mini'
            type='primary'
            icon={<Login size={'14'} />}
            className={styles.stateAction}
            title={t(clientRequired ? 'settings.mcpOAuthConfigureClient' : 'settings.mcpLogin')}
            disabled={isTogglingEnabled}
            onClick={() => clientRequired && onOAuthConfigure ? onOAuthConfigure(server) : onOAuthLogin(server)}
          >
            {t(clientRequired ? 'settings.mcpOAuthConfigureClient' : 'settings.mcpLogin')}
          </Button>
        )}
        {isLoggingIn && oauthAttempt?.authorization_url && onOAuthContinue && (
          <Button size='mini' type='text' className={styles.stateAction} onClick={() => onOAuthContinue(server)}>{t('settings.mcpOAuthContinue')}</Button>
        )}
        {isLoggingIn && onOAuthCancel && (
          <Button size='mini' type='text' className={styles.stateAction} onClick={() => onOAuthCancel(server)}>{t('common.cancel')}</Button>
        )}
      </div>
      <div
        className={styles.controls}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.stopPropagation()}
      >
        {!needsConfiguration && !isLoggingIn && (
          <Button
            size='mini'
            type='text'
            icon={<Refresh size={'16'} />}
            className={styles.testButton}
            title={t('settings.mcpTestConnection')}
            aria-label={t('settings.mcpTestConnection')}
            loading={isTestingConnection}
            disabled={isTogglingEnabled}
            onClick={() => onTestConnection(server)}
          >
            {t('settings.mcpTestConnection')}
          </Button>
        )}
        {(!server.builtin || oauthCapable) && (
          <Dropdown
            trigger='click'
            droplist={
              <Menu>
                {oauthCapable && onOAuthConfigure && <Menu.Item key='oauth-config' disabled={isLoggingIn} onClick={() => onOAuthConfigure(server)}>
                  {t('settings.mcpOAuthConfigureClient')}
                </Menu.Item>}
                {oauthCapable && onOAuthLogout && <Menu.Item key='oauth-logout' disabled={isLoggingIn} onClick={() => onOAuthLogout(server)}>
                  {t('settings.mcpOAuthLogout')}
                </Menu.Item>}
                {!server.builtin && <Menu.Item key='edit' onClick={() => onEditServer(server)}>
                  <div className='flex items-center gap-2'>
                    <Write size={'14'} />
                    {t('settings.mcpEditServer')}
                  </div>
                </Menu.Item>}
                {!server.builtin && <Menu.Item key='delete' onClick={() => onDeleteServer(server.mcp_server_id)}>
                  <div className='flex items-center gap-2 text-red-500'>
                    <DeleteFour size={'14'} />
                    {t('settings.mcpDeleteServer')}
                  </div>
                </Menu.Item>}
              </Menu>
            }
          >
            <Button
              size='mini'
              type='text'
              icon={<Setting size={'18'} />}
              className={styles.iconButton}
              title={t('settings.mcpEditServer')}
              aria-label={t('settings.mcpEditServer')}
            />
          </Dropdown>
        )}
        <span className='sr-only'>
          {t(server.enabled ? 'settings.mcpEnabled' : 'settings.mcpDisabled')}
        </span>
        <Switch
          size='small'
          checked={server.enabled}
          loading={isTogglingEnabled}
          disabled={isTogglingEnabled || isTestingConnection || server.last_test_status === 'testing' || isLoggingIn}
          aria-busy={Boolean(isTogglingEnabled)}
          aria-label={t(server.enabled ? 'settings.mcpDisableServer' : 'settings.mcpEnableServer', { name: displayName })}
          onChange={() => onToggleEnabled(server)}
        />
      </div>
    </div>
  );
};

export default McpServerHeader;
