import '../../../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { SWRConfig, useSWRConfig } from 'swr';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { HostedApp } from '@/common/types/appHosting';
import { buildWorkpathTree } from '../utils/workpathTree';
import { useAppProjectWorkpathNames } from './useAppProjectWorkpathNames';

afterEach(() => { cleanup(); mock.restore(); });
const wrapper = ({ children }: { children: React.ReactNode }) => <SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0, shouldRetryOnError: false }}>{children}</SWRConfig>;

test('an App owner read failure restores ordinary path labels without changing the retained session groups', async () => {
  const tree = buildWorkpathTree([], [], [], ['C:/apps/project']);
  const app: HostedApp = { id: 'project', name: 'Calendar', description: '', tags: [], directory: 'C:/apps/project', development_session_id: null,
    import_kind: 'source', settings: { project_directory: '.', environment: {} }, state: 'stopped', operation: null, last_error: null, output: '', pid: null, preview: false, prepared: false };
  const owner = spyOn(appHosting.list, 'invoke').mockResolvedValue([app]);
  const hook = renderHook(() => ({ tree: useAppProjectWorkpathNames(tree), refresh: useSWRConfig().mutate }), { wrapper });
  await waitFor(() => expect(hook.result.current.tree.find(node => node.key === app.directory)?.displayName).toBe('Calendar'));
  owner.mockRejectedValue(new Error('App owner temporarily unavailable'));
  await act(async () => { await hook.result.current.refresh('apps.hosted').catch(() => {}); });
  await waitFor(() => expect(hook.result.current.tree).toBe(tree));
  expect(hook.result.current.tree.map(node => node.key)).toEqual(tree.map(node => node.key));
  expect(hook.result.current.tree.find(node => node.key === app.directory)?.displayName).toBe('project');
});
