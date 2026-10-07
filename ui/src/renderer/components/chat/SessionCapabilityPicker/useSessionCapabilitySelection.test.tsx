import '../../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { AgentSessionCapabilitySelectionState } from '@/common/types/agentPlatform';
import { useSessionCapabilitySelection } from './useSessionCapabilitySelection';
import { defaultSessionCapabilityDraft, toSessionCapabilitySelection } from './model';

const sessionId = '019b0000-0000-7000-8000-000000000002';
const serverId = '019b0000-0000-7000-8000-000000000003';
const initial: AgentSessionCapabilitySelectionState = {
  selection: { skill_names: ['auto-skill'], mcp_server_ids: [serverId] },
  binding_version: 4,
  editable: true,
};
const restores: Array<() => void> = [];
const track = <T extends { mockRestore: () => void }>(spy: T) => { restores.push(() => spy.mockRestore()); return spy; };
let changed: (event: any) => void;
let switched: (event: any) => void;
beforeEach(() => {
  track(spyOn(ipcBridge.agentPlatform.sessions.onAgentChanged, 'on').mockImplementation((handler) => { switched = handler; return () => {}; }));
  track(spyOn(ipcBridge.agentPlatform.sessions.onCapabilitiesChanged, 'on').mockImplementation((handler) => { changed = handler; return () => {}; }));
});
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

test('loads canonical selection once, admits a draft before send, then reuses the committed revision', async () => {
  const get = track(spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue(initial));
  const next = { ...initial, binding_version: 5, selection: { skill_names: ['optional'], mcp_server_ids: [] } };
  const put = track(spyOn(ipcBridge.agentPlatform.sessions.putCapabilitySelection, 'invoke').mockResolvedValue(next));
  const hook = renderHook(() => useSessionCapabilitySelection(sessionId));
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  expect(hook.result.current.draft.skillNames).toEqual(['auto-skill']);
  act(() => hook.result.current.setDraft({ skillNames: ['optional'], mcpServerIds: [] }));
  let selected;
  await act(async () => { selected = await hook.result.current.applyBeforeSend(); });
  expect(selected).toEqual(next.selection);
  expect(put.mock.calls[0]?.[0]).toEqual({ agent_session_id: sessionId, expected_binding_version: 4, selection: next.selection });
  hook.rerender();
  await act(async () => { await hook.result.current.applyBeforeSend(); });
  expect(put).toHaveBeenCalledTimes(1);
  expect(get).toHaveBeenCalledTimes(1);
});

test('failed selection admission prevents sending and retains the local draft', async () => {
  track(spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue(initial));
  const failure = new Error('The selected MCP server is no longer available');
  track(spyOn(ipcBridge.agentPlatform.sessions.putCapabilitySelection, 'invoke').mockRejectedValue(failure));
  const send = mock(async () => {});
  const hook = renderHook(() => useSessionCapabilitySelection(sessionId));
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  act(() => hook.result.current.setDraft({ skillNames: [], mcpServerIds: [] }));
  await act(async () => {
    await expect((async () => { await hook.result.current.applyBeforeSend(); await send(); })()).rejects.toBe(failure);
  });
  expect(send).not.toHaveBeenCalled();
  expect(hook.result.current.error?.message).toBe(failure.message);
  expect(hook.result.current.draft).toEqual({ skillNames: [], mcpServerIds: [] });
});

test('a remote or active session cannot admit changed selection', async () => {
  track(spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue({ ...initial, editable: false }));
  const put = track(spyOn(ipcBridge.agentPlatform.sessions.putCapabilitySelection, 'invoke').mockResolvedValue(initial));
  const hook = renderHook(() => useSessionCapabilitySelection(sessionId));
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  expect(await hook.result.current.applyBeforeSend()).toEqual(initial.selection);
  act(() => hook.result.current.setDraft({ skillNames: [], mcpServerIds: [] }));
  await expect(hook.result.current.applyBeforeSend()).rejects.toThrow('read-only');
  expect(put).not.toHaveBeenCalled();
});

