import { appHosting } from '@/common/adapter/appHostingBridge';
import { useMemo } from 'react';
import useSWR from 'swr';
import { withAppProjectNames } from '../utils/appProjectWorkpaths';
import type { WorkpathNode } from '../utils/workpathTree';

/** Shares the Apps owner cache; unavailable metadata falls back to ordinary paths. */
export function useAppProjectWorkpathNames(tree: WorkpathNode[]): WorkpathNode[] {
  const { data, error } = useSWR('apps.hosted', () => appHosting.list.invoke(), { refreshInterval: 2000, shouldRetryOnError: false });
  return useMemo(() => withAppProjectNames(tree, error ? undefined : data), [tree, data, error]);
}
