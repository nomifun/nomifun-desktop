import { afterEach, expect, jest, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { ipcBridge } from '@/common';
import { parseConversationId, parseMessageId, type ConversationId } from '@/common/types/ids';
import type { IResponseMessage } from '@/common/adapter/ipcBridge';
import type { TaskPlanSnapshot } from '@/common/protocolBindings/TaskPlanSnapshot';
import { emitter } from '@/renderer/utils/emitter';
import { useConversationTaskPlan } from './useConversationTaskPlan';

const a = parseConversationId('0190f5fe-7c00-7a00-8000-000000000051');
const b = parseConversationId('0190f5fe-7c00-7a00-8000-000000000052');
afterEach(() => { cleanup(); jest.useRealTimers(); mock.restore(); });

function snapshot(sequence = 1, id: ConversationId = a, plan = true): TaskPlanSnapshot {
  return {
    conversation_id: id, sequence, turn_id: '0190f5fe-7c00-7a00-8000-000000000053', turn_status: 'running',
    plan: plan ? { revision: 1, explanation: '', needs_replan: false,
      steps: [{ step: 'Inspect', status: 'in_progress' }, { step: 'Verify', status: 'pending' }] } : null,
  };
}

function transport() {
  const requests: Array<{ id: ConversationId; resolve: (data: TaskPlanSnapshot) => void; reject: (error: Error) => void }> = [];
  spyOn(ipcBridge.conversation.taskPlan, 'invoke').mockImplementation((query) =>
    new Promise<TaskPlanSnapshot>((resolve, reject) => requests.push({ id: query.conversation_id, resolve, reject })));
  const streams = new Set<(event: IResponseMessage) => void>();
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation((listener) => {
    streams.add(listener); return () => { streams.delete(listener); };
  });
  const reconnects = new Set<() => void>();
  spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation((listener) => {
    reconnects.add(listener); return () => { reconnects.delete(listener); };
  });
  spyOn(ipcBridge.conversation.turnStarted, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.conversation.turnCompleted, 'on').mockImplementation(() => () => {});
  const reply = async (index: number, data: TaskPlanSnapshot) => {
    expect(requests[index]).toBeDefined();
    await act(async () => { requests[index].resolve(data); });
  };
  const signal = (id = a) => act(() => { streams.forEach((listener) => listener({
    conversation_id: id, msg_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000053'),
    type: 'task_plan_changed', data: {},
  })); });
  const reconnect = () => act(() => { reconnects.forEach((listener) => listener()); });
  return { requests, reply, signal, reconnect, streams, reconnects };
}

test('mount and realtime plan notifications load current progress without message history', async () => {
  const t = transport();
  const history = spyOn(ipcBridge.database.getConversationMessages, 'invoke');
  const hook = renderHook(() => useConversationTaskPlan(a));
  await t.reply(0, snapshot(100));
  expect(hook.result.current?.plan?.steps).toHaveLength(2);
  t.signal();
  const updated = snapshot(102);
  updated.plan!.steps[0].status = 'completed';
  await t.reply(1, updated);
  expect(hook.result.current?.plan?.steps[0].status).toBe('completed');
  expect(history).not.toHaveBeenCalled();
});

test('notifications while a read is pending coalesce and an older snapshot cannot roll back progress', async () => {
  const t = transport();
  const hook = renderHook(() => useConversationTaskPlan(a));
  t.signal(); t.signal(); t.signal(b);
  expect(t.requests).toHaveLength(1);
  await t.reply(0, snapshot(20));
  expect(t.requests).toHaveLength(2);
  await t.reply(1, snapshot(10));
  expect(hook.result.current?.sequence).toBe(20);
  expect(t.requests).toHaveLength(2);
});

test('A to B to A navigation fences earlier reads even when the old A response arrives last', async () => {
  const t = transport();
  const hook = renderHook(({ id }) => useConversationTaskPlan(id), { initialProps: { id: a } });
  hook.rerender({ id: b });
  await t.reply(1, snapshot(30, b));
  hook.rerender({ id: a });
  expect(hook.result.current).toBeNull();
  await t.reply(2, snapshot(40));
  await t.reply(0, snapshot(999));
  expect(hook.result.current?.sequence).toBe(40);
  hook.unmount();
  expect(t.streams.size).toBe(0);
  expect(t.reconnects.size).toBe(0);
});

test('reconnect and settled reads preserve lifecycle and clear an old plan for a new task', async () => {
  const t = transport();
  const hook = renderHook(() => useConversationTaskPlan(a));
  await t.reply(0, snapshot(10));
  t.reconnect();
  await t.reply(1, { ...snapshot(11), turn_status: 'paused' });
  expect(hook.result.current?.turn_status).toBe('paused');
  act(() => { emitter.emit('conversation.turn.settled', a); });
  const reset = snapshot(12, a, false);
  reset.turn_id = '0190f5fe-7c00-7a00-8000-000000000054';
  await t.reply(2, reset);
  expect(hook.result.current?.plan).toBeNull();
  expect(hook.result.current?.turn_id).toBe(reset.turn_id);
});

test('failed reads preserve the last known progress and later signals recover', async () => {
  const t = transport();
  spyOn(console, 'warn').mockImplementation(() => {});
  const hook = renderHook(() => useConversationTaskPlan(a));
  await t.reply(0, snapshot(10));
  t.signal();
  await act(async () => { t.requests[1].reject(new Error('Disconnected')); });
  expect(hook.result.current?.sequence).toBe(10);
  t.signal();
  await t.reply(2, snapshot(12));
  expect(hook.result.current?.sequence).toBe(12);
});

test('polling repairs a dropped notification and timed-out reads cannot strand progress', async () => {
  jest.useFakeTimers();
  const t = transport();
  spyOn(console, 'warn').mockImplementation(() => {});
  const hook = renderHook(() => useConversationTaskPlan(a));
  await t.reply(0, snapshot(10));
  act(() => { jest.advanceTimersByTime(4_000); });
  expect(t.requests).toHaveLength(2);
  await act(async () => { jest.advanceTimersByTime(4_000); });
  expect(hook.result.current?.sequence).toBe(10);
  act(() => { jest.advanceTimersByTime(3_000); });
  expect(t.requests).toHaveLength(3);
  await t.reply(2, snapshot(20));
  await t.reply(1, snapshot(999));
  expect(hook.result.current?.sequence).toBe(20);
});
