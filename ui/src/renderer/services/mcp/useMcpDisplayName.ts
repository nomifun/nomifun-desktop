import { useCallback, useSyncExternalStore } from 'react';
import type { IMcpServer } from '@/common/config/storage';
import { getMcpMarketSnapshot, resolveMcpDisplayName, subscribeMcpMarketCache } from './mcpMarket';

/** Market titles are presentation only; callers keep the original server identity. */
export const useMcpDisplayName = () => {
  const items = useSyncExternalStore(subscribeMcpMarketCache, getMcpMarketSnapshot);
  return useCallback(
    (server: Pick<IMcpServer, 'name' | 'original_json' | 'builtin'>) => resolveMcpDisplayName(server, items),
    [items]
  );
};
