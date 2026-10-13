import '../../../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { SWRConfig, useSWRConfig } from 'swr';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { HostedApp } from '@/common/types/appHosting';
import { addProjectWorkpath, getProjectWorkpaths } from '../utils/projectWorkpaths';
import { useOrdinaryProjectWorkpaths } from './useOrdinaryProjectWorkpaths';

const storageKey = 'nomifun:session-list-project-workpaths';
afterEach(() => { cleanup(); mock.restore(); localStorage.removeItem(storageKey); });
const wrapper = ({ children }: { children: React.ReactNode }) => <SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0, shouldRetryOnError: false }}>{children}</SWRConfig>;
const app = { id: 'owned-app', directory: 'C:/managed/owned-app' } as HostedApp;

test('Host-confirmed App directories are removed from old ordinary project preferences', async () => {
  addProjectWorkpath(app.directory); addProjectWorkpath('C:/ordinary/project'); addProjectWorkpath(`${app.directory}/child`);
  const paths = getProjectWorkpaths();
  spyOn(appHosting.list, 'invoke').mockResolvedValue([app]);
  const hook = renderHook(() => useOrdinaryProjectWorkpaths(paths), { wrapper });
  await waitFor(() => expect(hook.result.current).toEqual(['C:/managed/owned-app/child', 'C:/ordinary/project']));
  expect(getProjectWorkpaths()).toEqual(['C:/managed/owned-app/child', 'C:/ordinary/project']);
});

test('unknown App metadata preserves ordinary empty projects and their existing preferences', async () => {
  addProjectWorkpath(app.directory); addProjectWorkpath('C:/ordinary/project');
  const paths = getProjectWorkpaths();
  const read = spyOn(appHosting.list, 'invoke').mockRejectedValue(new Error('Host unavailable'));
  const hook = renderHook(() => useOrdinaryProjectWorkpaths(paths), { wrapper });
  await waitFor(() => expect(read).toHaveBeenCalled());
  expect(hook.result.current).toEqual(paths);
  expect(getProjectWorkpaths()).toEqual(paths);
});

test('a refresh error retains successful owner metadata and does not reveal an old App entry', async () => {
  const paths = [app.directory, 'C:/ordinary/project'];
  const read = spyOn(appHosting.list, 'invoke').mockResolvedValue([app]);
  const hook = renderHook(() => ({ paths: useOrdinaryProjectWorkpaths(paths), refresh: useSWRConfig().mutate }), { wrapper });
  await waitFor(() => expect(hook.result.current.paths).toEqual(['C:/ordinary/project']));
  read.mockRejectedValue(new Error('Refresh unavailable'));
  await act(async () => { await hook.result.current.refresh('apps.hosted').catch(() => {}); });
  expect(hook.result.current.paths).toEqual(['C:/ordinary/project']);
});
