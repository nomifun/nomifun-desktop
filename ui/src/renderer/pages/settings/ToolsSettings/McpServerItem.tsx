import { Collapse } from '@arco-design/web-react';
import React, { useEffect, useRef } from 'react';
import type { IMcpServer } from '@/common/config/storage';
import type { McpServerId } from '@/common/types/ids';
import McpServerHeader from './McpServerHeader';
import McpServerToolsList from './McpServerToolsList';
import type { McpOAuthStatus } from '@/renderer/hooks/mcp/useMcpOAuth';
import type { McpOAuthAttempt } from '@/common/types/mcpAuth';
import styles from './McpServerList.module.css';

interface McpServerItemProps {
  server: IMcpServer;
  expanded: boolean;
  highlighted?: boolean;
  isTestingConnection: boolean;
  oauthStatus?: McpOAuthStatus;
  oauthAttempt?: McpOAuthAttempt;
  isLoggingIn?: boolean;
  isTogglingEnabled?: boolean;
  onToggleExpanded: () => void;
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
  expanded,
  highlighted = false,
  isTestingConnection,
  oauthStatus,
  oauthAttempt,
  isLoggingIn,
  isTogglingEnabled,
  onToggleExpanded,
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
  const rowRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (highlighted) rowRef.current?.scrollIntoView?.({ block: 'nearest', behavior: 'smooth' });
  }, [highlighted]);
  const diagnostic = oauthStatus?.diagnostic ?? oauthAttempt?.diagnostic ?? oauthStatus?.auth?.diagnostic;
  return (
    <div ref={rowRef} className={highlighted ? styles.highlighted : undefined} data-mcp-server-id={server.mcp_server_id}>
    <Collapse
      key={server.mcp_server_id}
      activeKey={expanded ? ['1'] : []}
      onChange={onToggleExpanded}
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
    </div>
  );
};

export default McpServerItem;