test('selection events update canonical state without polling and Agent switches reload it', async () => {
  const get = track(spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue(initial));
  const hook = renderHook(() => useSessionCapabilitySelection(sessionId));
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  const selection = { skill_names: [], mcp_server_ids: [] };
  act(() => changed({ agent_session_id: 'another-session', selection, binding_version: 7 }));
  expect(hook.result.current.draft.skillNames).toEqual(['auto-skill']);
  act(() => changed({ agent_session_id: sessionId, selection, binding_version: 7 }));
  expect(hook.result.current.draft.skillNames).toEqual([]);
  expect(get).toHaveBeenCalledTimes(1);
  get.mockResolvedValue({ ...initial, selection, binding_version: 8 });
  act(() => switched({ agent_session_id: sessionId }));
  await waitFor(() => expect(hook.result.current.state?.binding_version).toBe(8));
  expect(get).toHaveBeenCalledTimes(2);
});

test('new-session defaults choose auto Skills and only enabled ready MCP; explicit empty means none', () => {
  const skill = (name: string, auto: boolean) => ({ name, auto, source: 'builtin', is_custom: false, location: '', description: '' } as const);
  const server = (mcp_server_id: string, enabled: boolean, tools: unknown[], last_test_status?: string) => ({ mcp_server_id, enabled, tools, last_test_status } as any);
  const defaults = defaultSessionCapabilityDraft({ skills: [skill('auto', true), skill('optional', false), { ...skill('bad-auto', true), session_available: false, session_error: 'Invalid resource' }], autoSkillNames: new Set(['auto']), mcpServers: [
    server('ready', true, [{}], 'connected'), server('unverified', true, [{}]), server('disabled', false, [{}]), server('empty', true, []), server('failed', true, [{}], 'error'),
  ] });
  expect(defaults).toEqual({ skillNames: ['auto'], mcpServerIds: ['ready'] });
  expect(toSessionCapabilitySelection({ skillNames: [], mcpServerIds: [] })).toEqual({ skill_names: [], mcp_server_ids: [] });
});

test('a failed load shows an error and retry can recover without a polling loop', async () => {
  const get = track(spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockRejectedValue(new Error('Connection lost')));
  const hook = renderHook(() => useSessionCapabilitySelection(sessionId));
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  expect(hook.result.current.error?.message).toBe('Connection lost');
  expect(hook.result.current.state).toBeUndefined();
  get.mockResolvedValue(initial);
  act(() => hook.result.current.retry());
  await waitFor(() => expect(hook.result.current.state?.binding_version).toBe(4));
  expect(get).toHaveBeenCalledTimes(2);
});

test('changing conversations never lends the previous session selection to a new send', async () => {
  const get = track(spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue(initial));
  const hook = renderHook(({ id }) => useSessionCapabilitySelection(id), { initialProps: { id: sessionId } });
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  let resolve!: (state: AgentSessionCapabilitySelectionState) => void;
  get.mockImplementation(() => new Promise((reply) => { resolve = reply; }));
  hook.rerender({ id: '019b0000-0000-7000-8000-000000000004' });
  expect(hook.result.current.state).toBeUndefined();
  await expect(hook.result.current.applyBeforeSend()).rejects.toThrow('loading');
  await act(async () => resolve({ ...initial, selection: { skill_names: [], mcp_server_ids: [] } }));
  expect(hook.result.current.draft.skillNames).toEqual([]);
});

