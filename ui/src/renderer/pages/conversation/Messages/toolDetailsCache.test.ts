import { expect, mock, test } from 'bun:test';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { toolDetailIdentityKey, toolDetailSnapshotKey, type ToolDetailPage, type ToolDetailRequest } from '@/common/chat/toolDetails';
import { ToolDetailsCache } from './toolDetailsCache';

const source = { conversation_id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000011'), turn_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000012'), call_id: 'command', through_seq: 50, expected_result_seq: 42 };
const response = (request: ToolDetailRequest): ToolDetailPage => ({ turn_id: request.turn_id, call_id: request.call_id,
  call_event_id: 'canonical-call', through_seq: 50, result_seq: 42, section: request.section,
  availability: 'available', content: '', format: 'text', offset: 0, total_bytes: 0, eof: true, non_text_parts: 0 });

test('deduplicates concurrent exact pages, bounds retained pages, and clears viewer data', async () => {
  const read = mock(async (request: ToolDetailRequest) => response(request));
  const cache = new ToolDetailsCache(read);
  await Promise.all([cache.get({ ...source, section: 'output' }), cache.get({ ...source, section: 'output' })]);
  expect(read).toHaveBeenCalledTimes(1);
  for (let index = 0; index < 33; index += 1) await cache.get({ ...source, section: 'output', cursor: `page-${index}` });
  await cache.get({ ...source, section: 'output' });
  expect(read).toHaveBeenCalledTimes(35);
  cache.clear();
  await cache.get({ ...source, section: 'output' });
  expect(read).toHaveBeenCalledTimes(36);
});

test('does not cache pending observations or an identity mismatch', async () => {
  const pending = mock(async (request: ToolDetailRequest) => ({ ...response(request), availability: 'pending' as const }));
  const cache = new ToolDetailsCache(pending);
  await cache.get({ ...source, section: 'output' });
  await cache.get({ ...source, section: 'output' });
  expect(pending).toHaveBeenCalledTimes(2);
  const wrong = new ToolDetailsCache(async request => ({ ...response(request), call_id: 'different-call' }));
  await expect(wrong.get({ ...source, section: 'output' })).rejects.toThrow('identity');
});

test('stable result identity does not merge independently requested read snapshots', async () => {
  const laterHead = { ...source, through_seq: 60 };
  expect(toolDetailIdentityKey(laterHead)).toBe(toolDetailIdentityKey(source));
  expect(toolDetailSnapshotKey(laterHead)).not.toBe(toolDetailSnapshotKey(source));
  const read = mock(async (request: ToolDetailRequest) => ({ ...response(request), through_seq: request.through_seq! }));
  const cache = new ToolDetailsCache(read);
  await cache.get({ ...source, section: 'output' });
  await cache.get({ ...laterHead, section: 'output' });
  expect(read).toHaveBeenCalledTimes(2);
});
