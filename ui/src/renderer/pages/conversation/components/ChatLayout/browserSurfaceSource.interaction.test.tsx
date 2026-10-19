import '../../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import * as nativeEvents from '@tauri-apps/api/event';
import { ipcBridge } from '@/common';
import { parseConversationId } from '@/common/types/ids';
import { initializeAgentBrowserStorageGeneration, setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { browserClient, type BrowserSnapshot, type BrowserSurfaceSource } from '../../Browser/client';
import { useBrowserLink } from '../../Browser/BrowserLinkContext';
import * as Preview from '../../Preview';
import * as Title from '../ChatTitleEditor';
import ChatLayout from './index';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: {} } }, interpolation: { escapeValue: false } });
const sessionA = parseConversationId('0190f5fe-7c00-7a00-8000-000000000741');
const sessionB = parseConversationId('0190f5fe-7c00-7a00-8000-000000000742');
const originalRect = HTMLElement.prototype.getBoundingClientRect;
const originalPort = Object.getOwnPropertyDescriptor(window, '__backendPort');
const originalOs = Object.getOwnPropertyDescriptor(window, '__os');
const originalWidth = Object.getOwnPropertyDescriptor(window, 'innerWidth');
type BrowserOpenListener = (event: nativeEvents.Event<string>) => void;
let listeners = new Set<BrowserOpenListener>();
let ensures: string[] = [];
let attachments: Array<{ session: string; source?: BrowserSurfaceSource }> = [];
let commands: string[] = [];
const snapshot = (id: string): BrowserSnapshot => ({ agent_session_id: id, browser_id: 'managed-browser',
  run: { revision: 1, input_state: 'user_ready', input_gate_failed: false }, runtime: null });

beforeEach(() => {
  listeners = new Set(); ensures = []; attachments = []; commands = [];
  localStorage.clear();
  setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000743');
  initializeAgentBrowserStorageGeneration(1);
  Object.defineProperty(window, '__backendPort', { configurable: true, value: 1 });
  Object.defineProperty(window, '__os', { configurable: true, value: 'macos' });
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1200 });
  HTMLElement.prototype.getBoundingClientRect = () => ({ x: 0, y: 0, width: 1200, height: 600,
    left: 0, top: 0, right: 1200, bottom: 600, toJSON: () => ({}) });
  spyOn(ipcBridge.preview.open, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockImplementation(() => () => {});
  spyOn(Preview, 'PreviewPanel').mockImplementation(() => <div>Attachment content</div>);
  spyOn(Title, 'default').mockImplementation(() => <span>Browser source fixture</span>);
  spyOn(nativeEvents, 'listen').mockImplementation(async (name, handler) => {
    if (name !== 'browser-workspace-open') return () => {};
    const listener = handler as BrowserOpenListener;
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  });
  spyOn(browserClient, 'listenShortcuts').mockImplementation(async () => () => {});
  spyOn(browserClient, 'ensure').mockImplementation(async id => { ensures.push(id); return snapshot(id); });
  spyOn(browserClient, 'command').mockImplementation(async id => { commands.push(id); return snapshot(id); });
  spyOn(browserClient, 'scaleFactor').mockImplementation(async () => 1);
  spyOn(browserClient, 'attach').mockImplementation(async (session, _bounds, _events, source) => {
    attachments.push({ session, source }); return attachments.length;
  });
  spyOn(browserClient, 'update').mockImplementation(async () => {});
  spyOn(browserClient, 'detach').mockImplementation(async () => {});
});

afterEach(() => {
  cleanup(); mock.restore(); localStorage.clear();
  HTMLElement.prototype.getBoundingClientRect = originalRect;
  for (const [key, descriptor] of [['__backendPort', originalPort], ['__os', originalOs], ['innerWidth', originalWidth]] as const) {
    if (descriptor) Object.defineProperty(window, key, descriptor); else Reflect.deleteProperty(window, key);
  }
});

function Composer() {
  const open = useBrowserLink();
  return <button type='button' onClick={() => open('http://localhost:3000/project')}>Open local link</button>;
}
function layout(id = sessionA) {
  return <I18nextProvider i18n={i18n}><ChatLayout conversation_id={id} hideAdvancedControls workspaceEnabled={false} sider={null}>
    <Composer />
  </ChatLayout></I18nextProvider>;
}
async function autoOpen(session: string) {
  await waitFor(() => expect(listeners.size).toBe(1));
  await act(async () => { for (const listener of listeners) listener({ event: 'browser-workspace-open', id: 1, payload: session }); });
}

