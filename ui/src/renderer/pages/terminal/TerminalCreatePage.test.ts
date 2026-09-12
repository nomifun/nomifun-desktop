/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { createElement as h } from 'react';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { MemoryRouter, Route, Routes, useLocation, useNavigate, type NavigateFunction } from 'react-router-dom';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { IAutoWorkState, IIdmmState, ITerminalSession } from '@/common/adapter/ipcBridge';
import { parseTerminalId } from '@/common/types/ids';
import { emitter } from '@/renderer/utils/emitter';
import TerminalCreatePage from './TerminalCreatePage';
import * as extendedPanel from './ExtendedCapabilitiesPanel';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('TerminalCreatePage extended capabilities', () => {
  test('wires smart decision as a create-time draft capability', () => {
    const createPageSource = readSource(new URL('./TerminalCreatePage.tsx', import.meta.url));
    const panelSource = readSource(new URL('./ExtendedCapabilitiesPanel.tsx', import.meta.url));

    expect(createPageSource.includes('defaultIdmmConfig')).toBe(true);
    expect(createPageSource.includes('const [idmm, setIdmm]')).toBe(true);
    expect(createPageSource.includes('ipcBridge.idmm.set.invoke')).toBe(true);
    expect(createPageSource.includes("kind: 'terminal'")).toBe(true);
    expect(createPageSource.includes('target_id: session.terminal_id')).toBe(true);

    expect(panelSource.includes('IdmmControl')).toBe(true);
    // The draft declares its kind: a terminal has no model of its own to lend the
    // model tier (its agent CLI owns the model), so a terminal watch must name a
    // bypass model itself. Without this the control would offer a one-click
    // enable that the backend then rejects with a 400.
    expect(panelSource.includes("draft={{ value: idmm, onChange: onIdmmChange, kind: 'terminal' }}")).toBe(true);
  });
});

// Exercise the page itself; only the optional draft editor is replaced at its
// props boundary, so these tests do not configure unrelated capability UIs.
const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach(dispose => dispose()); });
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const session: ITerminalSession = {
  terminal_id: parseTerminalId('019b0000-0000-7000-8000-000000000001'),
  name: 'fixture', cwd: '/first', command: 'claude', args: [], cols: 80, rows: 24,
  created_at: 1, updated_at: 1, last_status: 'running',
};
function fixture() {
  const created = deferred<ITerminalSession>();
  const decision = deferred<void>();
  const autowork = deferred<void>();
  const create = spyOn(ipcBridge.terminal.create, 'invoke').mockImplementation(() => created.promise);
  // These responses are opaque to this page: only completion/failure matters.
  const idmm = spyOn(ipcBridge.idmm.set, 'invoke').mockImplementation(async () => { await decision.promise; return {} as IIdmmState; });
  const auto = spyOn(ipcBridge.requirements.setAutoWork, 'invoke').mockImplementation(async () => { await autowork.promise; return {} as IAutoWorkState; });
  const bases = spyOn(ipcBridge.knowledge.listBases, 'invoke').mockResolvedValue([]);
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  const warning = spyOn(Message, 'warning').mockImplementation(() => () => {});
  const panel = spyOn(extendedPanel, 'default').mockImplementation(props => h('button', {
    onClick: () => {
      props.onIdmmChange({ ...props.idmm, fault_watch: { ...props.idmm.fault_watch, enabled: true } });
      props.onAutoworkChange({ enabled: true, tag: 'work' });
      props.onKbIdsChange(['knowledge-fixture']);
    },
  }, 'enable fixture options'));
  for (const spy of [create, idmm, auto, bases, error, warning, panel]) restore.push(() => spy.mockRestore());
  const storage = { getItem: mock((_key: string) => null), setItem: mock((_key: string, _value: string) => {}) };
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: storage });
  restore.push(() => {
    if (previous) Object.defineProperty(globalThis, 'localStorage', previous);
    else Reflect.deleteProperty(globalThis, 'localStorage');
  });
  const refreshed = mock();
  emitter.on('terminal.list.refresh', refreshed);
  restore.push(() => { emitter.off('terminal.list.refresh', refreshed); });
  let navigate!: NavigateFunction;
  function RouteProbe() {
    navigate = useNavigate();
    return h('output', { 'data-testid': 'route' }, useLocation().pathname);
  }
  const view = render(h(I18nextProvider, { i18n }, h(MemoryRouter, {
    initialEntries: ['/origin', { pathname: '/terminal-new', state: { cwd: '/first' } }], initialIndex: 1,
  }, h(RouteProbe), h(Routes, null,
    h(Route, { path: '/terminal-new', element: h(TerminalCreatePage) }),
    h(Route, { path: '*', element: h('div', null, 'destination') }),
  ))));
  return {
    view, created, decision, autowork, create, idmm, auto, error, warning, refreshed, storage,
    launch: () => fireEvent.click(view.getByRole('button', { name: 'terminal.create.launch' })),
    cancel: () => fireEvent.click(view.getByRole('button', { name: 'common.cancel' })),
    command: (value: string) => fireEvent.change(view.getByPlaceholderText('$SHELL'), { target: { value } }),
    options: () => fireEvent.click(view.getByText('enable fixture options')),
    revisit: () => act(() => { void navigate('/terminal-new'); }),
    path: () => view.getByTestId('route').textContent,
  };
}

