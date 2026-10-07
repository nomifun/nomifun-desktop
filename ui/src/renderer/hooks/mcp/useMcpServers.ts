import { useCallback, useEffect, useState } from 'react';
import type { IMcpServer } from '@/common/config/storage';
import { ensureBackendMcpCatalog } from './catalog';

/**
 * MCP server state hook.
 * Reads the canonical backend-managed MCP catalog.
 */
export const useMcpServers = () => {
  const [mcpServers, setMcpServers] = useState<IMcpServer[]>([]);
  const [isMcpServersLoading, setIsMcpServersLoading] = useState(true);
  const [mcpServersLoadFailed, setMcpServersLoadFailed] = useState(false);

  useEffect(() => {
    void ensureBackendMcpCatalog()
      .then(({ allServers }) => {
        setMcpServers(allServers);
        setMcpServersLoadFailed(false);
      })
      .catch((error) => {
        console.error('[useMcpServers] Failed to load MCP catalog:', error);
        setMcpServers([]);
        setMcpServersLoadFailed(true);
      })
      .finally(() => {
        setIsMcpServersLoading(false);
      });
  }, []);

  const saveMcpServers = useCallback((serversOrUpdater: IMcpServer[] | ((prev: IMcpServer[]) => IMcpServer[])) => {
    return new Promise<void>((resolve) => {
      setMcpServers((prevServers) => {
        const nextServers = typeof serversOrUpdater === 'function' ? serversOrUpdater(prevServers) : serversOrUpdater;
        queueMicrotask(resolve);
        return nextServers;
      });
    });
  }, []);

  return {
    mcpServers,
    isMcpServersLoading,
    mcpServersLoadFailed,
    setMcpServers,
    saveMcpServers,
  };
};
