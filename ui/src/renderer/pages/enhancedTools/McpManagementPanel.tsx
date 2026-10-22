/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMcpServer } from '@/common/config/storage';
import type { McpOAuthClientConfig } from '@/common/types/mcpAuth';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { useMcpConnection, useMcpOAuth, useMcpServerCRUD } from '@/renderer/hooks/mcp';
import { supportsMcpOAuthLogin } from '@/renderer/hooks/mcp/mcpAuthConfig';
import { resolveMcpDisplayName } from '@/renderer/services/mcp/mcpMarket';
import AddMcpServerModal from '@/renderer/pages/settings/components/AddMcpServerModal';
import McpServerItem from '@/renderer/pages/settings/ToolsSettings/McpServerItem';
import McpOAuthClientModal from '@/renderer/pages/settings/ToolsSettings/McpOAuthClientModal';
import { validatedMcpAuthorizationUrl } from '@/renderer/hooks/mcp/useMcpOAuth';
import { openExternalUrl } from '@/renderer/utils/platform';
import mcpListStyles from '@/renderer/pages/settings/ToolsSettings/McpServerList.module.css';
import NomiPagination from '@/renderer/components/base/NomiPagination';
import { Modal } from '@arco-design/web-react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { McpServerId } from '@/common/types/ids';
import { useMcpDisplayName } from '@/renderer/services/mcp/useMcpDisplayName';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';

const MCP_SERVER_PAGE_SIZE = 10;

export type McpImportMode = 'json' | 'oneclick';

interface McpManagementPanelProps {
  mcpServers: IMcpServer[];
  saveMcpServers: (serversOrUpdater: IMcpServer[] | ((prev: IMcpServer[]) => IMcpServer[])) => Promise<void>;
  searchQuery: string;
  importMode: McpImportMode | null;
  onImportClose: () => void;
  onImported?: (servers: IMcpServer[]) => void;
  highlightServerId?: string | null;
  visible?: boolean;
}

