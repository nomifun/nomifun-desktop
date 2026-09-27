import { afterEach, expect, mock, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { useWorkspaceSearch } from './useWorkspaceSearch';

afterEach(cleanup);

test('a failed workspace search keeps its input available for correction or retry', async () => {
  const loadWorkspace = mock(async () => null);
  const hook = renderHook(() => useWorkspaceSearch({ workspace: '/workspace', loadWorkspace }));
  await act(async () => {
    hook.result.current.onSearch('needle');
    await new Promise((resolve) => setTimeout(resolve, 250));
  });
  expect(loadWorkspace).toHaveBeenCalledWith('/workspace', 'needle');
  expect(hook.result.current.showSearch).toBe(true);
});
