import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import SystemBrowserControl from './SystemBrowserControl';
import type { SystemBrowserClient, SystemBrowserSnapshot } from './client';
import words from '../../../services/i18n/locales/en-US/browserWorkspace.json';
import { readFileSync } from 'node:fs';
import BrowserWorkspacePanel from '../Browser/BrowserWorkspacePanel';
import type { BrowserClient, BrowserSnapshot } from '../Browser/client';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', interpolation: { escapeValue: false }, resources: { 'en-US': { translation: { browserWorkspace: words } } } });
afterEach(cleanup);
const text = words.systemBrowser;
const connected: SystemBrowserSnapshot = { incarnation: 'connection-1', state: 'connected', tabs: [] };
const selected = { tab_id: 'grant-1', title: '<img src=x> Private page', url: 'https://example.test/private' };
function fixture(overrides: Partial<SystemBrowserClient> = {}, initialProps = { conversationId: 'chat-1', locked: false, available: true }) {
  const calls: unknown[][] = [];
  const client: SystemBrowserClient = {
    async snapshot(id) { calls.push(['snapshot', id]); return null; },
    async connect(id, incarnation) { calls.push(['connect', id, incarnation]); return connected; },
    async choices(id, incarnation) { calls.push(['choices', id, incarnation]); return { tabs: [{ choice_id: 'choice-1', title: selected.title, url: selected.url }] }; },
    async grant(id, incarnation, choice) { calls.push(['grant', id, incarnation, choice]); return { ...connected, tabs: [selected] }; },
    async disconnect(id, incarnation) { calls.push(['disconnect', id, incarnation]); return { ...connected, state: 'disconnected', tabs: [] }; },
    ...overrides,
  };
  let props = initialProps;
  const element = () => <I18nextProvider i18n={i18n}><SystemBrowserControl {...props} client={client} /></I18nextProvider>;
  const screen = render(element());
  return { ...screen, calls, client,
    change: (next: Partial<typeof initialProps>) => { props = { ...props, ...next }; screen.rerender(element()); },
    open: async () => { fireEvent.click(screen.getByRole('button', { name: text.title })); return screen.findByRole('dialog', { name: text.title }); },
    ready: async () => waitFor(() => expect((screen.getByRole('button', { name: text.refresh }) as HTMLButtonElement).disabled).toBe(false)),
  };
}
test('opening reads only status; connecting and enumerating each require a separate user click', async () => {
  const screen = fixture();
  expect(screen.calls).toEqual([]);
  await screen.open(); await screen.ready();
  expect(screen.calls).toEqual([['snapshot', 'chat-1']]);
  fireEvent.click(screen.getByRole('button', { name: text.connect }));
  await screen.findByRole('button', { name: text.chooseTabs }); await screen.ready();
  expect(screen.calls).toEqual([['snapshot', 'chat-1'], ['connect', 'chat-1', undefined]]);
  fireEvent.click(screen.getByRole('button', { name: text.chooseTabs }));
  const choice = await screen.findByRole('button', { name: `Authorize tab: ${selected.title}` });
  expect(screen.getByRole('group', { name: text.availableTabs }).querySelector('img')).toBeNull();
  expect(within(screen.getByRole('group', { name: text.authorizedTabs })).getByText(text.noAuthorizedTabs)).toBeTruthy();
  fireEvent.click(choice);
  await waitFor(() => expect(screen.calls.at(-1)).toEqual(['grant', 'chat-1', 'connection-1', 'choice-1']));
  expect(await within(screen.getByRole('group', { name: text.authorizedTabs })).findByText(selected.title)).toBeTruthy();
  expect(screen.queryByRole('group', { name: text.availableTabs })).toBeNull();
  expect(screen.getByRole('group', { name: text.authorizedTabs }).querySelector('img')).toBeNull();
});
test('running locks mutations and discards choices without replaying on unlock', async () => {
  const screen = fixture({ async snapshot() { return connected; } });
  await screen.open(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: text.chooseTabs }));
  await screen.findByRole('button', { name: `Authorize tab: ${selected.title}` });
  screen.change({ locked: true });
  expect(screen.queryByRole('group', { name: text.availableTabs })).toBeNull();
  expect((screen.getByRole('button', { name: text.disconnect }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByRole('button', { name: text.chooseTabs }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: text.disconnect }));
  screen.change({ locked: false });
  expect(screen.calls).toEqual([['choices', 'chat-1', 'connection-1']]);
});
test('unsupported platforms explain availability without HTTP or connection attempts', async () => {
  const screen = fixture({}, { conversationId: 'chat-1', locked: false, available: false });
  await screen.open();
  expect(screen.getByText(text.unavailable)).toBeTruthy();
  expect(screen.queryByRole('button', { name: text.connect })).toBeNull();
  expect(screen.calls).toEqual([]);
});
test('unknown mutation result refreshes state without retrying the mutation', async () => {
  let connects = 0, reads = 0;
  const screen = fixture({
    async snapshot() { return ++reads === 1 ? null : connected; },
    async connect() { connects++; throw new Error('timeout'); },
  });
  await screen.open(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: text.connect }));
  expect(await screen.findByText(text.requestFailed)).toBeTruthy();
  await screen.findByRole('button', { name: text.chooseTabs }); await screen.ready();
  expect(connects).toBe(1); expect(reads).toBe(2);
});
test('failed state refresh fences mutations until an explicit successful refresh', async () => {
  let reads = 0;
  const screen = fixture({ async snapshot() { if (++reads === 1) throw new Error('offline'); return null; } });
  await screen.open(); await screen.ready();
  expect((screen.getByRole('button', { name: text.connect }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: text.refresh }));
  await waitFor(() => expect((screen.getByRole('button', { name: text.connect }) as HTMLButtonElement).disabled).toBe(false));
  expect(reads).toBe(2); expect(screen.calls).toEqual([]);
});
test('lost connection offers explicit disconnect, never automatic reconnect', async () => {
  const screen = fixture({ async snapshot() { return { ...connected, state: 'connection_lost' }; } });
  await screen.open(); await screen.ready();
  expect(screen.getByText(text.states.connection_lost)).toBeTruthy();
  expect(screen.queryByRole('button', { name: text.connect })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: text.disconnect }));
  await screen.findByRole('button', { name: text.connect }); await screen.ready();
  expect(screen.calls).toEqual([['disconnect', 'chat-1', 'connection-1']]);
  fireEvent.click(screen.getByRole('button', { name: text.connect }));
  await waitFor(() => expect(screen.calls.at(-1)).toEqual(['connect', 'chat-1', 'connection-1']));
});
test('closing the popup or unmounting does not disconnect the user browser', async () => {
  const screen = fixture({ async snapshot() { return connected; } });
  await screen.open(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: text.title }));
  screen.unmount();
  expect(screen.calls).toEqual([]);
});
test('late connect result cannot change another conversation or trigger its refresh', async () => {
  let finish!: (value: SystemBrowserSnapshot) => void;
  const screen = fixture({ async connect() { return new Promise(resolve => { finish = resolve; }); } });
  await screen.open(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: text.connect }));
  screen.change({ conversationId: 'chat-2' });
  await act(async () => finish(connected));
  await screen.open(); await screen.ready();
  expect(screen.getByText(text.states.disconnected)).toBeTruthy();
  expect(screen.queryByRole('button', { name: text.chooseTabs })).toBeNull();
  expect(screen.calls).toEqual([['snapshot', 'chat-1'], ['snapshot', 'chat-2']]);
});
test('an in-flight choice list cannot reappear after Agent lock', async () => {
  let finish!: (value: { tabs: { choice_id: string; title: string; url: string }[] }) => void;
  const screen = fixture({ async snapshot() { return connected; }, async choices() { return new Promise(resolve => { finish = resolve; }); } });
  await screen.open(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: text.chooseTabs }));
  screen.change({ locked: true });
  await act(async () => finish({ tabs: [{ choice_id: 'choice-1', title: selected.title, url: selected.url }] }));
  expect(screen.queryByRole('group', { name: text.availableTabs })).toBeNull();
  screen.change({ locked: false });
  expect(screen.queryByRole('group', { name: text.availableTabs })).toBeNull();
});
test('double clicking connect cannot queue another connection while busy', async () => {
  let finish!: (value: SystemBrowserSnapshot) => void;
  let count = 0;
  const screen = fixture({ async connect() { count++; return new Promise(resolve => { finish = resolve; }); } });
  await screen.open(); await screen.ready();
  const button = screen.getByRole('button', { name: text.connect });
  fireEvent.click(button); fireEvent.click(button);
  expect(screen.getByRole('button', { name: text.cancelConnect })).toBeTruthy();
  expect(count).toBe(1);
  await act(async () => finish(connected));
  expect(count).toBe(1);
});
test('cleanup failure allows only explicit disconnect retry, not reconnect or authorization', async () => {
  const screen = fixture({ async snapshot() { return { ...connected, state: 'cleanup_failed' }; } });
  await screen.open(); await screen.ready();
  expect(screen.queryByRole('button', { name: text.connect })).toBeNull();
  expect(screen.queryByRole('button', { name: text.chooseTabs })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: text.disconnect }));
  await waitFor(() => expect(screen.calls).toEqual([['disconnect', 'chat-1', 'connection-1']]));
});
test('cancel own pending connection GETs its incarnation then DELETEs once despite Preparing lock', async () => {
  let finish!: (value: SystemBrowserSnapshot) => void;
  let reads = 0;
  const screen = fixture({
    async snapshot() { return ++reads === 1 ? null : { ...connected, state: 'connecting' }; },
    async connect() { return new Promise(resolve => { finish = resolve; }); },
  });
  await screen.open(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: text.connect }));
  screen.change({ locked: true });
  const cancel = screen.getByRole('button', { name: text.cancelConnect });
  expect((cancel as HTMLButtonElement).disabled).toBe(false);
  fireEvent.click(cancel); fireEvent.click(cancel);
  await screen.findByRole('button', { name: text.connect });
  expect(screen.calls).toEqual([['disconnect', 'chat-1', 'connection-1']]);
  expect(reads).toBe(2);
  await act(async () => finish(connected));
  expect(screen.queryByRole('button', { name: text.chooseTabs })).toBeNull();
  expect(screen.getByText(text.states.disconnected)).toBeTruthy();
});
test('uncertain cancellation observes status without repeating DELETE or original POST', async () => {
  let finish!: (value: SystemBrowserSnapshot) => void;
  let reads = 0, closes = 0, connects = 0;
  const screen = fixture({
    async snapshot() { return ++reads === 1 ? null : { ...connected, state: reads === 2 ? 'connecting' : 'cleanup_failed' }; },
    async connect() { connects++; return new Promise(resolve => { finish = resolve; }); },
    async disconnect() { closes++; throw new Error('response lost'); },
  });
  await screen.open(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: text.connect }));
  fireEvent.click(screen.getByRole('button', { name: text.cancelConnect }));
  expect(await screen.findByText(text.cancelUnconfirmed)).toBeTruthy();
  await screen.ready();
  expect(closes).toBe(1); expect(connects).toBe(1); expect(reads).toBe(3);
  await act(async () => finish(connected));
  expect(screen.getByText(text.states.cleanup_failed)).toBeTruthy();
});
test('late cancellation lookup cannot disconnect a different conversation', async () => {
  let read!: (value: SystemBrowserSnapshot) => void;
  let finish!: (value: SystemBrowserSnapshot) => void;
  let reads = 0;
  const screen = fixture({
    async snapshot() { return ++reads === 1 ? null : new Promise(resolve => { read = resolve; }); },
    async connect() { return new Promise(resolve => { finish = resolve; }); },
  });
  await screen.open(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: text.connect }));
  fireEvent.click(screen.getByRole('button', { name: text.cancelConnect }));
  screen.change({ conversationId: 'chat-2' });
  await act(async () => { read({ ...connected, state: 'connecting' }); finish(connected); });
  expect(screen.calls).toEqual([]);
});
test('an existing connecting snapshot does not grant cancellation of a request this component did not start', async () => {
  const screen = fixture({ async snapshot() { return { ...connected, state: 'connecting' }; } }, { conversationId: 'chat-1', locked: true, available: true });
  await screen.open(); await screen.ready();
  expect(screen.queryByRole('button', { name: text.cancelConnect })).toBeNull();
  expect((screen.getByRole('button', { name: text.disconnect }) as HTMLButtonElement).disabled).toBe(true);
});
test('cancellation never disconnects an incarnation different from its own completed POST', async () => {
  let read!: (value: SystemBrowserSnapshot) => void;
  let finish!: (value: SystemBrowserSnapshot) => void;
  let reads = 0;
  const screen = fixture({
    async snapshot() { return ++reads === 1 ? null : new Promise(resolve => { read = resolve; }); },
    async connect() { return new Promise(resolve => { finish = resolve; }); },
  });
  await screen.open(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: text.connect }));
  fireEvent.click(screen.getByRole('button', { name: text.cancelConnect }));
  await act(async () => finish(connected));
  await act(async () => read({ ...connected, incarnation: 'new-unrelated-incarnation' }));
  expect(screen.calls).toEqual([]);
  expect(screen.queryByRole('button', { name: text.cancelConnect })).toBeNull();
  expect(screen.getByText(text.cancelUnconfirmed)).toBeTruthy();
});
test('the system browser dialog occludes an overlapping native Browser surface', async () => {
  const original = HTMLElement.prototype.getBoundingClientRect;
  HTMLElement.prototype.getBoundingClientRect = () => ({ x: 20, y: 60, width: 640, height: 500, left: 20, top: 60, right: 660, bottom: 560, toJSON: () => ({}) });
  const visibility: boolean[] = [];
  const native: BrowserSnapshot = { conversation_id: 'chat-1', run: { revision: 1, input_state: 'user_ready', input_gate_failed: false }, runtime: { runtime_generation: 1, revision: 1, active_tab_id: null, tabs: [], downloads: [] } };
  const nativeClient: BrowserClient = { async listenShortcuts() { return () => {}; }, async ensure() { return native; }, async command() { return native; }, async attach() { return 1; }, async update(_id, _seq, _bounds, visible) { visibility.push(visible); }, async detach() {}, async closeWorkspace() {}, async scaleFactor() { return 1; } };
  try {
    const screen = fixture();
    const surface = render(<I18nextProvider i18n={i18n}><BrowserWorkspacePanel conversationId='chat-1' onClose={() => {}} client={nativeClient} /></I18nextProvider>);
    await waitFor(() => expect(visibility.at(-1)).toBe(true));
    await screen.open();
    await waitFor(() => expect(visibility.at(-1)).toBe(false));
    surface.unmount(); screen.unmount();
  } finally { HTMLElement.prototype.getBoundingClientRect = original; }
});
test('the entry is wired only to the exact Nomi Snapshot capability and idle runtime authority', () => {
  const chat = readFileSync(new URL('../components/ChatConversation.tsx', import.meta.url), 'utf8');
  const layout = readFileSync(new URL('../components/ChatLayout/index.tsx', import.meta.url), 'utf8');
  expect(chat).toContain("systemBrowserEnabled: conversation.agent_snapshot?.enabled_capabilities.includes('nomi_system_browser') ?? false");
  expect(chat).toContain("systemBrowserLocked: getConversationRuntimeAuthority(conversation) !== 'idle'");
  expect(layout).toContain('props.systemBrowserEnabled && <SystemBrowserControl');
  expect(layout).toContain('available={isWindowsRuntime}');
});
