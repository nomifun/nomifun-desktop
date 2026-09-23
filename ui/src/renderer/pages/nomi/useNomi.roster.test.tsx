import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { ICompanionWithStatus } from '@/common/adapter/ipcBridge';
import { parseCompanionId } from '@/common/types/ids';
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

test('desktop visibility saves the companion patch and updates its roster row without losing status', async () => {
  const subscriptions = [
    ipcBridge.companion.onCompanionCreated,
    ipcBridge.companion.onCompanionDeleted,
    ipcBridge.companion.onConfigUpdated,
    ipcBridge.companion.onLearnFinished,
  ].map((event) => spyOn(event, 'on').mockImplementation(() => () => {}));
  const companionId = parseCompanionId('019f0000-0000-7000-8000-000000000001');
  const row = { companion_id: companionId, appearance: { companion_enabled: false }, status: { level: 3 } } as ICompanionWithStatus;
  const list = spyOn(ipcBridge.companion.listCompanions, 'invoke').mockResolvedValue([row]);
  const { status: _status, ...profile } = row;
  const patch = spyOn(ipcBridge.companion.patchCompanion, 'invoke').mockResolvedValue({
    ...profile, appearance: { ...profile.appearance, companion_enabled: true },
  });
  try {
    const view = renderHook(() => useCompanions());
    await act(async () => { await Promise.resolve(); });
    await act(() => view.result.current.setDesktopVisible(companionId, true));
    expect(patch).toHaveBeenCalledWith({ companion_id: companionId, patch: { appearance: { companion_enabled: true } } });
    expect(view.result.current.companions[0].appearance.companion_enabled).toBe(true);
    expect(view.result.current.companions[0].status).toBe(row.status);
    view.unmount();
  } finally {
    patch.mockRestore(); list.mockRestore(); subscriptions.forEach((subscription) => subscription.mockRestore());
  }
});
