import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import { fromApiTurnCompletedEvent, type IConversationTurnStartedEvent } from '@/common/adapter/ipcBridge';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { AppUpdateResponse, HostedApp } from '@/common/types/appHosting';
import { useAppDevelopmentSync } from './useAppDevelopmentSync';

const session = '0190f5fe-7c00-7a00-8000-000000000611';
const turn = '0190f5fe-7c00-7a00-8000-000000000612';
const app: HostedApp = { id: 'application', name: 'New application', description: '', tags: [], development_session_id: session,
  directory: 'C:/apps/application', import_kind: 'source', settings: { project_directory: '.', environment: {} },
  state: 'stopped', operation: null, last_error: null, output: '', pid: null, preview: false, prepared: false };
const completed = (extra = {}) => fromApiTurnCompletedEvent({ conversation_id: session, turn_id: turn, state: 'ai_waiting_input',
  runtime: { state: 'idle', is_processing: false, can_send_message: true }, ...extra });
let emitComplete: (event: ReturnType<typeof completed>) => void;
let emitStarted: (event: IConversationTurnStartedEvent) => void;
let emitPaused: (event: { conversation_id: string; turn_id: string }) => void;

beforeEach(() => {
  spyOn(ipcBridge.conversation.turnStarted, 'on').mockImplementation(handler => { emitStarted = handler; return () => {}; });
  spyOn(ipcBridge.conversation.listChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.conversation.turnCompleted, 'on').mockImplementation(handler => { emitComplete = handler; return () => {}; });
  spyOn(ipcBridge.conversation.turnPaused, 'on').mockImplementation(handler => { emitPaused = handler as typeof emitPaused; return () => {}; });
});
afterEach(() => { cleanup(); mock.restore(); });
const started = () => ({ conversation_id: completed().conversation_id, turn_id: completed().turn_id! }) as IConversationTurnStartedEvent;

