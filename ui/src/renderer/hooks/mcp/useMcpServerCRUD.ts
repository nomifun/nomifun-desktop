import type { McpServerId } from '@/common/types/ids';
import { useCallback, useRef, useState } from 'react';
import { Message } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { mcpService } from '@/common/adapter/ipcBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { IMcpServer } from '@/common/config/storage';
import { notifyMcpCatalogChanged, toBackendMcpPayload } from './catalog';

const replaceUserServer = (servers: IMcpServer[], nextServer: IMcpServer) => {
  const remainingServers = servers.filter((server) => server.builtin === true || server.mcp_server_id !== nextServer.mcp_server_id);
  const insertIndex = remainingServers.findIndex((server) => server.builtin === true);

  if (insertIndex === -1) {
    return [...remainingServers, nextServer];
  }

  remainingServers.splice(insertIndex, 0, nextServer);
  return remainingServers;
};

const getMcpRequestErrorMessage = (error: unknown, fallback: string): string => {
  if (isBackendHttpError(error) && error.backendMessage.trim()) return error.backendMessage;
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === 'string' && error.trim()) return error;
  return fallback;
};
export const useMcpServerCRUD = (
  saveMcpServers: (serversOrUpdater: IMcpServer[] | ((prev: IMcpServer[]) => IMcpServer[])) => Promise<void>
) => {
  const { t } = useTranslation();
  const pendingToggles = useRef(new Set<McpServerId>());
  const [togglingServers, setTogglingServers] = useState<Record<string, boolean>>({});

  const persistEnabledState = useCallback(async (server: IMcpServer, enabled: boolean) => {
    if (server.enabled === enabled) {
      return server;
    }

    return mcpService.toggleServer.invoke({ mcp_server_id: server.mcp_server_id });
  }, []);

  const handleAddMcpServer = useCallback(
    async (serverData: Omit<IMcpServer, 'mcp_server_id' | 'created_at' | 'updated_at'>) => {
      try {
        let persisted = await mcpService.createServer.invoke(toBackendMcpPayload(serverData));
        persisted = await persistEnabledState(persisted, serverData.enabled);

        await saveMcpServers((prevServers) => replaceUserServer(prevServers, persisted));
        notifyMcpCatalogChanged();
        return persisted;
      } catch (error) {
        Message.error(getMcpRequestErrorMessage(error, t('settings.mcpImportFailed')));
        return undefined;
      }
    },
    [persistEnabledState, saveMcpServers, t]
  );

  const handleBatchImportMcpServers = useCallback(
    async (serversData: Omit<IMcpServer, 'mcp_server_id' | 'created_at' | 'updated_at'>[]) => {
      try {
        const imported = await mcpService.importServers.invoke({
          servers: serversData.map((server) => toBackendMcpPayload(server)),
        });

        const finalServers: IMcpServer[] = [];
        for (const importedServer of imported) {
          const original = serversData.find((server) => server.name === importedServer.name);
          const persisted = await persistEnabledState(importedServer, original?.enabled ?? false);
          finalServers.push(persisted);
        }

        await saveMcpServers((prevServers) => {
          let nextServers = prevServers.filter((server) => server.builtin === true);
          const existingUserServers = prevServers.filter((server) => server.builtin !== true);

          for (const server of existingUserServers) {
            if (!finalServers.some((next) => next.mcp_server_id === server.mcp_server_id || next.name === server.name)) {
              nextServers = [...nextServers, server];
            }
          }

          for (const server of finalServers) {
            nextServers = replaceUserServer(nextServers, server);
          }

          return nextServers;
        });
        notifyMcpCatalogChanged();

        return finalServers;
      } catch (error) {
        Message.error(getMcpRequestErrorMessage(error, t('settings.mcpImportFailed')));
        return [];
      }
    },
    [persistEnabledState, saveMcpServers, t]
  );

  const handleEditMcpServer = useCallback(
    async (
      editingMcpServer: IMcpServer | undefined,
      serverData: Omit<IMcpServer, 'mcp_server_id' | 'created_at' | 'updated_at'>
    ): Promise<IMcpServer | undefined> => {
      if (!editingMcpServer) {
        return undefined;
      }

      try {
        let persisted = await mcpService.updateServer.invoke({
          mcp_server_id: editingMcpServer.mcp_server_id,
          data: toBackendMcpPayload(serverData),
        });
        persisted = await persistEnabledState(persisted, serverData.enabled);

        await saveMcpServers((prevServers) =>
          prevServers.map((server) => (server.mcp_server_id === editingMcpServer.mcp_server_id ? persisted : server))
        );
        notifyMcpCatalogChanged();

        Message.success(t('settings.mcpImportSuccess'));
        return persisted;
      } catch (error) {
        Message.error(getMcpRequestErrorMessage(error, t('settings.mcpImportFailed')));
        return undefined;
      }
    },
    [persistEnabledState, saveMcpServers, t]
  );

  const handleDeleteMcpServer = useCallback(
    async (serverId: McpServerId) => {
      await mcpService.deleteServer.invoke({ mcp_server_id: serverId });
      await saveMcpServers((prevServers) => prevServers.filter((server) => server.mcp_server_id !== serverId));
      notifyMcpCatalogChanged();
      Message.success(t('settings.mcpDeleted'));
    },
    [saveMcpServers, t]
  );

  const handleToggleMcpServer = useCallback(
    async (server: IMcpServer): Promise<IMcpServer | undefined> => {
      const serverId = server.mcp_server_id;
      if (pendingToggles.current.has(serverId)) return undefined;

      pendingToggles.current.add(serverId);
      setTogglingServers((previous) => ({ ...previous, [serverId]: true }));
      try {
        const persisted = await mcpService.toggleServer.invoke({ mcp_server_id: serverId });
        await saveMcpServers((previous) =>
          previous.map((current) => {
            if (current.mcp_server_id !== serverId) return current;
            return current.updated_at > persisted.updated_at ? current : persisted;
          })
        );
        return persisted;
      } catch (error) {
        Message.error(getMcpRequestErrorMessage(error, t('settings.mcpToggleFailed')));
        return undefined;
      } finally {
        pendingToggles.current.delete(serverId);
        setTogglingServers((previous) => {
          const next = { ...previous };
          delete next[serverId];
          return next;
        });
        // A failed response can follow a persisted write; reload canonical state
        // after every attempted toggle without claiming an optimistic success.
        notifyMcpCatalogChanged();
      }
    },
    [saveMcpServers, t]
  );

  return {
    togglingServers,
    handleToggleMcpServer,
    handleAddMcpServer,
    handleBatchImportMcpServers,
    handleEditMcpServer,
    handleDeleteMcpServer,
  };
};
