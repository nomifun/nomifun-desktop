import type { McpServerId } from '@/common/types/ids';
import type { IMcpServer } from '@/common/config/storage';
import { Button, Dropdown, Menu, Popover, Tooltip } from '@arco-design/web-react';
import { Check, CloseSmall, Info, LoadingOne, Refresh, Write, DeleteFour, SettingOne, Login } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { McpOAuthStatus } from '@/renderer/hooks/mcp/useMcpOAuth';
import FeedbackButton from '@/renderer/components/base/FeedbackButton';
import { iconColors } from '@/renderer/styles/colors';
import { getMcpConfigurationFields, supportsMcpOAuthLogin } from '@/renderer/hooks/mcp/mcpAuthConfig';

interface McpServerHeaderProps {
  server: IMcpServer;
  isTestingConnection: boolean;
  oauthStatus?: McpOAuthStatus;
  isLoggingIn?: boolean;
  onTestConnection: (server: IMcpServer) => void;
  onEditServer: (server: IMcpServer) => void;
  onDeleteServer: (serverId: McpServerId) => void;
  onOAuthLogin?: (server: IMcpServer) => void;
}

const getStatusIcon = (
  last_test_status?: IMcpServer['last_test_status'],
  oauthStatus?: McpOAuthStatus,
  isTestingConnection?: boolean
) => {
  if (isTestingConnection || last_test_status === 'testing' || oauthStatus?.isChecking) {
    return <LoadingOne size='18' fill={iconColors.primary} />;
  }

  if (last_test_status === 'error') {
    return <CloseSmall size='18' fill={iconColors.danger} />;
  }

  if (oauthStatus?.needsLogin) {
    return <span className='text-16px font-bold leading-none text-orange-500'>△</span>;
  }

  if (last_test_status === 'connected') {
    return <Check size='18' fill={iconColors.success} />;
  }

  if (oauthStatus?.isAuthenticated) {
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
  oauthStatus?: McpOAuthStatus,
  isTestingConnection?: boolean,
  t?: (key: string, options?: Record<string, unknown>) => string
) => {
  if (isTestingConnection || last_test_status === 'testing' || oauthStatus?.isChecking) {
    return t?.('settings.mcpTesting') || 'testing';
  }

  if (last_test_status === 'error') {
    return t?.('settings.mcpCheckFailedSimple') || 'Failed';
  }

  if (oauthStatus?.needsLogin) {
    return t?.('settings.mcpNeedsLogin') || 'Login required';
  }

  if (last_test_status === 'connected') {
    return t?.('settings.mcpCheckPassedSimple') || 'Manual check passed';
  }

  if (oauthStatus?.isAuthenticated) {
    return t?.('settings.mcpAuthenticated') || 'Authenticated';
  }

  return t?.('settings.mcpDisconnected') || 'Not tested';
};

const McpServerHeader: React.FC<McpServerHeaderProps> = ({
  server,
  isTestingConnection,
  oauthStatus,
  isLoggingIn,
  onTestConnection,
  onEditServer,
  onDeleteServer,
  onOAuthLogin,
}) => {
  const { t } = useTranslation();

  const configurationFields = getMcpConfigurationFields(server.transport);
  const needsConfiguration = configurationFields.length > 0;
  const oauthCapable = supportsMcpOAuthLogin(server.transport);
  const needsLogin = oauthCapable && oauthStatus?.needsLogin;
  const statusText = needsConfiguration
    ? t('settings.mcpConfigurationRequiredShort', {
        defaultValue: 'Configuration required',
      })
    : getStatusText(server.last_test_status, oauthStatus, isTestingConnection, t);
  const statusIcon = needsConfiguration ? (
    <span className='text-16px font-bold leading-none text-orange-500'>!</span>
  ) : (
    getStatusIcon(server.last_test_status, oauthStatus, isTestingConnection)
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
    <div className='group flex min-w-0 w-full items-center justify-between gap-8px'>
      <div className='flex min-w-0 flex-1 items-center gap-8px'>
        <span className='min-w-0 truncate text-14px leading-24px'>{server.name}</span>
        {statusPopoverContent ? (
          <Popover content={statusPopoverContent} trigger='hover' position='top'>
            <span className='inline-flex size-24px flex-none cursor-default items-center justify-center line-height-0'>
              {statusIcon}
            </span>
          </Popover>
        ) : (
          <Tooltip content={statusText} position='top'>
            <span className='inline-flex size-24px flex-none cursor-default items-center justify-center line-height-0'>
              {statusIcon}
            </span>
          </Tooltip>
        )}
        {isError && <FeedbackButton className='!h-24px !px-6px !py-0 [&_svg]:!pt-0' />}
        {needsConfiguration && !server.builtin && (
          <Button
            size='mini'
            type='outline'
            status='warning'
            icon={<Write size={'14'} />}
            className='!h-24px [&_.arco-btn-icon]:inline-flex [&_.arco-btn-icon]:items-center'
            title={statusText}
            onClick={() => onEditServer(server)}
          >
            {t('settings.mcpConfigure', { defaultValue: 'Configure' })}
          </Button>
        )}
        {!needsConfiguration && needsLogin && onOAuthLogin && (
          <Button
            size='mini'
            type='primary'
            icon={<Login size={'14'} />}
            className='!h-24px [&_.arco-btn-icon]:inline-flex [&_.arco-btn-icon]:items-center'
            title={t('settings.mcpLogin') || 'Login'}
            loading={isLoggingIn}
            onClick={() => onOAuthLogin(server)}
          >
            {t('settings.mcpLogin') || 'Login'}
          </Button>
        )}
        {!needsConfiguration && !needsLogin && (
          <Button
            size='mini'
            icon={<Refresh size={'14'} />}
            className='!size-24px !p-0 [&_.arco-btn-icon]:inline-flex [&_.arco-btn-icon]:items-center'
            title={t('settings.mcpTestConnection')}
            loading={isTestingConnection}
            onClick={() => onTestConnection(server)}
          />
        )}
      </div>
      <div
        className='invisible flex flex-none items-center gap-8px group-hover:visible'
        onClick={(e) => e.stopPropagation()}
      >
        {!server.builtin && (
          <Dropdown
            trigger='hover'
            droplist={
              <Menu>
                <Menu.Item key='edit' onClick={() => onEditServer(server)}>
                  <div className='flex items-center gap-2'>
                    <Write size={'14'} />
                    {t('settings.mcpEditServer')}
                  </div>
                </Menu.Item>
                <Menu.Item key='delete' onClick={() => onDeleteServer(server.mcp_server_id)}>
                  <div className='flex items-center gap-2 text-red-500'>
                    <DeleteFour size={'14'} />
                    {t('settings.mcpDeleteServer')}
                  </div>
                </Menu.Item>
              </Menu>
            }
          >
            <Button
              size='mini'
              icon={<SettingOne size={'14'} />}
              className='!size-24px !p-0 [&_.arco-btn-icon]:inline-flex [&_.arco-btn-icon]:items-center'
            />
          </Dropdown>
        )}
      </div>
    </div>
  );
};

export default McpServerHeader;
