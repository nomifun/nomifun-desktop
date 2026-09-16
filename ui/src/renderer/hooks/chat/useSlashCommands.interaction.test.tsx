import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { ConversationId } from '@/common/types/ids';
import { useSlashCommands } from './useSlashCommands';

afterEach(() => { cleanup(); mock.restore(); });
const id = (value: string) => value as ConversationId;
const guide = [{ command: 'skill:acme.guide', description: 'Package guide' }];

test('cold conversation discovers commands and refreshes when runtime becomes active', async () => {
  const query = spyOn(ipcBridge.conversation.getSlashCommands, 'invoke').mockResolvedValue(guide);
  const view = renderHook(({ status }: { status: string | null }) =>
    useSlashCommands(id('cold-discovery'), { conversation_type: 'nomi', agentStatus: status }),
    { initialProps: { status: null as string | null } });
  await waitFor(() => expect(view.result.current[0]?.name).toBe('skill:acme.guide'));
  expect(query).toHaveBeenCalledTimes(1);
  query.mockResolvedValue([]);
  view.rerender({ status: 'pending' });
  await waitFor(() => expect(view.result.current).toEqual([]));
  expect(query).toHaveBeenCalledTimes(2);
});

test('switching cold conversations clears previous commands and ignores an old response', async () => {
  let resolveOld!: (value: typeof guide) => void;
  const query = spyOn(ipcBridge.conversation.getSlashCommands, 'invoke')
    .mockImplementation(({ conversation_id }) => conversation_id === id('cold-old')
      ? new Promise(resolve => { resolveOld = resolve; }) : Promise.resolve([]));
  const view = renderHook(({ conversation }) =>
    useSlashCommands(conversation, { conversation_type: 'nomi' }),
    { initialProps: { conversation: id('cold-old') } });
  view.rerender({ conversation: id('cold-new') });
  await act(async () => { resolveOld(guide); });
  expect(view.result.current).toEqual([]);
  expect(query).toHaveBeenCalledTimes(2);
});

test('empty discovery removes cached commands before the next mount', async () => {
  const query = spyOn(ipcBridge.conversation.getSlashCommands, 'invoke').mockResolvedValue(guide);
  const hook = () => useSlashCommands(id('cold-cache'), { conversation_type: 'nomi' });
  const first = renderHook(hook);
  await waitFor(() => expect(first.result.current).toHaveLength(1));
  first.unmount();
  query.mockResolvedValue([]);
  const second = renderHook(hook);
  await waitFor(() => expect(second.result.current).toEqual([]));
  second.unmount();
  query.mockImplementation(() => new Promise(() => {}));
  const third = renderHook(hook);
  expect(third.result.current).toEqual([]);
});

test('non-Nomi conversations do not query the command catalog', () => {
  const query = spyOn(ipcBridge.conversation.getSlashCommands, 'invoke').mockResolvedValue(guide);
  const view = renderHook(() => useSlashCommands(id('other-runtime'), { conversation_type: 'other' }));
  expect(view.result.current).toEqual([]);
  expect(query).not.toHaveBeenCalled();
});
