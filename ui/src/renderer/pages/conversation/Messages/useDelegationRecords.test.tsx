import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import { database } from '@/common/adapter/ipcBridge';
import { parseConversationId, parseExecutionId, parseMessageId } from '@/common/types/ids';
import type { DelegationRecordsPage } from '@/common/chat/delegationRecords';
import { executionId, leadConversationId, makeDelegationRecord, requestId } from '../../../../../test/fixtures/conversationDelegation';
import { useDelegationRecords } from './useDelegationRecords';

let changed: (execution?: typeof executionId) => void;
let restore = () => {};
beforeEach(() => {
  const change = spyOn(ipcBridge.agentExecution.events.changed, 'on').mockImplementation(listener => {
    changed = (id = executionId) => listener({ execution_id: id, sequence: 1 } as Parameters<typeof listener>[0]);
    return () => {};
  });
  const response = spyOn(ipcBridge.conversation.responseStream, 'on').mockReturnValue(() => {});
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  restore = () => { change.mockRestore(); response.mockRestore(); reconnect.mockRestore(); };
});
afterEach(() => { cleanup(); restore(); });

test('same parent head accepts later execution completions without an incorrect Session watermark', async () => {
  let records = [makeDelegationRecord()];
  const read = spyOn(database.getDelegationRecords, 'invoke').mockImplementation(async () => ({ records, through_session_seq: 100 }));
  try {
    const hook = renderHook(({ running }) => useDelegationRecords(leadConversationId, [requestId], running, executionId), { initialProps: { running: false } });
    await waitFor(() => expect(hook.result.current.records).toHaveLength(1));
    hook.rerender({ running: true });
    expect(read).toHaveBeenCalledTimes(1);
    records = [...records, makeDelegationRecord({ id: 'completed', kind: 'completed', execution_event_sequence: 2 })];
    act(() => changed());
    await waitFor(() => expect(hook.result.current.records).toHaveLength(2));
    expect(read.mock.calls[1][0]).toMatchObject({ source_message_ids: requestId });
    expect(read.mock.calls[1][0]).not.toHaveProperty('after_session_seq');
    act(() => changed(parseExecutionId('00000000-07d0-7000-8000-000000000099')));
    expect(read).toHaveBeenCalledTimes(2);
  } finally { read.mockRestore(); }
});

test('latest records are shown first, more uses opaque snapshot pages, and refresh preserves loaded depth', async () => {
  const first = makeDelegationRecord({ id: 'done', kind: 'completed', execution_event_sequence: 2 });
  const read = spyOn(database.getDelegationRecords, 'invoke').mockImplementation(async request => request.cursor
    ? { records: [makeDelegationRecord()], through_session_seq: 100 }
    : { records: [first], through_session_seq: 100, next_cursor: 'snapshot-fence' });
  try {
    const hook = renderHook(() => useDelegationRecords(leadConversationId, [requestId], false, executionId));
    await waitFor(() => expect(hook.result.current.hasMore).toBe(true));
    expect(hook.result.current.records[0].kind).toBe('completed');
    act(() => hook.result.current.loadMore());
    await waitFor(() => expect(hook.result.current.records).toHaveLength(2));
    expect(hook.result.current.hasMore).toBe(false);
    expect(read).toHaveBeenCalledTimes(2);
    expect(read.mock.calls[1][0].cursor).toBe('snapshot-fence');
    act(() => changed());
    await waitFor(() => expect(read).toHaveBeenCalledTimes(4));
    expect(hook.result.current.records).toHaveLength(2);
  } finally { read.mockRestore(); }
});

test('a late prior-Session response cannot replace the newly selected exact source window', async () => {
  const otherSession = parseConversationId('00000000-03e8-7000-8000-000000000091');
  const otherRoot = parseMessageId('00000000-03e8-7000-8000-000000000092');
  let release!: (page: DelegationRecordsPage) => void;
  const original = makeDelegationRecord();
  const other = makeDelegationRecord({ id: 'other', source: { ...original.source, agent_session_id: otherSession, source_message_id: otherRoot },
    anchor: { ...original.anchor, agent_session_id: otherSession } });
  const read = spyOn(database.getDelegationRecords, 'invoke').mockImplementation(request => request.conversation_id === leadConversationId
    ? new Promise(resolve => { release = resolve; }) : Promise.resolve({ records: [other], through_session_seq: 100 }));
  try {
    const hook = renderHook(({ session, root }) => useDelegationRecords(session, [root], false), { initialProps: { session: leadConversationId, root: requestId } });
    hook.rerender({ session: otherSession, root: otherRoot });
    await waitFor(() => expect(hook.result.current.records[0]?.id).toBe('other'));
    await act(async () => release({ records: [original], through_session_seq: 100 }));
    expect(hook.result.current.records[0]?.id).toBe('other');
  } finally { read.mockRestore(); }
});

test('a failed refresh retains confirmed records and explicit retry actually starts a new read', async () => {
  const read = spyOn(database.getDelegationRecords, 'invoke').mockResolvedValue({ records: [makeDelegationRecord()], through_session_seq: 100 });
  try {
    const hook = renderHook(() => useDelegationRecords(leadConversationId, [requestId], false, executionId));
    await waitFor(() => expect(hook.result.current.records).toHaveLength(1));
    read.mockRejectedValue(new Error('Temporarily unavailable'));
    act(() => changed());
    await waitFor(() => expect(hook.result.current.failed).toBe(true));
    expect(hook.result.current.records).toHaveLength(1);
    read.mockResolvedValue({ records: [makeDelegationRecord({ kind: 'completed' })], through_session_seq: 100 });
    act(() => hook.result.current.retry());
    await waitFor(() => expect(hook.result.current.failed).toBe(false));
    expect(hook.result.current.records[0].kind).toBe('completed');
  } finally { read.mockRestore(); }
});
