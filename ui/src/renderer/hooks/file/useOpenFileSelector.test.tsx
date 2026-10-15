import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import { useOpenFileSelector } from './useOpenFileSelector';

afterEach(() => { cleanup(); mock.restore(); });

test('file selection shares one action and reports only genuine current-target failures', async () => {
  const selected = mock();
  const dialog = spyOn(ipcBridge.dialog.showOpen, 'invoke').mockResolvedValueOnce(['/workspace/file.txt'])
    .mockResolvedValueOnce([]).mockRejectedValueOnce(new Error('current dialog failed'));
  const hook = renderHook(() => useOpenFileSelector({ targetKey: 'current', onFilesSelected: selected }));
  await act(async () => hook.result.current.openFileSelector());
  expect(selected).toHaveBeenCalledWith(['/workspace/file.txt']);
  await act(async () => hook.result.current.openFileSelector());
  expect(selected).toHaveBeenCalledTimes(1);
  await expect(hook.result.current.openFileSelector()).rejects.toThrow('current dialog failed');
  expect(dialog).toHaveBeenCalledTimes(3);
});

test.each(['success', 'failure'])('a stale file selection cannot affect a replacement target (%s)', async outcome => {
  let finish!: (paths: string[]) => void; let reject!: (error: Error) => void;
  const selected = mock();
  spyOn(ipcBridge.dialog.showOpen, 'invoke').mockImplementation(() => new Promise((resolve, fail) => { finish = resolve; reject = fail; }));
  const hook = renderHook(({ target }) => useOpenFileSelector({ targetKey: target, onFilesSelected: selected }), { initialProps: { target: 'A' } });
  const pending = hook.result.current.openFileSelector();
  hook.rerender({ target: 'B' }); hook.rerender({ target: 'A' });
  await act(async () => {
    if (outcome === 'success') finish(['/workspace/stale.txt']); else reject(new Error('stale dialog failed'));
    await pending;
  });
  expect(selected).not.toHaveBeenCalled();
});

test('unmount and an invalidated command context both discard picker results', async () => {
  const selected = mock();
  let finish!: (paths: string[]) => void;
  spyOn(ipcBridge.dialog.showOpen, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const hook = renderHook(() => useOpenFileSelector({ targetKey: 'A', onFilesSelected: selected }));
  let current = true;
  const first = hook.result.current.openFileSelector({ isCurrent: () => current, idempotencyKey: 'fixture' });
  current = false;
  await act(async () => { finish(['/workspace/stale.txt']); await first; });
  const second = hook.result.current.openFileSelector();
  hook.unmount();
  await act(async () => { finish(['/workspace/stale.txt']); await second; });
  expect(selected).not.toHaveBeenCalled();
});
