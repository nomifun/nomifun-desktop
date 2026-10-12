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
import { Message, Modal } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { ITerminalSession } from '@/common/adapter/ipcBridge';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { runtimeTools, type RuntimeToolSnapshot } from '@/common/adapter/runtimeTools';
import { parseTerminalId } from '@/common/types/ids';
import { emitter } from '@/renderer/utils/emitter';
import TerminalCreatePage from './TerminalCreatePage';
import * as extendedPanel from './ExtendedCapabilitiesPanel';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('TerminalCreatePage extended capabilities', () => {
  test('keeps Knowledge and CLI registration without terminal automation controls', () => {
    const createPageSource = readSource(new URL('./TerminalCreatePage.tsx', import.meta.url));
    const panelSource = readSource(new URL('./ExtendedCapabilitiesPanel.tsx', import.meta.url));

    expect(createPageSource.includes('ipcBridge.idmm')).toBe(false);
    expect(createPageSource.includes('setAutoWork')).toBe(false);
    expect(panelSource.includes('IdmmControl')).toBe(false);
    expect(panelSource.includes('AutoWorkControl')).toBe(false);
    expect(panelSource.includes('<RegisterKnowledgeButton')).toBe(true);
    expect(panelSource.includes('<PlatformMcpRegisterPanel')).toBe(true);
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
const missingBun = () => new BackendHttpError({
  method: 'POST', path: '/api/terminal', status: 409,
  body: { code: 'COMPONENT_PREPARATION_REQUIRED', error: 'Bun is required', details: { component: 'bun', phase: 'runtime_resolution' } },
});
function fixture() {
  const created = deferred<ITerminalSession>();
  const create = spyOn(ipcBridge.terminal.create, 'invoke').mockImplementation(() => created.promise);
  const bases = spyOn(ipcBridge.knowledge.listBases, 'invoke').mockResolvedValue([]);
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  const warning = spyOn(Message, 'warning').mockImplementation(() => () => {});
  let confirmation: Parameters<typeof Modal.confirm>[0] | undefined;
  const closeConfirmation = mock();
  const confirm = spyOn(Modal, 'confirm').mockImplementation((options) => {
    confirmation = options;
    return { close: closeConfirmation, update: mock() };
  });
  const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockImplementation(async ({ path }) => ({
    name: path.split(/[\\/]/).pop() || path,
    path,
    size: 0,
    type: 'inode/directory',
    lastModified: 1,
    isDirectory: true,
  }));
  const panel = spyOn(extendedPanel, 'default').mockImplementation(props => h('button', {
    onClick: () => {
      props.onKbIdsChange(['knowledge-fixture']);
    },
  }, 'enable fixture options'));
  for (const spy of [create, bases, error, warning, metadata, panel, confirm]) restore.push(() => spy.mockRestore());
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
    view, created, create, error, warning, metadata, refreshed, storage, confirm, closeConfirmation,
    acceptDownload: () => act(async () => { await confirmation?.onOk?.(); }),
    declineDownload: () => act(async () => { confirmation?.onCancel?.(); await Promise.resolve(); }),
    launch: (count = 1) => act(async () => {
      for (let index = 0; index < count; index += 1) {
        fireEvent.click(view.getByRole('button', { name: 'terminal.create.launch' }));
      }
      await Promise.resolve();
    }),
    cancel: () => act(() => { fireEvent.click(view.getByRole('button', { name: 'common.cancel' })); }),
    command: (value: string) => act(() => { fireEvent.input(view.getByPlaceholderText('$SHELL'), { target: { value } }); }),
    options: () => act(() => { fireEvent.click(view.getByText('enable fixture options')); }),
    revisit: () => act(() => { void navigate('/terminal-new'); }),
    path: () => view.getByTestId('route').textContent,
  };
}

test('only a missing Bun create result prompts a download and retries the captured payload', async () => {
  const prepared = deferred<RuntimeToolSnapshot>();
  const prepare = spyOn(runtimeTools.prepare, 'invoke').mockImplementation(() => prepared.promise);
  restore.push(() => prepare.mockRestore());
  const f = fixture(); f.command('bun --version'); f.options();
  f.create.mockRejectedValueOnce(missingBun());
  await f.launch(2);
  expect(f.create).toHaveBeenCalledTimes(1);
  const request = f.create.mock.calls[0]![0];
  expect(f.confirm).toHaveBeenCalledTimes(1);
  expect(prepare).not.toHaveBeenCalled();
  f.command('changed draft');
  await f.acceptDownload();
  expect(prepare).toHaveBeenCalledTimes(1);
  expect(f.create).toHaveBeenCalledTimes(1);
  await act(async () => { prepared.resolve({ tool: 'bun', version: '1.3.13', revision: 1, phase: 'ready', received_bytes: 0 }); await Promise.resolve(); });
  expect(f.create).toHaveBeenCalledTimes(2);
  expect(f.create.mock.calls[1]![0]).toBe(request);
  expect(request).toEqual({ cwd: '/first', command: 'bun', args: ['--version'],
    backend: undefined, mode: undefined, defer_spawn: true, knowledge_base_ids: ['knowledge-fixture'] });
  await act(async () => { f.created.resolve(session); });
  expect(f.path()).toBe(`/terminal/${session.terminal_id}`);
});

for (const command of ['bun --version', 'bunx workspace-server']) test(`the owner can create ${command} in its launch directory without a generic runtime probe`, async () => {
  const prepare = spyOn(runtimeTools.prepare, 'invoke');
  restore.push(() => prepare.mockRestore());
  const f = fixture(); f.command(command);
  await f.launch();
  expect(f.create).toHaveBeenCalledTimes(1);
  expect(f.create.mock.calls[0]![0].cwd).toBe('/first');
  expect(f.create.mock.calls[0]![0].command).toBe(command.split(' ')[0]!);
  expect(f.confirm).not.toHaveBeenCalled();
  expect(prepare).not.toHaveBeenCalled();
  await act(async () => { f.created.resolve(session); });
});

test('declining a missing runtime preserves the draft without downloading or retrying create', async () => {
  const prepare = spyOn(runtimeTools.prepare, 'invoke');
  restore.push(() => prepare.mockRestore());
  const f = fixture(); f.command('bun --version');
  f.create.mockRejectedValueOnce(missingBun());
  await f.launch();
  await f.declineDownload();
  expect(f.create).toHaveBeenCalledTimes(1);
  expect(prepare).not.toHaveBeenCalled();
  expect(f.path()).toBe('/terminal-new');
  expect((f.view.getByPlaceholderText('$SHELL') as HTMLInputElement).value).toBe('bun --version');
  expect(f.view.getByRole('button', { name: 'terminal.create.launch' }).classList.contains('arco-btn-loading')).toBe(false);
});

test('leaving closes the download choice and rejects a stale acceptance', async () => {
  const prepare = spyOn(runtimeTools.prepare, 'invoke');
  restore.push(() => prepare.mockRestore());
  const f = fixture(); f.command('bun --version');
  f.create.mockRejectedValueOnce(missingBun());
  await f.launch(); f.cancel();
  expect(f.closeConfirmation).toHaveBeenCalledTimes(1);
  await f.acceptDownload();
  expect(f.create).toHaveBeenCalledTimes(1);
  expect(prepare).not.toHaveBeenCalled();
});

test('leaving during preparation prevents a late terminal creation', async () => {
  const prepared = deferred<RuntimeToolSnapshot>();
  const prepare = spyOn(runtimeTools.prepare, 'invoke').mockImplementation(() => prepared.promise);
  restore.push(() => prepare.mockRestore());
  const f = fixture(); f.command('bun --version');
  f.create.mockRejectedValueOnce(missingBun());
  await f.launch(); await f.acceptDownload(); f.cancel();
  await act(async () => { prepared.resolve({ tool: 'bun', version: '1.3.13', revision: 1, phase: 'ready', received_bytes: 0 }); await Promise.resolve(); });
  expect(f.create).toHaveBeenCalledTimes(1);
  expect(f.path()).toBe('/origin');
});

test('a missing runtime result after leaving cannot open a download choice', async () => {
  const prepare = spyOn(runtimeTools.prepare, 'invoke');
  restore.push(() => prepare.mockRestore());
  const f = fixture(); f.command('bun --version');
  await f.launch(); f.cancel();
  await act(async () => { f.created.reject(missingBun()); });
  expect(f.create).toHaveBeenCalledTimes(1);
  expect(f.confirm).not.toHaveBeenCalled();
  expect(prepare).not.toHaveBeenCalled();
  expect(f.error).not.toHaveBeenCalled();
});

for (const failure of [
  { label: 'launcher spawn error', error: new Error('PTY spawn failed') },
  { label: 'broken system Bun', error: new BackendHttpError({ method: 'POST', path: '/api/terminal', status: 422,
    body: { code: 'COMPONENT_UNAVAILABLE', error: 'System Bun cannot execute', details: { component: 'bun', phase: 'runtime_resolution' } } }) },
  { label: 'missing different component', error: new BackendHttpError({ method: 'POST', path: '/api/terminal', status: 409,
    body: { code: 'COMPONENT_PREPARATION_REQUIRED', error: 'Node is missing', details: { component: 'node' } } }) },
]) test(`${failure.label} never offers a Bun download or retries creation`, async () => {
  const prepare = spyOn(runtimeTools.prepare, 'invoke');
  restore.push(() => prepare.mockRestore());
  const f = fixture(); f.command('bun --version');
  f.create.mockRejectedValueOnce(failure.error);
  await f.launch();
  expect(f.create).toHaveBeenCalledTimes(1);
  expect(f.confirm).not.toHaveBeenCalled();
  expect(prepare).not.toHaveBeenCalled();
  expect(f.error).toHaveBeenCalledTimes(1);
});

test('same-render clicks issue one create; failure keeps the draft and unlocks retry', async () => {
  const f = fixture();
  f.command('claude --fixture');
  f.create.mockRejectedValueOnce(new Error('offline'));
  await f.launch(2);
  const count = f.create.mock.calls.length;
  expect(count).toBe(1);
  expect(f.error).toHaveBeenCalledWith('offline');
  expect((f.view.getByPlaceholderText('$SHELL') as HTMLInputElement).value).toBe('claude --fixture');
  await f.launch();
  await act(async () => { f.created.resolve(session); });
  expect(f.create).toHaveBeenCalledTimes(2);
  expect(f.path()).toBe('/terminal/' + session.terminal_id);
});

test('leaving during create prevents stale continuation, warnings and navigation', async () => {
  const f = fixture();
  f.command('claude --fixture');
  f.options();
  await f.launch();
  f.cancel();
  expect(f.path()).toBe('/origin');
  await act(async () => { f.created.resolve(session); });
  expect(f.path()).toBe('/origin');
  expect(f.error).not.toHaveBeenCalled();
  expect(f.warning).not.toHaveBeenCalled();
  expect(f.refreshed).not.toHaveBeenCalled();
  expect(f.storage.setItem).not.toHaveBeenCalled();
});

test('a create rejection after leaving is silent', async () => {
  const f = fixture();
  await f.launch();
  f.cancel();
  await act(async () => { f.created.reject(null); });
  expect(f.error).not.toHaveBeenCalled();
  expect(f.path()).toBe('/origin');
});

test('same-route navigation resets the default cwd and invalidates the former launch', async () => {
  const f = fixture();
  await f.launch();
  f.revisit();
  expect((f.view.getByPlaceholderText('terminal.create.workspacePlaceholder') as HTMLInputElement).value).toBe('');
  const current = deferred<ITerminalSession>();
  f.create.mockImplementation(() => current.promise);
  await f.launch();
  expect(f.create).toHaveBeenCalledTimes(2);
  expect(f.create.mock.calls[1]![0].cwd).toBe('');
  await act(async () => { f.created.resolve(session); });
  expect(f.path()).toBe('/terminal-new');
  expect(f.refreshed).not.toHaveBeenCalled();
  expect(f.view.getByRole('button', { name: 'terminal.create.launch' }).classList.contains('arco-btn-loading')).toBe(true);
  await act(async () => { current.resolve(session); });
  expect(f.path()).toBe('/terminal/' + session.terminal_id);
});

test('creation payload captures the Knowledge mount and reaches the created terminal directly', async () => {
  const f = fixture();
  f.command('claude --fixture');
  f.options();
  await f.launch();
  expect(f.create).toHaveBeenCalledWith({ cwd: '/first', command: 'claude', args: ['--fixture'],
    backend: undefined, mode: undefined, defer_spawn: true, knowledge_base_ids: ['knowledge-fixture'] });
  f.command('changed draft');
  await act(async () => { f.created.resolve(session); });
  expect(f.warning).not.toHaveBeenCalled();
  expect(f.error).not.toHaveBeenCalled();
  expect(f.storage.setItem).toHaveBeenCalledWith('nomifun:recent-terminal-commands', JSON.stringify(['claude --fixture']));
  expect(f.refreshed).toHaveBeenCalledTimes(1);
  expect(f.path()).toBe('/terminal/' + session.terminal_id);
});

test('a removed working directory is rejected before a terminal row is created', async () => {
  const f = fixture();
  f.metadata.mockRejectedValueOnce(new Error('directory not found'));

  await f.launch();

  expect(f.create).not.toHaveBeenCalled();
  expect(f.error).toHaveBeenCalledWith('terminal.create.workspaceUnavailable');
});

test('blank input retains the default shell contract; an explicitly empty program is rejected', async () => {
  const f = fixture();
  f.command('""');
  await f.launch();
  expect(f.create).not.toHaveBeenCalled();
  expect(f.warning).toHaveBeenCalledWith('terminal.create.commandRequired');
  f.command('   ');
  await f.launch();
  expect(f.create.mock.calls[0]![0].command).toBe('$SHELL');
  await act(async () => { f.created.resolve(session); });
  expect(f.path()).toBe('/terminal/' + session.terminal_id);
});
