import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { AppProjectSessionPage } from '@/common/types/appHosting';
import { useEffect, useMemo } from 'react';
import useSWRInfinite from 'swr/infinite';

/** Project grouping is a Host query over ordinary Sessions with the same current workspace. */
export function useAppProjectSessions(appId: string | undefined) {
  const { data, error, isLoading, isValidating, mutate, size, setSize } = useSWRInfinite(
    (index, previous: AppProjectSessionPage | null) => {
      if (!appId || index > 0 && (!previous?.has_more || !previous.next_cursor)) return null;
      return ['app-project-sessions', appId, index === 0 ? '' : previous!.next_cursor] as const;
    },
    ([, id, cursor]) => appHosting.developmentSessions.invoke({ id, limit: 30, ...(cursor ? { cursor } : {}) }),
    { refreshInterval: 2000, revalidateAll: true, shouldRetryOnError: false },
  );
  const items = useMemo(() => {
    const unique = new Map<string, AppProjectSessionPage['items'][number]>();
    for (const page of data ?? []) for (const item of page.items) unique.set(String(item.conversation.id), item);
    return [...unique.values()];
  }, [data]);
  useEffect(() => {
    if (!appId) return;
    const refresh = () => { void mutate().catch(() => {}); };
    const dispose = [ipcBridge.conversation.listChanged.on(refresh), ipcBridge.conversation.turnStarted.on(refresh),
      ipcBridge.conversation.turnCompleted.on(refresh), ipcBridge.conversation.turnPaused.on(refresh),
      ipcBridge.conversation.reconnected.on(refresh)];
    return () => dispose.forEach(stop => stop());
  }, [appId, mutate]);
  return { items, total: data?.[0]?.total ?? 0, error, isLoading, isValidating,
    hasMore: data?.[data.length - 1]?.has_more === true,
    loadMore: () => { void setSize(size + 1); }, refresh: () => mutate() };
}
