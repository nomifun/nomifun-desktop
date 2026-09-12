import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { useLayoutEffect, type ComponentProps } from 'react';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { MemoryRouter, Route, Routes, useNavigate, type NavigateFunction } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { ITerminalSession } from '@/common/adapter/ipcBridge';
import { parseTerminalId } from '@/common/types/ids';
import * as storageKeys from '@/common/utils/browserStorageKey';
import { LayoutContext } from '@/renderer/hooks/context/LayoutContext';
import * as preview from '@/renderer/pages/conversation/Preview';
import * as knowledgeTab from '@/renderer/pages/conversation/Workspace/KnowledgePanel/useSessionKnowledgeTab';
import * as knowledge from '@/renderer/pages/conversation/components/KnowledgeControl';
import * as idmm from '@/renderer/pages/conversation/components/IdmmControl';
import * as autowork from '@/renderer/pages/conversation/components/AutoWorkControl';
import * as xterm from './XtermView';
import * as composer from './TerminalSendBox';
import TerminalSessionPage from './TerminalSessionPage';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach(dispose => dispose()); });
function queue<T>() {
  const pending: Array<{ resolve: (value: T) => void; reject: (error: unknown) => void }> = [];
  return { pending, invoke: () => new Promise<T>((resolve, reject) => { pending.push({ resolve, reject }); }) };
}
function event<T>(source: { on: (handler: (data: T) => void) => () => void }) {
  const handlers = new Set<(data: T) => void>();
  const spy = spyOn(source, 'on').mockImplementation(handler => {
    handlers.add(handler); return () => { handlers.delete(handler); };
  });
  restore.push(() => spy.mockRestore());
  return { handlers, emit: (data: T) => { for (const handler of [...handlers]) handler(data); } };
}
const session = (n = 1, last_status: ITerminalSession['last_status'] = 'running'): ITerminalSession => ({
  terminal_id: parseTerminalId('019b0000-0000-7000-8000-' + String(n).padStart(12, '0')),
  name: 'terminal ' + n, cwd: '/fixture', command: 'claude', args: [], cols: 80, rows: 24,
  created_at: 1, updated_at: 1, last_status,
});
function fixture() {
  const entityKey = spyOn(storageKeys, 'browserStorageKey').mockReturnValue('fixture-entity');
  const sessionKey = spyOn(storageKeys, 'sessionStorageKey').mockReturnValue('fixture-session');
  restore.push(() => entityKey.mockRestore(), () => sessionKey.mockRestore());
  const loads = queue<ITerminalSession>();
  const restarts = queue<ITerminalSession>();
  const fallbacks = queue<ITerminalSession>();
  const renames = queue<ITerminalSession>();
  const get = spyOn(ipcBridge.terminal.get, 'invoke').mockImplementation(loads.invoke);
  const relaunch = spyOn(ipcBridge.terminal.relaunch, 'invoke').mockImplementation(restarts.invoke);
  const fallback = spyOn(ipcBridge.terminal.relaunchShell, 'invoke').mockImplementation(fallbacks.invoke);
  const rename = spyOn(ipcBridge.terminal.update, 'invoke').mockImplementation(renames.invoke);
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  const log = spyOn(console, 'error').mockImplementation(() => {});
  for (const spy of [get, relaunch, fallback, rename, error, success, log]) restore.push(() => spy.mockRestore());
  const exit = event(ipcBridge.terminal.onExit);
  const updated = event(ipcBridge.terminal.onUpdated);
  const removed = event(ipcBridge.terminal.onRemoved);
  const reconnected = event<void>(ipcBridge.terminal.onReconnected);
  // Keep the page/router real; unrelated capability/preview UIs and the PTY
  // renderer are leaf seams. No module or fetch mocks escape this fixture.
  for (const module of [knowledge, idmm, autowork]) {
    const spy = spyOn(module, 'default').mockImplementation(() => null);
    restore.push(() => spy.mockRestore());
  }
  const provider = spyOn(preview, 'PreviewProvider').mockImplementation(({ children }) => <>{children}</>);
  const context = spyOn(preview, 'usePreviewContext').mockReturnValue({ isOpen: false } as ReturnType<typeof preview.usePreviewContext>);
  const tabs = spyOn(knowledgeTab, 'useSessionKnowledgeTab').mockReturnValue([]);
  const sendBox = spyOn(composer, 'default').mockImplementation(props => <button data-testid='composer' disabled={props.disabled} />);
  const frames: Array<ComponentProps<typeof xterm.default>> = [];
  const api = { clear: mock(), reset: mock(), focus: mock(), writeToPty: mock(async () => {}), isBracketedPaste: () => false };
  const terminal = spyOn(xterm, 'default').mockImplementation(props => {
    frames.push(props);
    useLayoutEffect(() => {
      if (props.apiRef) props.apiRef.current = api;
      return () => { if (props.apiRef) props.apiRef.current = null; };
    }, [props.apiRef]);
    return <div data-testid='terminal' data-running={String(props.isRunning)} />;
  });
  for (const spy of [provider, context, tabs, sendBox, terminal]) restore.push(() => spy.mockRestore());
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: { getItem: () => null, setItem: () => {} } });
  restore.push(() => {
    if (previous) Object.defineProperty(globalThis, 'localStorage', previous);
    else Reflect.deleteProperty(globalThis, 'localStorage');
  });
  let navigate!: NavigateFunction;
  function Navigation() { navigate = useNavigate(); return null; }
  const view = render(<I18nextProvider i18n={i18n}>
    <LayoutContext.Provider value={{ isMobile: true, siderCollapsed: true, setSiderCollapsed: () => {} }}>
      <MemoryRouter initialEntries={['/terminal/' + session().terminal_id]}>
        <Navigation /><Routes><Route path='/terminal/:id' element={<TerminalSessionPage />} /></Routes>
      </MemoryRouter>
    </LayoutContext.Provider>
  </I18nextProvider>);
  return {
    view, loads: loads.pending, restarts: restarts.pending, fallbacks: fallbacks.pending, renames: renames.pending,
    get, relaunch, fallback, rename, error, success, exit, updated, removed, reconnected, frames, api,
    load: async (value = session()) => { await act(async () => { loads.pending[0]!.resolve(value); }); },
    click: (name: string) => fireEvent.click(view.getByRole('button', { name })),
    edit: (name: string) => {
      fireEvent.click(view.getByTitle('terminal.action.rename'));
      fireEvent.change(view.getByRole('textbox'), { target: { value: name } });
    },
    switchTo: (n: number) => act(() => { void navigate('/terminal/' + session(n).terminal_id); }),
  };
}