test('macOS automatic open stays automatic until a local link takes over the existing panel', async () => {
  const screen = render(layout());
  await autoOpen(sessionA);
  await waitFor(() => expect(attachments).toEqual([{ session: sessionA, source: 'agent_auto' }]));
  fireEvent.click(screen.getByRole('button', { name: 'Open local link' }));
  await waitFor(() => expect(commands).toEqual([sessionA]));
  await autoOpen(sessionA);
  expect(ensures).toEqual([sessionA]);
  expect(attachments).toHaveLength(1);
  const sourceUpdates = (browserClient.update as ReturnType<typeof spyOn>).mock.calls;
  expect(sourceUpdates.some((call: readonly unknown[]) => call[4] === 'user')).toBe(true);
  expect(sourceUpdates.at(-1)?.[4]).toBe('user');
});

test('switching Sessions keeps manual reopening user-owned and isolates the previous Session late link completion', async () => {
  let finish!: (value: BrowserSnapshot) => void;
  (browserClient.command as ReturnType<typeof spyOn>).mockImplementation(async (id: string) => {
    commands.push(id); return new Promise<BrowserSnapshot>(resolve => { finish = resolve; });
  });
  const screen = render(layout());
  await autoOpen(sessionA);
  await waitFor(() => expect(attachments).toHaveLength(1));
  fireEvent.click(screen.getByRole('button', { name: 'Open local link' }));
  await waitFor(() => expect(commands).toEqual([sessionA]));
  const oldListener = [...listeners][0]!;
  screen.rerender(layout(sessionB));
  await act(async () => {
    oldListener({ event: 'browser-workspace-open', id: 2, payload: sessionA });
    finish(snapshot(sessionA));
  });
  // The existing per-Session PreviewProvider key remounts the layout. A new
  // Session therefore waits for its own explicit Browser open.
  expect(screen.queryByRole('region', { name: 'Browser' })).toBeNull();
  expect(ensures).toEqual([sessionA]);
  fireEvent.click(screen.getByRole('button', { name: 'Browser' }));
  await waitFor(() => expect(attachments).toEqual([{ session: sessionA, source: 'agent_auto' }, { session: sessionB, source: 'user' }]));
  expect(ensures).toEqual([sessionA, sessionB]);
  await autoOpen(sessionB);
  expect(attachments).toHaveLength(2);
  expect(commands).toEqual([sessionA]);
});

test('an automatic surface does not implicitly attach another Session or accept its previous late event', async () => {
  const screen = render(layout());
  await autoOpen(sessionA);
  await waitFor(() => expect(attachments).toHaveLength(1));
  const oldListener = [...listeners][0]!;
  screen.rerender(layout(sessionB));
  await act(async () => oldListener({ event: 'browser-workspace-open', id: 2, payload: sessionA }));
  expect(screen.queryByRole('region', { name: 'Browser' })).toBeNull();
  expect(ensures).toEqual([sessionA]);
  expect(attachments).toHaveLength(1);
  screen.rerender(layout(sessionA));
  expect(screen.queryByRole('region', { name: 'Browser' })).toBeNull();
  await autoOpen(sessionA);
  await waitFor(() => expect(attachments).toHaveLength(2));
  expect(attachments.at(-1)).toEqual({ session: sessionA, source: 'agent_auto' });
});

test('the manual browser button remains user-owned after a later automatic open', async () => {
  const screen = render(layout());
  fireEvent.click(screen.getByRole('button', { name: 'Browser' }));
  await waitFor(() => expect(attachments).toEqual([{ session: sessionA, source: 'user' }]));
  await autoOpen(sessionA);
  expect(ensures).toEqual([sessionA]);
  expect(attachments).toHaveLength(1);
});

test('Windows keeps its original user surface path for the same automatic event', async () => {
  Object.defineProperty(window, '__os', { configurable: true, value: 'windows' });
  render(layout());
  await autoOpen(sessionA);
  await waitFor(() => expect(attachments).toEqual([{ session: sessionA, source: 'user' }]));
});
