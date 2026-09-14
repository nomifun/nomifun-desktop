import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { ICompanionWithStatus } from '@/common/adapter/ipcBridge';
import { useCompanions } from './useNomi';

afterEach(cleanup);

test('roster failure is recoverable and keeps the last successful list', async () => {
  const subscriptions = [
    ipcBridge.companion.onCompanionCreated,
    ipcBridge.companion.onCompanionDeleted,
    ipcBridge.companion.onConfigUpdated,
    ipcBridge.companion.onLearnFinished,
  ].map((event) => spyOn(event, 'on').mockImplementation(() => () => {}));
  const row = { name: 'Companion' } as ICompanionWithStatus;
  const request = spyOn(ipcBridge.companion.listCompanions, 'invoke')
    .mockRejectedValueOnce(new Error('offline'))
    .mockResolvedValueOnce([row])
    .mockRejectedValueOnce(new Error('offline again'));
  try {
    const view = renderHook(() => useCompanions());
    await act(async () => { await Promise.resolve(); });
    expect(view.result.current.loading).toBe(false);
    expect(view.result.current.error?.message).toBe('offline');
    await act(() => view.result.current.refresh());
    expect(view.result.current.error).toBeNull();
    expect(view.result.current.companions).toEqual([row]);
    await act(() => view.result.current.refresh());
    expect(view.result.current.error?.message).toBe('offline again');
    expect(view.result.current.companions).toEqual([row]);
    view.unmount();
  } finally { request.mockRestore(); subscriptions.forEach((subscription) => subscription.mockRestore()); }
});
