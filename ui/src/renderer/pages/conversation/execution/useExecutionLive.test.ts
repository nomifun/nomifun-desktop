import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import { executionId, makeDetail } from '../../../../../test/fixtures/conversationDelegation';
import { parseExecutionId } from '@/common/types/ids';
import { useExecutionLive } from './useExecutionLive';

let restore = () => {};
beforeEach(() => {
  const changed = spyOn(ipcBridge.agentExecution.events.changed, 'on').mockImplementation(() => () => {});
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(() => () => {});
  restore = () => { changed.mockRestore(); reconnect.mockRestore(); };
});
afterEach(() => { cleanup(); restore(); });

test('a transient snapshot read failure preserves known work until an authoritative completion', async () => {
  const running = makeDetail();
  const get = spyOn(ipcBridge.agentExecution.get, 'invoke').mockResolvedValue(running);
  const error = spyOn(console, 'error').mockImplementation(() => {});
  try {
    const page = renderHook(() => useExecutionLive(executionId));
    await waitFor(() => expect(page.result.current.detail).toBe(running));
    get.mockRejectedValueOnce(new Error('temporary disconnect'));
    await act(async () => { await page.result.current.refetch(); });
    expect(page.result.current.detail?.execution.status).toBe('running');
    expect(page.result.current.loading).toBe(false);
    const completed = makeDetail({ execution: { ...running.execution, status: 'completed', updated_at: 5000 } });
    get.mockResolvedValue(completed);
    await act(async () => { await page.result.current.refetch(); });
    expect(page.result.current.detail).toBe(completed);
  } finally { get.mockRestore(); error.mockRestore(); }
});

test('paused work still reconciles by polling when realtime notifications are missed', async () => {
  const detail = makeDetail(); detail.execution.status = 'paused';
  const get = spyOn(ipcBridge.agentExecution.get, 'invoke').mockResolvedValue(detail);
  try {
    const page = renderHook(() => useExecutionLive(executionId));
    await waitFor(() => expect(page.result.current.detail?.execution.status).toBe('paused'));
    const calls = get.mock.calls.length;
    get.mockResolvedValue(makeDetail({ execution: { ...detail.execution, status: 'running', event_sequence: 2 } }));
    await waitFor(() => expect(page.result.current.detail?.execution.status).toBe('running'), { timeout: 2500 });
    expect(get.mock.calls.length).toBeGreaterThan(calls);
  } finally { get.mockRestore(); }
});

test('changing execution clears the previous snapshot even when the replacement read fails', async () => {
  const get = spyOn(ipcBridge.agentExecution.get, 'invoke').mockResolvedValue(makeDetail());
  const error = spyOn(console, 'error').mockImplementation(() => {});
  try {
    const page = renderHook(({ id }) => useExecutionLive(id), { initialProps: { id: executionId } });
    await waitFor(() => expect(page.result.current.detail).not.toBeNull());
    get.mockRejectedValueOnce(new Error('replacement is temporarily unavailable'));
    page.rerender({ id: parseExecutionId('00000000-0bb8-7000-8000-000000000099') });
    await waitFor(() => expect(page.result.current.loading).toBe(false));
    expect(page.result.current.detail).toBeNull();
  } finally { get.mockRestore(); error.mockRestore(); }
});