test('same-render clicks issue one create; failure keeps the draft and unlocks retry', async () => {
  const f = fixture();
  f.command('claude --fixture');
  f.create.mockRejectedValueOnce(new Error('offline'));
  act(() => { f.launch(); f.launch(); });
  const count = f.create.mock.calls.length;
  await act(async () => {});
  expect(count).toBe(1);
  expect(f.error).toHaveBeenCalledWith('offline');
  expect((f.view.getByPlaceholderText('$SHELL') as HTMLInputElement).value).toBe('claude --fixture');
  f.launch();
  await act(async () => { f.created.resolve(session); });
  expect(f.create).toHaveBeenCalledTimes(2);
  expect(f.path()).toBe('/terminal/' + session.terminal_id);
});

test.each(['create', 'idmm', 'autowork'] as const)('leaving during %s prevents stale continuation, warnings and navigation', async stage => {
  const f = fixture();
  f.command('claude --fixture');
  f.options();
  f.launch();
  if (stage !== 'create') await act(async () => { f.created.resolve(session); });
  if (stage === 'autowork') await act(async () => { f.decision.resolve(); });
  f.cancel();
  expect(f.path()).toBe('/origin');
  await act(async () => {
    if (stage === 'create') f.created.resolve(session);
    else if (stage === 'idmm') f.decision.reject('late idmm failure');
    else f.autowork.reject('late autowork failure');
  });
  expect(f.path()).toBe('/origin');
  expect(f.error).not.toHaveBeenCalled();
  expect(f.warning).not.toHaveBeenCalled();
  expect(f.refreshed).not.toHaveBeenCalled();
  expect(f.idmm).toHaveBeenCalledTimes(stage === 'create' ? 0 : 1);
  expect(f.auto).toHaveBeenCalledTimes(stage === 'autowork' ? 1 : 0);
  if (stage === 'create') expect(f.storage.setItem).not.toHaveBeenCalled();
});

test('a create rejection after leaving is silent', async () => {
  const f = fixture();
  f.launch();
  f.cancel();
  await act(async () => { f.created.reject(null); });
  expect(f.error).not.toHaveBeenCalled();
  expect(f.path()).toBe('/origin');
});

test('same-route navigation resets the default cwd and invalidates the former launch', async () => {
  const f = fixture();
  f.launch();
  f.revisit();
  expect((f.view.getByPlaceholderText('terminal.create.workspacePlaceholder') as HTMLInputElement).value).toBe('');
  const current = deferred<ITerminalSession>();
  f.create.mockImplementation(() => current.promise);
  f.launch();
  expect(f.create).toHaveBeenCalledTimes(2);
  expect(f.create.mock.calls[1]![0].cwd).toBe('');
  await act(async () => { f.created.resolve(session); });
  expect(f.path()).toBe('/terminal-new');
  expect(f.refreshed).not.toHaveBeenCalled();
  expect(f.view.getByRole('button', { name: 'terminal.create.launch' }).classList.contains('arco-btn-loading')).toBe(true);
  await act(async () => { current.resolve(session); });
  expect(f.path()).toBe('/terminal/' + session.terminal_id);
});

test('creation payload is captured once, and optional failures still reach the created terminal in order', async () => {
  const f = fixture();
  f.command('claude --fixture');
  f.options();
  f.launch();
  expect(f.create).toHaveBeenCalledWith({ cwd: '/first', command: 'claude', args: ['--fixture'],
    backend: undefined, mode: undefined, defer_spawn: true, knowledge_base_ids: ['knowledge-fixture'] });
  f.command('changed draft');
  await act(async () => { f.created.resolve(session); });
  expect(f.idmm).toHaveBeenCalledWith(expect.objectContaining({ kind: 'terminal', target_id: session.terminal_id }));
  expect(f.auto).not.toHaveBeenCalled();
  await act(async () => { f.decision.reject('idmm unavailable'); });
  expect(f.auto).toHaveBeenCalledWith({ kind: 'terminal', target_id: session.terminal_id, enabled: true, tag: 'work' });
  await act(async () => { f.autowork.reject('autowork unavailable'); });
  expect(f.warning).toHaveBeenCalledTimes(2);
  expect(f.error).not.toHaveBeenCalled();
  expect(f.storage.setItem).toHaveBeenCalledWith('nomifun:recent-terminal-commands', JSON.stringify(['claude --fixture']));
  expect(f.refreshed).toHaveBeenCalledTimes(1);
  expect(f.path()).toBe('/terminal/' + session.terminal_id);
});

test('blank input retains the default shell contract; an explicitly empty program is rejected', async () => {
  const f = fixture();
  f.command('""');
  f.launch();
  expect(f.create).not.toHaveBeenCalled();
  expect(f.warning).toHaveBeenCalledWith('terminal.create.commandRequired');
  f.command('   ');
  f.launch();
  expect(f.create.mock.calls[0]![0].command).toBe('$SHELL');
  await act(async () => { f.created.resolve(session); });
  expect(f.path()).toBe('/terminal/' + session.terminal_id);
});