test('retry reconciles the revision while preserving an unsubmitted capability draft', async () => {
  const get = track(spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue(initial));
  const hook = renderHook(() => useSessionCapabilitySelection(sessionId));
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  act(() => hook.result.current.setDraft({ skillNames: ['optional'], mcpServerIds: [] }));
  get.mockResolvedValue({ ...initial, binding_version: 9 });
  act(() => hook.result.current.retry());
  await waitFor(() => expect(hook.result.current.state?.binding_version).toBe(9));
  expect(hook.result.current.draft).toEqual({ skillNames: ['optional'], mcpServerIds: [] });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((reply) => { resolve = reply; });
  return { promise, resolve };
}

test('batched events stay monotonic and a late initial GET cannot replace the newest selection', async () => {
  const read = deferred<AgentSessionCapabilitySelectionState>();
  const get = track(spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockReturnValue(read.promise));
  const put = track(spyOn(ipcBridge.agentPlatform.sessions.putCapabilitySelection, 'invoke').mockResolvedValue(initial));
  const hook = renderHook(() => useSessionCapabilitySelection(sessionId));
  const newest = { skill_names: ['latest'], mcp_server_ids: [] };
  act(() => {
    changed({ agent_session_id: sessionId, selection: newest, binding_version: 9, editable: false });
    changed({ agent_session_id: sessionId, selection: initial.selection, binding_version: 8, editable: true });
  });
  await act(async () => { read.resolve(initial); });
  expect(hook.result.current.state).toEqual({ selection: newest, binding_version: 9, editable: false });
  expect(hook.result.current.draft.skillNames).toEqual(['latest']);
  act(() => hook.result.current.setDraft({ skillNames: [], mcpServerIds: [] }));
  await expect(hook.result.current.applyBeforeSend()).rejects.toThrow('read-only');
  expect(get).toHaveBeenCalledTimes(1);
  expect(put).not.toHaveBeenCalled();
});

test('a refresh GET started before a PUT receipt cannot roll back that committed selection', async () => {
  const read = deferred<AgentSessionCapabilitySelectionState>();
  const write = deferred<AgentSessionCapabilitySelectionState>();
  const get = track(spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue(initial));
  track(spyOn(ipcBridge.agentPlatform.sessions.putCapabilitySelection, 'invoke').mockReturnValue(write.promise));
  const hook = renderHook(() => useSessionCapabilitySelection(sessionId));
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  act(() => hook.result.current.setDraft({ skillNames: ['optional'], mcpServerIds: [] }));
  let admission!: Promise<unknown>;
  act(() => { admission = hook.result.current.applyBeforeSend(); });
  get.mockReturnValue(read.promise);
  act(() => switched({ agent_session_id: sessionId }));
  await waitFor(() => expect(get).toHaveBeenCalledTimes(2));
  const committed = { ...initial, binding_version: 5, selection: { skill_names: ['optional'], mcp_server_ids: [] } };
  await act(async () => { write.resolve(committed); await admission; });
  await act(async () => { read.resolve(initial); });
  expect(hook.result.current.state).toEqual(committed);
  expect(hook.result.current.draft.skillNames).toEqual(['optional']);
  expect(hook.result.current.loading).toBe(false);
  expect(get).toHaveBeenCalledTimes(2);
});

test('a delayed PUT receipt cannot authorize a send after a newer selection event', async () => {
  const write = deferred<AgentSessionCapabilitySelectionState>();
  track(spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue(initial));
  track(spyOn(ipcBridge.agentPlatform.sessions.putCapabilitySelection, 'invoke').mockReturnValue(write.promise));
  const hook = renderHook(() => useSessionCapabilitySelection(sessionId));
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  act(() => hook.result.current.setDraft({ skillNames: ['optional'], mcpServerIds: [] }));
  let admission!: Promise<unknown>;
  act(() => { admission = hook.result.current.applyBeforeSend(); });
  const newest = { skill_names: [], mcp_server_ids: [] };
  act(() => changed({ agent_session_id: sessionId, selection: newest, binding_version: 6, editable: true }));
  await act(async () => {
    write.resolve({ ...initial, binding_version: 5, selection: { skill_names: ['optional'], mcp_server_ids: [] } });
    await expect(admission).rejects.toThrow('changed while applying');
  });
  expect(hook.result.current.state?.binding_version).toBe(6);
  expect(hook.result.current.state?.selection).toEqual(newest);
});

test('late callbacks from the prior Session cannot overwrite a new Session with a lower version', async () => {
  const first = deferred<AgentSessionCapabilitySelectionState>();
  const second = deferred<AgentSessionCapabilitySelectionState>();
  const otherId = '019b0000-0000-7000-8000-000000000004';
  track(spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockImplementation(({ agent_session_id }) =>
    agent_session_id === sessionId ? first.promise : second.promise));
  const hook = renderHook(({ id }) => useSessionCapabilitySelection(id), { initialProps: { id: sessionId } });
  const oldEventHandler = changed;
  hook.rerender({ id: otherId });
  const next = { ...initial, binding_version: 2, selection: { skill_names: ['other-session'], mcp_server_ids: [] } };
  act(() => oldEventHandler({ agent_session_id: sessionId, selection: initial.selection, binding_version: 100, editable: false }));
  await act(async () => { second.resolve(next); first.resolve({ ...initial, binding_version: 100 }); });
  expect(hook.result.current.state).toEqual(next);
  expect(hook.result.current.draft.skillNames).toEqual(['other-session']);
});