test('exit received before initial GET is replayed over the snapshot', async () => {
  const f = fixture();
  act(() => f.exit.emit({ terminal_id: session().terminal_id, exit_code: 7 }));
  await f.load();
  expect(f.view.getByTestId('terminal').getAttribute('data-running')).toBe('false');
  expect((f.view.getByTestId('composer') as HTMLButtonElement).disabled).toBe(true);
});

test.each([false, true])('newer full metadata survives initial GET completion (failure=%s)', async failure => {
  const f = fixture();
  act(() => f.updated.emit({ ...session(), name: 'new name', last_status: 'exited' }));
  await act(async () => { if (failure) f.loads[0]!.reject('offline'); else f.loads[0]!.resolve(session()); });
  expect(f.view.getByText('new name')).not.toBeNull();
  expect(f.view.getByTestId('terminal').getAttribute('data-running')).toBe('false');
});

test('an exit without metadata does not hide GET failure or block retry', async () => {
  const f = fixture();
  act(() => f.exit.emit({ terminal_id: session().terminal_id, exit_code: 1 }));
  await act(async () => { f.loads[0]!.reject('offline'); });
  expect(f.view.getByRole('alert').textContent).toContain('Failed to load terminal session.');
  f.click('Retry');
  await act(async () => { f.loads[1]!.resolve(session(1, 'exited')); });
  expect(f.view.getByTestId('terminal').getAttribute('data-running')).toBe('false');
});

test('reconnect refreshes metadata, and removal cannot be undone by that pending GET', async () => {
  const f = fixture();
  await f.load();
  act(() => f.reconnected.emit());
  expect(f.get).toHaveBeenCalledTimes(2);
  act(() => f.removed.emit({ terminal_id: session().terminal_id }));
  await act(async () => { f.loads[1]!.resolve(session()); });
  expect(f.view.getByRole('alert').textContent).toContain('Terminal session not found.');
  f.click('Retry');
  await act(async () => { f.loads[2]!.resolve(session()); });
  expect(f.view.getByText('terminal 1')).not.toBeNull();
});