/** Management uses the host's canonical catalog; changing categories never creates a second copy. */
const McpManagementPanel: React.FC<McpManagementPanelProps> = ({
  mcpServers, saveMcpServers, searchQuery, importMode, onImportClose, onImported, highlightServerId, visible = true,
}) => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage({ maxCount: 10 });
  const getDisplayName = useMcpDisplayName();
  const { oauthStatus, attempts, loggingIn, checkOAuthStatus, login, cancelLogin, logout } = useMcpOAuth();
  const [configuringOAuthServer, setConfiguringOAuthServer] = useState<IMcpServer>();
  const [editingMcpServer, setEditingMcpServer] = useState<IMcpServer>();
  const dialogOwner = useMemo(() => ({}), [visible, importMode, editingMcpServer]);
  const currentDialogOwner = useRef(dialogOwner);
  currentDialogOwner.current = dialogOwner;
  const [serverToDelete, setServerToDelete] = useState<McpServerId | null>(null);
  const [expandedServerIds, setExpandedServerIds] = useState<Set<McpServerId>>(() => new Set());
  const query = searchQuery.trim().toLocaleLowerCase();
  const visibleMcpServers = useMemo(
    () => mcpServers.filter((server) =>
      [getDisplayName(server), server.name, server.description].some((value) => value?.toLocaleLowerCase().includes(query))
    ),
    [mcpServers, getDisplayName, query]
  );
  const [currentPage, setCurrentPage] = useState(1);
  const previousServerCount = useRef(mcpServers.length);
  const previousQuery = useRef(query);
  const appliedHighlight = useRef<string | null>(null);
  const totalPages = Math.max(1, Math.ceil(visibleMcpServers.length / MCP_SERVER_PAGE_SIZE));
  const effectivePage = previousQuery.current === query ? Math.min(currentPage, totalPages) : 1;
  const paginatedMcpServers = useMemo(() => {
    const start = (effectivePage - 1) * MCP_SERVER_PAGE_SIZE;
    return visibleMcpServers.slice(start, start + MCP_SERVER_PAGE_SIZE);
  }, [effectivePage, visibleMcpServers]);

  useEffect(() => {
    const queryChanged = previousQuery.current !== query;
    const serverWasAdded = mcpServers.length > previousServerCount.current;
    const highlightedIndex = highlightServerId && highlightServerId !== appliedHighlight.current
      ? visibleMcpServers.findIndex((server) => server.mcp_server_id === highlightServerId)
      : -1;
    setCurrentPage((page) => {
      if (highlightedIndex >= 0) return Math.floor(highlightedIndex / MCP_SERVER_PAGE_SIZE) + 1;
      if (queryChanged) return 1;
      if (serverWasAdded && !query) return totalPages;
      return Math.min(page, totalPages);
    });
    previousQuery.current = query;
    previousServerCount.current = mcpServers.length;
    if (highlightedIndex >= 0) appliedHighlight.current = highlightServerId ?? null;
    if (!highlightServerId) appliedHighlight.current = null;
  }, [highlightServerId, mcpServers.length, query, totalPages, visibleMcpServers]);

  const closeServerModal = useCallback(() => {
    // An import/edit may finish after this dialog was closed or its category
    // hidden. Its writeback remains valid; it cannot dismiss a newer dialog.
    if (currentDialogOwner.current !== dialogOwner) return;
    setEditingMcpServer(undefined);
    onImportClose();
  }, [dialogOwner, onImportClose]);
  const toggleServer = useCallback((id: McpServerId) => {
    setExpandedServerIds((previous) => {
      const next = new Set(previous);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const handleAuthRequired = useCallback(
    (server: IMcpServer) => {
      if (supportsMcpOAuthLogin(server.transport)) {
        void checkOAuthStatus(server);
      }
    },
    [checkOAuthStatus]
  );
  const handleAuthResolved = useCallback(
    (server: IMcpServer) => {
      void checkOAuthStatus(server);
    },
    [checkOAuthStatus]
  );

  const { testingServers, handleTestMcpConnection, handleTestMcpConnections } = useMcpConnection(
    handleAuthRequired,
    handleAuthResolved
  );
  const {
    handleAddMcpServer,
    handleBatchImportMcpServers,
    handleEditMcpServer,
    handleDeleteMcpServer,
    handleToggleMcpServer,
    togglingServers,
  } = useMcpServerCRUD(saveMcpServers);

  const handleOAuthLogin = useCallback(
    async (server: IMcpServer, client?: McpOAuthClientConfig) => {
      const displayName = resolveMcpDisplayName(server);
      const result = await login(server, client);

      if (result.success) {
        message.success(`${displayName}: ${t('settings.mcpOAuthLoginSuccess') || 'Login successful'}`);
        void handleTestMcpConnection(server);
      } else if (!result.cancelled) {
        message.error(`${displayName}: ${result.error || t('settings.mcpOAuthLoginFailed') || 'Login failed'}`);
        if (result.diagnostic?.user_action === 'configure_client') setConfiguringOAuthServer(server);
      }
    },
    [login, message, t, handleTestMcpConnection]
  );

  const handleOAuthCancel = useCallback(async (server: IMcpServer) => {
    const result = await cancelLogin(server);
    if (!result.success) message.error(result.error ?? t('settings.mcpOAuthLoginFailed'));
  }, [cancelLogin, message, t]);

  const handleOAuthLogout = useCallback(async (server: IMcpServer) => {
    const result = await logout(server);
    if (!result.success) message.error(result.error ?? t('settings.mcpOAuthLoginFailed'));
  }, [logout, message, t]);

  const handleOAuthContinue = useCallback((server: IMcpServer) => {
    const url = attempts[server.mcp_server_id]?.authorization_url;
    if (!url) return;
    try {
      void openExternalUrl(validatedMcpAuthorizationUrl(url)).catch(() => message.error(t('settings.mcpOAuthOpenFailed')));
    } catch { message.error(t('settings.mcpOAuthOpenFailed')); }
  }, [attempts, message, t]);

  const wrappedHandleAddMcpServer = useCallback(
    async (serverData: Omit<IMcpServer, 'mcp_server_id' | 'created_at' | 'updated_at'>) => {
      const addedServer = await handleAddMcpServer(serverData);
      if (addedServer) {
        void handleTestMcpConnection(addedServer, { notify: false });
        onImported?.([addedServer]);
      }
    },
    [handleAddMcpServer, handleTestMcpConnection, onImported]
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
        onImported?.(addedServers);
        await handleTestMcpConnections(addedServers, { concurrency: 4, notify: false });
      }
      return addedServers;
    },
    [handleBatchImportMcpServers, handleTestMcpConnections, onImported]
  );

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
    setServerToDelete(null);
    await handleDeleteMcpServer(serverToDelete);
  }, [serverToDelete, handleDeleteMcpServer]);

  return (
    <div className='flex min-h-0 flex-1 flex-col' data-testid='mcp-management-panel'>
      {messageContext}

      <NomiScrollArea className='flex-1 min-h-0' data-testid='mcp-installed-surface'>
        {visibleMcpServers.length === 0 ? (
          <div className='py-20px text-center text-t-secondary text-14px border border-dashed border-arco-2 rd-12px'>
            {t('settings.mcpNoServersFound')}
          </div>
        ) : (
          <div className={mcpListStyles.surface}>
            <div className={mcpListStyles.list}>
              {paginatedMcpServers.map((server) => (
                  <McpServerItem
                    key={server.mcp_server_id}
                    server={server}
                    expanded={expandedServerIds.has(server.mcp_server_id)}
                    highlighted={server.mcp_server_id === highlightServerId}
                    isTestingConnection={testingServers[server.mcp_server_id] || false}
                    oauthStatus={oauthStatus[server.mcp_server_id]}
                    oauthAttempt={attempts[server.mcp_server_id]}
                    isLoggingIn={loggingIn[server.mcp_server_id]}
                    isTogglingEnabled={togglingServers[server.mcp_server_id] || false}
                    onToggleExpanded={() => toggleServer(server.mcp_server_id)}
                    onTestConnection={handleTestMcpConnection}
                    onEditServer={setEditingMcpServer}
                    onDeleteServer={setServerToDelete}
                    onOAuthLogin={handleOAuthLogin}
                    onOAuthCancel={handleOAuthCancel}
                    onOAuthLogout={handleOAuthLogout}
                    onOAuthContinue={handleOAuthContinue}
                    onOAuthConfigure={setConfiguringOAuthServer}
                    onToggleEnabled={handleToggleMcpServer}
                  />
              ))}
            </div>
          </div>
        )}
        {visibleMcpServers.length > MCP_SERVER_PAGE_SIZE && (
          <div data-testid='mcp-server-pagination' className='mt-12px flex justify-end pb-16px'>
            <NomiPagination
              current={effectivePage}
              pageSize={MCP_SERVER_PAGE_SIZE}
              total={visibleMcpServers.length}
              showTotal
              onChange={(page) => setCurrentPage(page)}
            />
          </div>
        )}
      </NomiScrollArea>

      <AddMcpServerModal
        visible={visible && Boolean(editingMcpServer || importMode)}
        server={editingMcpServer}
        existingServerNames={mcpServers.map((server) => server.name)}
        onCancel={closeServerModal}
        onSubmit={
          editingMcpServer
            ? (serverData) => wrappedHandleEditMcpServer(editingMcpServer, serverData)
            : wrappedHandleAddMcpServer
        }
        onBatchImport={wrappedHandleBatchImportMcpServers}
        importMode={importMode ?? 'json'}
      />

      {visible && configuringOAuthServer && <McpOAuthClientModal
        serverName={resolveMcpDisplayName(configuringOAuthServer)}
        onCancel={() => setConfiguringOAuthServer(undefined)}
        onSubmit={(client) => {
          const server = configuringOAuthServer;
          setConfiguringOAuthServer(undefined);
          void handleOAuthLogin(server, client);
        }}
      />}

      {visible && <Modal
        title={t('settings.mcpDeleteServer')}
        visible={serverToDelete !== null}
        onCancel={() => setServerToDelete(null)}
        onOk={handleConfirmDelete}
        okButtonProps={{ status: 'danger' }}
        okText={t('common.confirm')}
        cancelText={t('common.cancel')}
      >
        <p>{t('settings.mcpDeleteConfirm')}</p>
      </Modal>}
    </div>
  );
};

export default McpManagementPanel;