test('a completed ordinary application Turn fills configuration once and never starts or creates an application', async () => {
  let finish!: (value: HostedApp) => void;
  const update = spyOn(appHosting.update, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const operate = spyOn(appHosting.operate, 'invoke');
  const create = spyOn(appHosting.create, 'invoke');
  const refresh = mock(async () => {}), accept = mock(async (_value: HostedApp) => {});
  const view = renderHook(() => useAppDevelopmentSync(app, refresh, accept));
  expect(update).not.toHaveBeenCalled();
  act(() => emitComplete(completed()));
  act(() => emitComplete(completed()));
  expect(update.mock.calls).toEqual([[{ id: app.id, detect_settings: true, completion: { session_id: session, turn_id: turn } }]]);
  expect(view.result.current.configuring).toBe(true);
  const configured = { ...app, name: 'Snake game', settings: { project_directory: 'snake-game', environment: {}, start_command: 'node .nomifun-static-server.cjs' } };
  await act(async () => finish(configured));
  await waitFor(() => expect(view.result.current.configuring).toBe(false));
  expect(accept).toHaveBeenCalledWith(configured);
  expect(operate).not.toHaveBeenCalled(); expect(create).not.toHaveBeenCalled();
  expect(view.result.current.configurationError).toBe('');
});

test('mounts, owner polling, unrelated Turns, failures, pauses and notifications without exact identity are read-only', () => {
  const update = spyOn(appHosting.update, 'invoke');
  const refresh = mock(async () => {}), accept = mock(async (_value: HostedApp) => {});
  const view = renderHook(({ owner }) => useAppDevelopmentSync(owner, refresh, accept), { initialProps: { owner: app } });
  view.rerender({ owner: { ...app, name: 'Already generated in history' } });
  act(() => emitComplete(completed({ conversation_id: '0190f5fe-7c00-7a00-8000-000000000613' })));
  act(() => emitComplete(completed({ state: 'error' })));
  act(() => emitComplete(completed({ turn_id: undefined })));
  act(() => emitComplete(completed({ runtime: { state: 'idle', is_processing: true, can_send_message: true } })));
  act(() => emitPaused({ conversation_id: session, turn_id: turn }));
  expect(update).not.toHaveBeenCalled(); expect(accept).not.toHaveBeenCalled();
  expect(refresh).toHaveBeenCalledTimes(4);
  expect(view.result.current.configuring).toBe(false);
});

test('an input handoff only refreshes the owner and the next ordinary Turn can still discover settings', async () => {
  const update = spyOn(appHosting.update, 'invoke').mockResolvedValue(app);
  const refresh = mock(async () => {}), accept = mock(async (_value: HostedApp) => {});
  const view = renderHook(() => useAppDevelopmentSync(app, refresh, accept));
  act(() => emitComplete(completed({ stop_reason: 'input_required' })));
  expect(refresh).toHaveBeenCalledTimes(1);
  expect(update).not.toHaveBeenCalled();
  expect(accept).not.toHaveBeenCalled();
  expect(view.result.current.configuring).toBe(false);
  expect(view.result.current.configurationError).toBe('');
  expect(view.result.current.previewFileChange).toBeUndefined();

  const nextTurn = '0190f5fe-7c00-7a00-8000-000000000614';
  act(() => emitComplete(completed({ turn_id: nextTurn, stop_reason: 'end_turn' })));
  await waitFor(() => expect(view.result.current.configuring).toBe(false));
  expect(update.mock.calls).toEqual([[{ id: app.id, detect_settings: true, completion: { session_id: session, turn_id: nextTurn } }]]);
  expect(accept).toHaveBeenCalledWith(app);
  expect(view.result.current.configurationError).toBe('');
});

test('canonical owner rejection remains an explicit configuration error without retry or invented success', async () => {
  const update = spyOn(appHosting.update, 'invoke').mockRejectedValue(new Error('The completed Turn is no longer current'));
  const refresh = mock(async () => {}), accept = mock(async (_value: HostedApp) => {});
  const view = renderHook(() => useAppDevelopmentSync(app, refresh, accept));
  act(() => emitComplete(completed()));
  await waitFor(() => expect(view.result.current.configurationError).toBe('The completed Turn is no longer current'));
  expect(view.result.current.configuring).toBe(false);
  expect(update).toHaveBeenCalledTimes(1); expect(accept).not.toHaveBeenCalled();
});

test('a late configuration response cannot update another mounted application', async () => {
  let finish!: (value: HostedApp) => void;
  spyOn(appHosting.update, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const refresh = mock(async () => {}), accept = mock(async (_value: HostedApp) => {});
  const view = renderHook(({ owner }) => useAppDevelopmentSync(owner, refresh, accept), { initialProps: { owner: app } });
  act(() => emitComplete(completed()));
  view.rerender({ owner: { ...app, id: 'another-application', development_session_id: '0190f5fe-7c00-7a00-8000-000000000613' } });
  await act(async () => finish(app));
  expect(accept).not.toHaveBeenCalled(); expect(view.result.current.configuring).toBe(false);
});

test.each([true, false])('only the exact owner completion observation notifies a same-process file change (files_changed=%s)', async filesChanged => {
  const running = { ...app, state: 'running' as const, pid: 41, preview: true, settings: { ...app.settings, web_url: 'http://127.0.0.1:4317' } };
  spyOn(appHosting.update, 'invoke').mockResolvedValue({ ...running,
    completion_observation: { session_id: session, turn_id: turn, files_changed: filesChanged } });
  const refresh = mock(async () => {}), accept = mock(async (_value: HostedApp) => {});
  const view = renderHook(() => useAppDevelopmentSync(running, refresh, accept));
  act(() => emitStarted(started()));
  act(() => emitComplete(completed()));
  await waitFor(() => expect(view.result.current.configuring).toBe(false));
  expect(accept).toHaveBeenCalledWith(running);
  expect(accept.mock.calls[0][0]).not.toHaveProperty('completion_observation');
  expect(view.result.current.previewFileChange).toEqual(filesChanged
    ? { appId: app.id, sessionId: session, turnId: turn, pid: 41, url: running.settings.web_url } : undefined);
});

test('a late file observation does not request a second reload after the owner already restarted its process', async () => {
  const running = { ...app, state: 'running' as const, pid: 41, preview: true, settings: { ...app.settings, web_url: 'http://127.0.0.1:4317' } };
  let finish!: (value: AppUpdateResponse) => void;
  spyOn(appHosting.update, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const refresh = mock(async () => {}), accept = mock(async (_value: HostedApp) => {});
  const view = renderHook(({ owner }) => useAppDevelopmentSync(owner, refresh, accept), { initialProps: { owner: running } });
  act(() => emitStarted(started()));
  act(() => emitComplete(completed()));
  const restarted = { ...running, pid: 42 };
  view.rerender({ owner: restarted });
  await act(async () => finish({ ...restarted, completion_observation: { session_id: session, turn_id: turn, files_changed: true } }));
  expect(view.result.current.previewFileChange).toBeUndefined();
  expect(accept).toHaveBeenCalledWith(restarted);
});

test('an observation without an observed Turn start never guesses a preview baseline', async () => {
  spyOn(appHosting.update, 'invoke').mockResolvedValue({ ...app, completion_observation: { session_id: session, turn_id: turn, files_changed: true } });
  const refresh = mock(async () => {}), accept = mock(async (_value: HostedApp) => {});
  const view = renderHook(() => useAppDevelopmentSync(app, refresh, accept));
  act(() => emitComplete(completed()));
  await waitFor(() => expect(view.result.current.configuring).toBe(false));
  expect(view.result.current.previewFileChange).toBeUndefined();
});

test('a completion observation with a different Turn cannot mutate the accepted owner or reload the preview', async () => {
  spyOn(appHosting.update, 'invoke').mockResolvedValue({ ...app, completion_observation: { session_id: session,
    turn_id: '0190f5fe-7c00-7a00-8000-000000000613', files_changed: true } });
  const refresh = mock(async () => {}), accept = mock(async (_value: HostedApp) => {});
  const view = renderHook(() => useAppDevelopmentSync(app, refresh, accept));
  act(() => emitStarted(started())); act(() => emitComplete(completed()));
  await waitFor(() => expect(view.result.current.configurationError).toBe('Application configuration identity changed'));
  expect(accept).not.toHaveBeenCalled(); expect(view.result.current.previewFileChange).toBeUndefined();
});

test('a selected earlier project Session completes against the same app without borrowing the latest navigation reference', async () => {
  const running = { ...app, development_session_id: '0190f5fe-7c00-7a00-8000-000000000613', state: 'running' as const, pid: 41, preview: true,
    settings: { ...app.settings, web_url: 'http://127.0.0.1:4317' } };
  const update = spyOn(appHosting.update, 'invoke').mockResolvedValue({ ...running,
    completion_observation: { session_id: session, turn_id: turn, files_changed: true } });
  const accept = mock(async (_value: HostedApp) => {});
  const refresh = mock(async () => {});
  const view = renderHook(() => useAppDevelopmentSync(running, refresh, accept, session));
  act(() => emitStarted(started())); act(() => emitComplete(completed()));
  await waitFor(() => expect(view.result.current.configuring).toBe(false));
  expect(update.mock.calls[0][0].completion).toEqual({ session_id: session, turn_id: turn });
  expect(accept).toHaveBeenCalledWith(running);
  expect(view.result.current.previewFileChange?.sessionId).toBe(session);
  expect(view.result.current.configurationError).toBe('');
});