test.each(['relaunch', 'fallback', 'rename'] as const)('late %s completion after switching session is silent', async action => {
  const f = fixture();
  await f.load(session(1, 'exited'));
  if (action === 'rename') {
    f.edit('new name'); fireEvent.keyDown(f.view.getByRole('textbox'), { key: 'Enter', keyCode: 13 });
  } else f.click(action === 'relaunch' ? 'terminal.relaunch' : 'terminal.fallbackShell');
  f.switchTo(2);
  await act(async () => { f.loads[1]!.resolve(session(2)); });
  await act(async () => {
    if (action === 'relaunch') f.restarts[0]!.reject('late restart failure');
    else if (action === 'fallback') f.fallbacks[0]!.resolve({ ...session(), command: '$SHELL' });
    else f.renames[0]!.reject('late rename failure');
  });
  expect(f.error).not.toHaveBeenCalled();
  expect(f.success).not.toHaveBeenCalled();
  expect(f.view.getByText('terminal 2')).not.toBeNull();
});

test('restart and fallback share a synchronous gate; newer exit wins over the restart response', async () => {
  const f = fixture();
  await f.load(session(1, 'exited'));
  act(() => { f.click('terminal.relaunch'); f.click('terminal.relaunch'); f.click('terminal.fallbackShell'); });
  expect(f.relaunch).toHaveBeenCalledTimes(1);
  expect(f.fallback).not.toHaveBeenCalled();
  act(() => {
    f.updated.emit(session());
    f.exit.emit({ terminal_id: session().terminal_id, exit_code: 9 });
  });
  await act(async () => { f.restarts[0]!.resolve(session()); });
  expect(f.view.getByTestId('terminal').getAttribute('data-running')).toBe('false');
});

test('Escape does not suppress the next edit blur, and rename response cannot undo an exit', async () => {
  const f = fixture();
  await f.load();
  f.edit('cancelled');
  fireEvent.keyDown(f.view.getByRole('textbox'), { key: 'Escape' });
  f.edit('  renamed  ');
  fireEvent.blur(f.view.getByRole('textbox'));
  expect(f.rename).toHaveBeenCalledTimes(1);
  expect(f.rename).toHaveBeenCalledWith({ terminal_id: session().terminal_id, name: 'renamed' });
  act(() => f.exit.emit({ terminal_id: session().terminal_id, exit_code: 2 }));
  await act(async () => { f.renames[0]!.resolve({ ...session(), name: 'renamed' }); });
  expect(f.view.getByText('renamed')).not.toBeNull();
  expect(f.view.getByTestId('terminal').getAttribute('data-running')).toBe('false');
});

test('a successful rename invalidates a pending reconnect snapshot even without an update event', async () => {
  const f = fixture();
  await f.load();
  act(() => f.reconnected.emit());
  f.edit('renamed');
  fireEvent.blur(f.view.getByRole('textbox'));
  await act(async () => { f.renames[0]!.resolve({ ...session(), name: 'renamed' }); });
  await act(async () => { f.loads[1]!.resolve(session()); });
  expect(f.view.getByText('renamed')).not.toBeNull();
});

test('retry and exit invalidate old resize failures while exited scrollback remains visible', async () => {
  const f = fixture();
  await f.load();
  const oldFailure = f.frames.at(-1)!.onResizeFailure!;
  act(() => oldFailure(new Error('first resize failed')));
  f.click('Retry');
  act(() => oldFailure(new Error('obsolete failure')));
  expect(f.view.queryByRole('alert') !== null).toBe(false);
  const currentFailure = f.frames.at(-1)!.onResizeFailure!;
  act(() => currentFailure(new Error('current failure')));
  act(() => f.exit.emit({ terminal_id: session().terminal_id, exit_code: 0 }));
  expect(f.view.queryByRole('alert') !== null).toBe(false);
  expect(f.view.getByTestId('terminal').getAttribute('data-running')).toBe('false');
  act(() => currentFailure(new Error('late failure after exit')));
  expect(f.view.queryByRole('alert') !== null).toBe(false);
  f.view.unmount();
  expect([f.exit, f.updated, f.removed, f.reconnected].map(source => source.handlers.size)).toEqual([0, 0, 0, 0]);
});
