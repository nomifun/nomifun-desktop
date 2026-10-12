import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { useConversationExport } from './useConversationExport';

const id = parseConversationId('0190f5fe-7c00-7a00-8000-000000000001');
const restores: Array<() => void> = [];
afterEach(() => {
  cleanup();
  restores.splice(0).reverse().forEach((restore) => restore());
});

function fixture() {
  const conversation = spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ id, name: 'topic', type: 'nomi' } as TChatConversation);
  const message = (index: number) => ({
    id: `row-${index}`, message_id: parseMessageId(`0190f5fe-7c00-7a00-8000-${String(index).padStart(12, '0')}`),
    conversation_id: id, type: 'text' as const, position: 'right' as const,
    content: { content: index ? 'latest-message' : 'original-request' }, created_at: 1000 + index,
  });
  const history = spyOn(ipcBridge.database.getConversationMessages, 'invoke')
    .mockResolvedValueOnce({ items: [message(1)], total: 2, has_more: true, next_cursor: `1001:${id}` })
    .mockResolvedValueOnce({ items: [message(0)], total: 2, has_more: false });
  const write = spyOn(ipcBridge.fs.writeFile, 'invoke').mockResolvedValue(true);
  const log = spyOn(console, 'error').mockImplementation(() => {});
  for (const spy of [conversation, history, write, log]) restores.push(() => spy.mockRestore());
  const error = mock();
  const success = mock();
  const hook = renderHook(() => useConversationExport({ conversation_id: id, workspace: 'D:/project', t: (key) => key, messageApi: { error, success } }));
  return { hook, history, write, error, success };
}

test('slash export uses all history windows and derives its filename from the original request', async () => {
  const f = fixture();
  await act(async () => { await f.hook.result.current.openExportFlow(); });
  expect(f.hook.result.current.filename).toContain('-original-request.txt');
  act(() => f.hook.result.current.onSelectMenuItem('save'));
  await act(async () => { await f.hook.result.current.submitFilename(); });
  expect(f.history).toHaveBeenCalledTimes(2);
  const request = f.write.mock.calls[0][0];
  expect(request.data.indexOf('original-request')).toBeLessThan(request.data.indexOf('latest-message'));
  expect(request.path).toStartWith('D:/project/');
  expect(f.success).toHaveBeenCalledTimes(1);
  expect(f.hook.result.current.step).toBe('closed');
});

test('a later read failure keeps slash export closed and reports the actual cause', async () => {
  const f = fixture();
  f.history.mockReset()
    .mockResolvedValueOnce({ items: [], total: 2, has_more: true, next_cursor: `1001:${id}` })
    .mockRejectedValueOnce(new Error('history is unreadable'));
  await act(async () => { await f.hook.result.current.openExportFlow(); });
  expect(f.hook.result.current.step).toBe('closed');
  expect(f.error.mock.calls[0][0]).toContain('history is unreadable');
  expect(f.write).not.toHaveBeenCalled();
  expect(f.success).not.toHaveBeenCalled();
});

test('closing during preparation discards the late response and prevents reopening the menu', async () => {
  const f = fixture();
  let resolve!: (value: { items: []; total: number; has_more: boolean }) => void;
  f.history.mockReset().mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
  let completion!: Promise<void>;
  await act(async () => { completion = f.hook.result.current.openExportFlow(); });
  act(() => f.hook.result.current.closeExportFlow());
  await act(async () => { resolve({ items: [], total: 0, has_more: false }); await completion; });
  expect(f.hook.result.current.step).toBe('closed');
  expect(f.hook.result.current.loading).toBe(false);
  expect(f.error).not.toHaveBeenCalled();
});
