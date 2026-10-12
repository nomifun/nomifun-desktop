import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { ipcBridge } from '@/common';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import type { TurnFileSummary } from '@/common/types/platform/turnFiles';
import { emitter } from '@/renderer/utils/emitter';
import { useTurnFiles } from './useTurnFiles';

const a = parseConversationId('0190f5fe-7c00-7a00-8000-000000000051');
const b = parseConversationId('0190f5fe-7c00-7a00-8000-000000000052');
const turn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000001');
const turn2 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000002');
afterEach(() => { cleanup(); mock.restore(); });

const summary = (id = turn): TurnFileSummary => ({ turn_id: id, complete: true, files: [
  { identity: 'workspace:doc', relative_path: 'report.docx', file_name: 'report.docx', operation: 'create' },
] });

function transport() {
  const requests: Array<{ query: Parameters<typeof ipcBridge.conversation.turnFiles.invoke>[0]; resolve: (data: TurnFileSummary[]) => void }> = [];
  spyOn(ipcBridge.conversation.turnFiles, 'invoke').mockImplementation(query => new Promise(resolve => requests.push({ query, resolve })));
  const completed = new Set<Parameters<typeof ipcBridge.conversation.turnCompleted.on>[0]>();
  spyOn(ipcBridge.conversation.turnCompleted, 'on').mockImplementation(listener => {
    completed.add(listener); return () => { completed.delete(listener); };
  });
  const reconnects = new Set<() => void>();
  spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(listener => {
    reconnects.add(listener); return () => { reconnects.delete(listener); };
  });
  const reply = async (index: number, data = [summary()]) => {
    expect(requests[index]).toBeDefined();
    await act(async () => { requests[index].resolve(data); });
  };
  return { requests, completed, reconnects, reply };
}

test('loads closed Turn files without reading message pages and avoids identical-scope refetches', async () => {
  const t = transport();
  const history = spyOn(ipcBridge.database.getConversationMessages, 'invoke');
  const hook = renderHook(({ ids }) => useTurnFiles(a, ids), { initialProps: { ids: [turn] } });
  await t.reply(0);
  expect(hook.result.current.get(turn)?.files[0].file_name).toBe('report.docx');
  hook.rerender({ ids: [turn] });
  expect(t.requests).toHaveLength(1);
  expect(t.requests[0].query).toEqual({ conversation_id: a, turn_ids: [turn] });
  expect(history).not.toHaveBeenCalled();
});

test('running and resumed Turns do not query or render closed file summaries', async () => {
  const t = transport();
  const hook = renderHook(({ ids }) => useTurnFiles(a, ids), { initialProps: { ids: [] as typeof turn[] } });
  expect(t.requests).toHaveLength(0);
  hook.rerender({ ids: [turn] });
  await t.reply(0);
  expect(hook.result.current.size).toBe(1);
  hook.rerender({ ids: [] });
  expect(hook.result.current.size).toBe(0);
});

test('settlement and reconnect refresh canonical data, coalesce reads, and ignore other sessions', async () => {
  const t = transport();
  const hook = renderHook(() => useTurnFiles(a, [turn]));
  act(() => { emitter.emit('conversation.turn.settled', b); });
  expect(t.requests).toHaveLength(1);
  act(() => {
    emitter.emit('conversation.turn.settled', a);
    emitter.emit('conversation.turn.settled', a);
  });
  await t.reply(0);
  expect(t.requests).toHaveLength(2);
  await t.reply(1, [{ ...summary(), complete: false }]);
  expect(hook.result.current.get(turn)?.complete).toBe(false);
  act(() => { t.reconnects.forEach(listener => listener()); });
  await t.reply(2);
  expect(hook.result.current.get(turn)?.complete).toBe(true);
  hook.unmount();
  expect(t.completed.size).toBe(0);
  expect(t.reconnects.size).toBe(0);
});

test('conversation navigation and changed Turn scope fence late responses', async () => {
  const t = transport();
  const hook = renderHook(({ id, ids }) => useTurnFiles(id, ids), { initialProps: { id: a, ids: [turn] } });
  hook.rerender({ id: b, ids: [turn2] });
  await t.reply(1, [summary(turn2)]);
  await t.reply(0);
  expect(hook.result.current.has(turn2)).toBe(true);
  expect(hook.result.current.has(turn)).toBe(false);
  hook.rerender({ id: a, ids: [turn] });
  expect(hook.result.current.size).toBe(0);
  await t.reply(2);
  expect(hook.result.current.has(turn)).toBe(true);
});

test('loading another history page retains current cards while querying complete data for added Turns', async () => {
  const t = transport();
  const hook = renderHook(({ ids }) => useTurnFiles(a, ids), { initialProps: { ids: [turn] } });
  await t.reply(0);
  hook.rerender({ ids: [turn2, turn] });
  expect(hook.result.current.get(turn)?.files).toHaveLength(1);
  await t.reply(1, [summary(), summary(turn2)]);
  expect(hook.result.current.size).toBe(2);
  expect(t.requests[1].query.turn_ids).toEqual([turn, turn2]);
});

test('more than 100 closed roots use bounded endpoint batches', async () => {
  const t = transport();
  const ids = Array.from({ length: 101 }, (_, index) => parseMessageId(`0190f5fe-7c00-7a00-8000-${String(index + 1).padStart(12, '0')}`));
  const hook = renderHook(() => useTurnFiles(a, ids));
  expect(t.requests[0].query.turn_ids).toHaveLength(100);
  await t.reply(0, ids.slice(0, 100).map(id => summary(id)));
  expect(t.requests[1].query.turn_ids).toEqual(ids.slice(100));
  await t.reply(1, ids.slice(100).map(id => summary(id)));
  expect(hook.result.current.size).toBe(101);
});

test('resuming and reclosing the same root never reuses its pre-resume file snapshot', async () => {
  const t = transport();
  const hook = renderHook(({ ids }) => useTurnFiles(a, ids), { initialProps: { ids: [turn] } });
  await t.reply(0);
  hook.rerender({ ids: [] });
  hook.rerender({ ids: [turn] });
  expect(hook.result.current.size).toBe(0);
  await t.reply(1, [{ ...summary(), files: [] }]);
  expect(hook.result.current.get(turn)?.files).toHaveLength(0);
});
