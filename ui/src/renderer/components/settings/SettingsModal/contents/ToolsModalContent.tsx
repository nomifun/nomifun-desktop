/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMcpServer } from '@/common/config/storage';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { getAgents } from '@/renderer/hooks/agent/useAgents';
import { useMcpConnection,useMcpModal,useMcpOAuth,useMcpServerCRUD } from '@/renderer/hooks/mcp';
import { mcpServerUiKey } from '@/renderer/hooks/mcp/mcpUiKey';
import { supportsMcpOAuthLogin } from '@/renderer/hooks/mcp/mcpAuthConfig';
import AddMcpServerModal from '@/renderer/pages/settings/components/AddMcpServerModal';
import { ENHANCED_TOOLS_SURFACE_CLASS } from '@/renderer/pages/settings/enhancedToolsLayout';
import McpServerItem from '@/renderer/pages/settings/ToolsSettings/McpServerItem';
import { Button,Dropdown,Menu,Message,Modal,Pagination } from '@arco-design/web-react';
import { Down,Plus } from '@icon-park/react';
import React,{ useCallback,useEffect,useMemo,useRef,useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

type MessageInstance = Required<ReturnType<typeof Message.useMessage>[0]>;
const MCP_SERVER_PAGE_SIZE = 10;

const ModalMcpManagementSection: React.FC<{
  message: MessageInstance;
  mcpServers: IMcpServer[];
  setMcpServers: React.Dispatch<React.SetStateAction<IMcpServer[]>>;
  saveMcpServers: (serversOrUpdater: IMcpServer[] | ((prev: IMcpServer[]) => IMcpServer[])) => Promise<void>;
  headerActionHost: HTMLDivElement | null;
}> = ({ message, mcpServers, setMcpServers, saveMcpServers, headerActionHost }) => {
  const { t } = useTranslation();
  const { oauthStatus, loggingIn, checkOAuthStatus, markLoginRequired, clearLoginRequired, login } = useMcpOAuth();
  const visibleMcpServers = useMemo(() => mcpServers, [mcpServers]);
  const [currentPage, setCurrentPage] = useState(1);
  const previousServerCount = useRef(visibleMcpServers.length);
  const totalPages = Math.max(1, Math.ceil(visibleMcpServers.length / MCP_SERVER_PAGE_SIZE));
  const effectivePage = Math.min(currentPage, totalPages);
  const paginatedMcpServers = useMemo(() => {
    const start = (effectivePage - 1) * MCP_SERVER_PAGE_SIZE;
    return visibleMcpServers.slice(start, start + MCP_SERVER_PAGE_SIZE);
  }, [effectivePage, visibleMcpServers]);

  useEffect(() => {
    const serverWasAdded = visibleMcpServers.length > previousServerCount.current;
    setCurrentPage((page) => (serverWasAdded ? totalPages : Math.min(page, totalPages)));
    previousServerCount.current = visibleMcpServers.length;
  }, [totalPages, visibleMcpServers.length]);

  const handleAuthRequired = useCallback(
    (server: IMcpServer) => {
      if (supportsMcpOAuthLogin(server.transport)) {
        markLoginRequired(server.mcp_server_id);
      }
    },
    [markLoginRequired]
  );
  const handleAuthResolved = useCallback(
    (server: IMcpServer) => {
      clearLoginRequired(server.mcp_server_id);
    },
    [clearLoginRequired]
  );

  const { testingServers, handleTestMcpConnection, handleTestMcpConnections } = useMcpConnection(
    setMcpServers,
    handleAuthRequired,
    handleAuthResolved
  );
  const {
    showMcpModal,
    editingMcpServer,
    deleteConfirmVisible,
    serverToDelete,
    mcpCollapseKey,
    showAddMcpModal,
    showEditMcpModal,
    hideMcpModal,
    showDeleteConfirm,
    hideDeleteConfirm,
    toggleServerCollapse,
  } = useMcpModal();
  const { handleAddMcpServer, handleBatchImportMcpServers, handleEditMcpServer, handleDeleteMcpServer } =
    useMcpServerCRUD(saveMcpServers);

  const handleOAuthLogin = useCallback(
    async (server: IMcpServer) => {
      const result = await login(server);

      if (result.success) {
        message.success(`${server.name}: ${t('settings.mcpOAuthLoginSuccess') || 'Login successful'}`);
        void handleTestMcpConnection(server);
      } else {
        message.error(`${server.name}: ${result.error || t('settings.mcpOAuthLoginFailed') || 'Login failed'}`);
      }
    },
    [login, message, t, handleTestMcpConnection]
  );

  const wrappedHandleAddMcpServer = useCallback(
    async (serverData: Omit<IMcpServer, 'mcp_server_id' | 'created_at' | 'updated_at'>) => {
      const addedServer = await handleAddMcpServer(serverData);
      if (addedServer) {
        void handleTestMcpConnection(addedServer, { notify: false });
      }
    },
    [handleAddMcpServer, handleTestMcpConnection]
  );

  const wrappedHandleEditMcpServer = useCallback(
    async (serverToEdit: IMcpServer | undefined, serverData: Omit<IMcpServer, 'mcp_server_id' | 'created_at' | 'updated_at'>) => {
      const updatedServer = await handleEditMcpServer(serverToEdit, serverData);
      if (updatedServer) {
        void handleTestMcpConnection(updatedServer, { notify: false });
      }
    },
    [handleEditMcpServer, handleTestMcpConnection]
  );

  const wrappedHandleBatchImportMcpServers = useCallback(
    async (serversData: Omit<IMcpServer, 'mcp_server_id' | 'created_at' | 'updated_at'>[]) => {
      const addedServers = await handleBatchImportMcpServers(serversData);
      if (addedServers && addedServers.length > 0) {
        await handleTestMcpConnections(addedServers, { concurrency: 4, notify: false });
      }
      return addedServers;
    },
    [handleBatchImportMcpServers, handleTestMcpConnections]
  );

  const [detectedAgents, setDetectedAgents] = useState<Array<{ backend: string; name: string }>>([]);
  const [importMode, setImportMode] = useState<'json' | 'oneclick'>('json');

  useEffect(() => {
    const loadAgents = async () => {
      try {
        const agents = await getAgents();
        setDetectedAgents(
          agents.map((agent) => ({
            backend: agent.backend ?? '',
            name: agent.name,
          }))
        );
      } catch (error) {
        console.error('Failed to load agents:', error);
      }
    };
    void loadAgents();
  }, []);

  useEffect(() => {
    const httpServers = mcpServers.filter((server) => supportsMcpOAuthLogin(server.transport));
    if (httpServers.length > 0) {
      httpServers.forEach((server) => {
        void checkOAuthStatus(server);
      });
    }
  }, [mcpServers, checkOAuthStatus]);

  const handleConfirmDelete = useCallback(async () => {
    if (!serverToDelete) return;
    hideDeleteConfirm();
    await handleDeleteMcpServer(serverToDelete);
  }, [serverToDelete, hideDeleteConfirm, handleDeleteMcpServer]);

  const renderAddButton = () => {
    if (detectedAgents.length > 0) {
      return (
        <Dropdown
          trigger='click'
          droplist={
            <Menu>
              <Menu.Item
                key='json'
                onClick={(e) => {
                  e.stopPropagation();
                  setImportMode('json');
                  showAddMcpModal();
                }}
              >
                {t('settings.mcpImportFromJSON')}
              </Menu.Item>
              <Menu.Item
                key='oneclick'
                onClick={(e) => {
                  e.stopPropagation();
                  setImportMode('oneclick');
                  showAddMcpModal();
                }}
              >
                {t('settings.mcpOneKeyImport')}
              </Menu.Item>
            </Menu>
          }
        >
          <Button type='outline' icon={<Plus size={'16'} />} shape='round' onClick={(e) => e.stopPropagation()}>
            {t('settings.mcpAddServer')} <Down size='12' />
          </Button>
        </Dropdown>
      );
    }

    return (
      <Button
        type='outline'
        icon={<Plus size={'16'} />}
        shape='round'
        onClick={() => {
          setImportMode('json');
          showAddMcpModal();
        }}
      >
        {t('settings.mcpAddServer')}
      </Button>
    );
  };

  return (
    <div className='flex min-h-0 flex-col gap-10px'>
      {headerActionHost && createPortal(renderAddButton(), headerActionHost)}

      <div className='flex-1 min-h-0'>
        {visibleMcpServers.length === 0 ? (
          <div className='py-20px text-center text-t-secondary text-14px border border-dashed border-arco-2 rd-12px'>
            {t('settings.mcpNoServersFound')}
          </div>
        ) : (
          <NomiScrollArea className='max-h-360px max-h-none' disableOverflow>
            <div className='space-y-10px'>
              {paginatedMcpServers.map((server) => {
                const uiKey = mcpServerUiKey(server.mcp_server_id);
                return (
                  <McpServerItem
                    key={server.mcp_server_id}
                    server={server}
                    isCollapsed={mcpCollapseKey[uiKey] || false}
                    isTestingConnection={testingServers[server.mcp_server_id] || false}
                    oauthStatus={oauthStatus[server.mcp_server_id]}
                    isLoggingIn={loggingIn[server.mcp_server_id]}
                    onToggleCollapse={() => toggleServerCollapse(uiKey)}
                    onTestConnection={handleTestMcpConnection}
                    onEditServer={showEditMcpModal}
                    onDeleteServer={showDeleteConfirm}
                    onOAuthLogin={handleOAuthLogin}
                  />
                );
              })}
            </div>
          </NomiScrollArea>
        )}
        {visibleMcpServers.length > MCP_SERVER_PAGE_SIZE && (
          <div data-testid='mcp-server-pagination' className='mt-12px flex justify-end'>
            <Pagination
              size='small'
              current={effectivePage}
              pageSize={MCP_SERVER_PAGE_SIZE}
              total={visibleMcpServers.length}
              showTotal
              onChange={(page) => setCurrentPage(page)}
            />
          </div>
        )}
      </div>

      <AddMcpServerModal
        visible={showMcpModal}
        server={editingMcpServer}
        existingServerNames={mcpServers.map((server) => server.name)}
        onCancel={hideMcpModal}
        onSubmit={
          editingMcpServer
            ? (serverData) => wrappedHandleEditMcpServer(editingMcpServer, serverData)
            : wrappedHandleAddMcpServer
        }
        onBatchImport={wrappedHandleBatchImportMcpServers}
        importMode={importMode}
      />

      <Modal
        title={t('settings.mcpDeleteServer')}
        visible={deleteConfirmVisible}
        onCancel={hideDeleteConfirm}
        onOk={handleConfirmDelete}
        okButtonProps={{ status: 'danger' }}
        okText={t('common.confirm')}
        cancelText={t('common.cancel')}
      >
        <p>{t('settings.mcpDeleteConfirm')}</p>
      </Modal>
    </div>
  );
};

/**
 * State-injected variant so hosts that already own the MCP server state (e.g.
 * the /mcp hub page with its market tabs) can share one `useMcpServers`
 * instance across tabs instead of double-fetching.
 */
export const ToolsModalContentWithState: React.FC<{
  mcpMessage: MessageInstance;
  mcpMessageContext: React.ReactNode;
  mcpServers: IMcpServer[];
  setMcpServers: React.Dispatch<React.SetStateAction<IMcpServer[]>>;
  saveMcpServers: (serversOrUpdater: IMcpServer[] | ((prev: IMcpServer[]) => IMcpServer[])) => Promise<void>;
  headerActionHost: HTMLDivElement | null;
}> = ({ mcpMessage, mcpMessageContext, mcpServers, saveMcpServers, setMcpServers, headerActionHost }) => {
  return (
    <div className='flex flex-col h-full w-full'>
      {mcpMessageContext}

      <NomiScrollArea className='flex-1 min-h-0 pb-12px' disableOverflow>
        <div
          data-testid='mcp-installed-surface'
          className={`${ENHANCED_TOOLS_SURFACE_CLASS} flex min-h-0 flex-col`}
        >
          <NomiScrollArea className='h-full overflow-visible' disableOverflow>
            <ModalMcpManagementSection
              message={mcpMessage}
              mcpServers={mcpServers}
              setMcpServers={setMcpServers}
              saveMcpServers={saveMcpServers}
              headerActionHost={headerActionHost}
            />
          </NomiScrollArea>
        </div>
      </NomiScrollArea>
    </div>
  );
};
