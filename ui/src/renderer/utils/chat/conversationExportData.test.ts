import { afterEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { readMessageContent } from './conversationExport';
import { fetchConversationExportMessages, fetchLatestConversationReply } from './conversationExportData';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000001');
const message = (index: number): TMessage => ({
  id: `row-${index}`,
  message_id: parseMessageId(`0190f5fe-7c00-7a00-8000-${String(index).padStart(12, '0')}`),
  conversation_id: conversationId,
  type: 'text', position: index % 2 ? 'right' : 'left',
  content: { content: `message-${index}` },
  created_at: 1000 + index,
});

const restores: Array<() => void> = [];
afterEach(() => restores.splice(0).forEach((restore) => restore()));

test('reads more than 500 messages and crosses an empty filtered history window in chronological order', async () => {
  const pages = [
    { items: Array.from({ length: 500 }, (_, i) => message(510 - i)), total: 511, has_more: true, next_cursor: `1011:${conversationId}` },
    { items: [], total: 511, has_more: true, next_cursor: `1009:${conversationId}` },
    { items: Array.from({ length: 9 }, (_, i) => message(8 - i)), total: 511, has_more: false },
  ];
  const read = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockImplementation(async () => pages.shift()!);
  restores.push(() => read.mockRestore());

  const result = await fetchConversationExportMessages(conversationId);
  expect(result).toHaveLength(509);
  expect(readMessageContent(result[0])).toBe('message-0');
  expect(readMessageContent(result.at(-1)!)).toBe('message-510');
  expect(read.mock.calls.map(([params]) => params)).toEqual([
    { conversation_id: conversationId, cursor: '', page_size: 500, content_mode: 'full' },
    { conversation_id: conversationId, cursor: `1011:${conversationId}`, page_size: 500, content_mode: 'full' },
    { conversation_id: conversationId, cursor: `1009:${conversationId}`, page_size: 500, content_mode: 'full' },
  ]);
});

test('a later history read failure rejects the export instead of returning a partial transcript', async () => {
  const read = spyOn(ipcBridge.database.getConversationMessages, 'invoke')
    .mockResolvedValueOnce({ items: [message(3)], total: 2, has_more: true, next_cursor: `1003:${conversationId}` })
    .mockRejectedValueOnce(new Error('canonical payload is unreadable'));
  restores.push(() => read.mockRestore());
  await expect(fetchConversationExportMessages(conversationId)).rejects.toThrow('canonical payload is unreadable');
});

test('missing or repeating authoritative cursors fail instead of truncating or looping', async () => {
  const read = spyOn(ipcBridge.database.getConversationMessages, 'invoke')
    .mockResolvedValueOnce({ items: [], total: 2, has_more: true })
    .mockResolvedValue({ items: [], total: 2, has_more: true, next_cursor: `1003:${conversationId}` });
  restores.push(() => read.mockRestore());
  await expect(fetchConversationExportMessages(conversationId)).rejects.toThrow('missing its pagination cursor');
  await expect(fetchConversationExportMessages(conversationId)).rejects.toThrow('did not advance');
  expect(read).toHaveBeenCalledTimes(3);
});

test('canceling a read prevents subsequent windows from being requested', async () => {
  let canceled = false;
  const read = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockImplementation(async () => {
    canceled = true;
    return { items: [message(3)], total: 2, has_more: true, next_cursor: `1003:${conversationId}` };
  });
  restores.push(() => read.mockRestore());
  await expect(fetchConversationExportMessages(conversationId, () => {
    if (canceled) throw new Error('canceled');
  })).rejects.toThrow('canceled');
  expect(read).toHaveBeenCalledTimes(1);
});

test('copy finds the latest reply beyond a newest user-only window without reading all history', async () => {
  const read = spyOn(ipcBridge.database.getConversationMessages, 'invoke')
    .mockResolvedValueOnce({ items: [message(5), message(3)], total: 6, has_more: true, next_cursor: `1003:${conversationId}` })
    .mockResolvedValueOnce({ items: [message(2), message(1)], total: 6, has_more: true, next_cursor: `1001:${conversationId}` });
  restores.push(() => read.mockRestore());
  const reply = await fetchLatestConversationReply(conversationId, () => true);
  expect(reply.at(-1)?.content).toEqual({ content: 'message-2' });
  expect(read).toHaveBeenCalledTimes(2);
  expect(read.mock.calls[0][0]).toMatchObject({ content_mode: 'full', page_size: 50 });
});

test('copy stops after target invalidation and fails closed on unreadable or repeating pages', async () => {
  let current = true;
  const read = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockImplementationOnce(async () => {
    current = false;
    return { items: [message(2)], total: 1, has_more: false };
  }).mockRejectedValueOnce(new Error('unreadable history'))
    .mockResolvedValue({ items: [], total: 1, has_more: true, next_cursor: `1003:${conversationId}` });
  restores.push(() => read.mockRestore());
  expect(await fetchLatestConversationReply(conversationId, () => current)).toEqual([]);
  await expect(fetchLatestConversationReply(conversationId, () => true)).rejects.toThrow('unreadable history');
  await expect(fetchLatestConversationReply(conversationId, () => true)).rejects.toThrow('did not advance');
});
