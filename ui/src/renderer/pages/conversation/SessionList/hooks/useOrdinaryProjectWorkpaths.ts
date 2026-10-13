import { appHosting } from '@/common/adapter/appHostingBridge';
import { useEffect, useMemo } from 'react';
import useSWR from 'swr';
import { removeProjectWorkpath } from '../utils/projectWorkpaths';
import { workpathKey } from '../utils/workpathKey';

/** Retire old App entries from ordinary sidebar preferences using Host-confirmed directories. */
export function useOrdinaryProjectWorkpaths(paths: string[]): string[] {
  const { data: apps } = useSWR(paths.length ? 'apps.hosted' : null, () => appHosting.list.invoke(), {
    refreshInterval: 2000, shouldRetryOnError: false,
  });
  const appPaths = useMemo(() => new Set((apps ?? []).map(app => workpathKey(app.directory))), [apps]);
  useEffect(() => {
    for (const path of paths) if (appPaths.has(workpathKey(path))) removeProjectWorkpath(path);
  }, [paths, appPaths]);
  // A failed read does not change ordinary project preferences. SWR retains
  // successful metadata during refresh failures, so confirmed App paths stay hidden.
  return useMemo(() => paths.filter(path => !appPaths.has(workpathKey(path))), [paths, appPaths]);
}
