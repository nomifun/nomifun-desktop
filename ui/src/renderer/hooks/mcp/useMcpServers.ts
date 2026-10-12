import { useCallback, useEffect, useRef, useState } from 'react';
import type { IMcpServer } from '@/common/config/storage';
import { ensureBackendMcpCatalog, subscribeMcpCatalogChanged } from './catalog';

/**
 * MCP server state hook.
 * Reads the canonical backend-managed MCP catalog.
 */
export const useMcpServers = () => {
  const [mcpServers, setMcpServers] = useState<IMcpServer[]>([]);
  const [isMcpServersLoading, setIsMcpServersLoading] = useState(true);
  const [mcpServersLoadFailed, setMcpServersLoadFailed] = useState(false);
  const hasLoaded = useRef(false);
  const mounted = useRef(true);
  const generation = useRef(0);
  const [reloadToken, setReloadToken] = useState(0);
  const retryMcpServers = useCallback(() => setReloadToken((value) => value + 1), []);

  useEffect(() => {
    mounted.current = true;
    let active = true;
    const reload = async () => {
      const requestGeneration = ++generation.current;
      if (!hasLoaded.current) {
        setIsMcpServersLoading(true);
        setMcpServersLoadFailed(false);
      }
      try {
        const { allServers } = await ensureBackendMcpCatalog();
        if (!active || requestGeneration !== generation.current) return;
        hasLoaded.current = true;
        setMcpServers(allServers);
        setMcpServersLoadFailed(false);
      } catch (error) {
        if (!active || requestGeneration !== generation.current) return;
        console.error('[useMcpServers] Failed to load MCP catalog:', error);
        // A failed refresh must not erase a catalog already shown to the user.
        setMcpServersLoadFailed(true);
      } finally {
        if (active && requestGeneration === generation.current) setIsMcpServersLoading(false);
      }
    };
    const unsubscribe = subscribeMcpCatalogChanged(() => { void reload(); });
    void reload();
    return () => { active = false; mounted.current = false; unsubscribe(); };
  }, [reloadToken]);

  const saveMcpServers = useCallback(async (serversOrUpdater: IMcpServer[] | ((prev: IMcpServer[]) => IMcpServer[])) => {
    // CRUD applies its saved row before broadcasting invalidation. Fence reads
    // that started before that write, including the pending initial catalog.
    generation.current += 1;
    if (!mounted.current) return;
    // Resolve independently of React executing the updater. A pending CRUD
    // response must still broadcast its backend change after this view closes.
    setMcpServers((prevServers) => typeof serversOrUpdater === 'function' ? serversOrUpdater(prevServers) : serversOrUpdater);
  }, []);

  return {
    mcpServers,
    isMcpServersLoading,
    mcpServersLoadFailed,
    retryMcpServers,
    saveMcpServers,
  };
};
