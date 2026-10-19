import { Collapse } from '@arco-design/web-react';
import React from 'react';
import type { IMcpServer } from '@/common/config/storage';
import type { McpServerId } from '@/common/types/ids';
import McpServerHeader from './McpServerHeader';
import McpServerToolsList from './McpServerToolsList';
import type { McpOAuthStatus } from '@/renderer/hooks/mcp/useMcpOAuth';
import type { McpOAuthAttempt } from '@/common/types/mcpAuth';
import styles from './McpServerList.module.css';

interface McpServerItemProps {
  server: IMcpServer;
  isCollapsed: boolean;
  isTestingConnection: boolean;
  oauthStatus?: McpOAuthStatus;
  oauthAttempt?: McpOAuthAttempt;
  isLoggingIn?: boolean;
  isTogglingEnabled?: boolean;
  onToggleCollapse: () => void;
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

const McpServerItem: React.FC<McpServerItemProps> = ({
  server,
  isCollapsed,
  isTestingConnection,
  oauthStatus,
  oauthAttempt,
  isLoggingIn,
  isTogglingEnabled,
  onToggleCollapse,
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
  const diagnostic = oauthStatus?.diagnostic ?? oauthAttempt?.diagnostic ?? oauthStatus?.auth?.diagnostic;
  return (
    <Collapse
      key={server.mcp_server_id}
      activeKey={isCollapsed ? ['1'] : []}
      onChange={onToggleCollapse}
      className={styles.item}
    >
      <Collapse.Item
        header={
          <McpServerHeader
            server={server}
            isTestingConnection={isTestingConnection}
            oauthStatus={oauthStatus}
            oauthAttempt={oauthAttempt}
            isLoggingIn={isLoggingIn}
            isTogglingEnabled={isTogglingEnabled}
            onTestConnection={onTestConnection}
            onEditServer={onEditServer}
            onDeleteServer={onDeleteServer}
            onOAuthLogin={onOAuthLogin}
            onOAuthCancel={onOAuthCancel}
            onOAuthLogout={onOAuthLogout}
            onOAuthConfigure={onOAuthConfigure}
            onOAuthContinue={onOAuthContinue}
            onToggleEnabled={onToggleEnabled}
          />
        }
        name='1'
      >
        {(diagnostic || oauthStatus?.error) && <div aria-live='polite' className='mb-8px break-words text-12px text-t-secondary'>
          <div>{diagnostic?.message ?? oauthStatus?.error}</div>
          {diagnostic && <div className='mt-4px font-mono'>{diagnostic.code} · {diagnostic.stage} · {diagnostic.trace_id}</div>}
        </div>}
        <McpServerToolsList server={server} />
      </Collapse.Item>
    </Collapse>
  );
};

export default McpServerItem;
