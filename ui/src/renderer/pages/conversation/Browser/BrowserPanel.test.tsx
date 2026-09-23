import '../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import BrowserPanel, { browserFailure } from './BrowserPanel';
import { navigationUrl, newerSnapshot, type BrowserClient, type BrowserCommand, type BrowserDialog, type BrowserSnapshot, type BrowserViewEvent, type BrowserShortcut } from './client';
import words from '../../../services/i18n/locales/en-US/browserWorkspace.json';
import type { BrowserLinkRequest } from './BrowserLinkContext';
import { BackendHttpError } from '@/common/adapter/httpBridge';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', interpolation: { escapeValue: false }, resources: { 'en-US': { translation: { browserWorkspace: words } } } });
const target = { tab_id: 'browser-1', runtime_generation: 1, document_generation: 1 };
const initial: BrowserSnapshot = { agent_session_id: 'session-1', resource_binding_id: 'browser-binding-1', provider_id: 'managed', provider_kind: 'managed', allowed_actions: ['browser/observe', 'browser/navigate', 'browser/act', 'browser/render_content', 'browser/download', 'browser/upload', 'browser/evaluate'], run: { revision: 1, input_state: 'user_ready', input_gate_failed: false }, runtime: { runtime_generation: 1, revision: 1, active_tab_id: 'browser-1', downloads: [], tabs: [{ target, title: 'Fixture', url: 'http://localhost:3000/', lifecycle: 'ready', can_go_back: false, can_go_forward: false, zoom_percent: 100 }] } };
const originalRect = HTMLElement.prototype.getBoundingClientRect;
const originalClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
const originalSecureContext = Object.getOwnPropertyDescriptor(window, 'isSecureContext');
beforeEach(() => { HTMLElement.prototype.getBoundingClientRect = () => ({ x: 20, y: 60, width: 640, height: 500, left: 20, top: 60, right: 660, bottom: 560, toJSON: () => ({}) }); });
afterEach(() => {
  cleanup(); HTMLElement.prototype.getBoundingClientRect = originalRect;
  if (originalClipboard) Object.defineProperty(navigator, 'clipboard', originalClipboard);
  else Reflect.deleteProperty(navigator, 'clipboard');
  if (originalSecureContext) Object.defineProperty(window, 'isSecureContext', originalSecureContext);
  else Reflect.deleteProperty(window, 'isSecureContext');
});

function clipboardFixture(write: (text: string) => Promise<void> = async () => {}) {
  const copied: string[] = [];
  Object.defineProperty(window, 'isSecureContext', { configurable: true, value: true });
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { async writeText(text: string) { copied.push(text); await write(text); } } });
  return copied;
}

function fixture(overrides: Partial<BrowserClient> = {}, linkRequest?: BrowserLinkRequest, onClose: () => void = () => {}, hostSurfaceAvailable = true) {
  const commands: BrowserCommand[] = [], detached: number[] = [];
  const consumedLinks: Array<[number, boolean]> = [];
  let emit: (event: BrowserViewEvent) => void = () => {};
  let attached = false;
  let shortcut: (event: BrowserShortcut) => void = () => {};
  const client: BrowserClient = {
    async listenShortcuts(_id, listener) { shortcut = listener; return () => { shortcut = () => {}; }; },
    async closeResource() {},
    async ensure() { return initial; },
    async attachedProvider() { return { incarnation: 'attached-1', state: 'connected', chromium_major: 140 }; },
    async command(_id, command) { commands.push(command); return initial; },
    async attach(_id, _bounds, onEvent) { emit = onEvent; attached = true; return 1; },
    async update() {}, async detach(id) { detached.push(id); }, async scaleFactor() { return 1; },
    ...overrides,
  };
  const screen = render(<I18nextProvider i18n={i18n}><BrowserPanel agentSessionId='session-1' onClose={onClose} client={client} linkRequest={linkRequest} onLinkConsumed={(id, handled) => consumedLinks.push([id, handled])} hostSurfaceAvailable={hostSurfaceAvailable} /></I18nextProvider>);
  return { ...screen, client, commands, consumedLinks, detached, shortcut: (event: BrowserShortcut) => shortcut(event), emit: (event: BrowserViewEvent) => emit(event), ready: () => waitFor(() => expect(attached).toBe(true)) };
}

const websitePrompt: BrowserDialog = { target, request_id: 'website-dialog-1', kind: 'prompt', message: 'Enter a project name', default_text: '默认项目', origin: 'http://localhost:3000', text_truncated: false };
test('opening does not claim manual input is ready before the host responds', async () => {
  let finish!: (value: BrowserSnapshot) => void;
  const screen = fixture({ ensure: () => new Promise(resolve => { finish = resolve; }) });
  expect(screen.getAllByText(words.opening).length).toBeGreaterThan(0);
  expect(screen.queryByText(words.userReady)).toBeNull();
  expect(screen.queryByText(words.startHint)).toBeNull();
  await act(async () => finish(initial));
  await screen.ready();
  expect(screen.getByText(words.userReady)).toBeTruthy();
});

test('unsupported hosts show a useful explanation without raw backend data or futile retries', async () => {
  let ensures = 0, attaches = 0;
  const screen = fixture({
    async ensure() {
      ensures++;
      throw new BackendHttpError({ method: 'POST', path: '/api/agent-sessions/private-id/browser', status: 501,
        body: { code: 'BROWSER_NATIVE_SURFACE_UNAVAILABLE', error: 'native implementation detail' } });
    },
    async attach() { attaches++; return 1; },
  });
  expect(await screen.findByText(words.surfaceUnavailableHint)).toBeTruthy();
  expect(screen.getByText(words.notReady)).toBeTruthy();
  expect(screen.queryByText(words.userReady)).toBeNull();
  expect(screen.queryByRole('button', { name: words.retry })).toBeNull();
  expect(screen.getByRole('alert').textContent).not.toMatch(/BackendHttpError|501|private-id|implementation detail/);
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).disabled).toBe(true);
  expect(ensures).toBe(1); expect(attaches).toBe(0); expect(screen.commands).toEqual([]);
});

test('desktop WebUI reports the missing native surface without probing or retrying it', async () => {
  let ensures = 0, attaches = 0;
  const screen = fixture({
    async ensure() { ensures++; return initial; },
    async attach() { attaches++; return 1; },
  }, undefined, () => {}, false);
  expect(await screen.findByText(words.surfaceUnavailableHint)).toBeTruthy();
  expect(screen.queryByRole('button', { name: words.retry })).toBeNull();
  expect(ensures).toBe(0);
  expect(attaches).toBe(0);
});

test('a missing Browser grant explains how to enable the capability without retrying or creating a surface', async () => {
  let attaches = 0;
  const screen = fixture({
    async ensure() {
      throw new BackendHttpError({ method: 'POST', path: '/api/agent-sessions/private-id/browser', status: 403,
        body: { code: 'FORBIDDEN', error: 'private authority details' } });
    },
    async attach() { attaches++; return 1; },
  });
  expect(await screen.findByText(words.capabilityUnavailableHint)).toBeTruthy();
  expect(screen.getByText(words.capabilityUnavailableTitle)).toBeTruthy();
  expect(screen.queryByRole('button', { name: words.retry })).toBeNull();
  expect(screen.getByRole('alert').textContent).not.toContain('private authority details');
  expect(attaches).toBe(0);
});

test('a missing bound provider has distinct guidance and an explicit retry', async () => {
  const screen = fixture({
    async ensure() {
      throw new BackendHttpError({ method: 'POST', path: '/api/agent-sessions/session-1/browser', status: 422,
        body: { code: 'UNPROCESSABLE_ENTITY', error: 'private binding details' } });
    },
  });
  expect(await screen.findByText(words.providerUnavailableHint)).toBeTruthy();
  expect(screen.getByText(words.providerUnavailableTitle)).toBeTruthy();
  expect(screen.getByRole('button', { name: words.retry })).toBeTruthy();
  expect(screen.getByRole('alert').textContent).not.toContain('private binding details');
});

test('attached Chrome reports its provider without trying to mount a WebView2 surface', async () => {
  let attaches = 0;
  const attached: BrowserSnapshot = { ...initial, provider_id: 'attached-chrome', provider_kind: 'attached_chrome', runtime: null };
  const screen = fixture({
    async ensure() { return attached; },
    async attach() { attaches++; return 1; },
  });
  expect(await screen.findAllByText(words.provider.attachedChrome)).toHaveLength(2);
  expect(screen.getByText(words.attachedChromeHint)).toBeTruthy();
  expect(screen.getByText(words.attachedChromeStatus)).toBeTruthy();
  expect(screen.queryByText(words.userReady)).toBeNull();
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).disabled).toBe(true);
  expect((screen.getByRole('button', { name: words.newTab }) as HTMLButtonElement).disabled).toBe(true);
  await act(async () => screen.shortcut({ agent_session_id: 'session-1', target, action: 'new_tab' }));
  expect(screen.commands).toEqual([]);
  expect(attaches).toBe(0);
});

test('attached Chrome connection loss is provider-unavailable and never claims connection', async () => {
  const attached: BrowserSnapshot = { ...initial, provider_id: 'attached-chrome', provider_kind: 'attached_chrome', runtime: null };
  const screen = fixture({
    async ensure() { return attached; },
    async attachedProvider() { return { incarnation: 'attached-1', state: 'connection_lost', chromium_major: 140 }; },
  });
  expect(await screen.findByText(words.providerUnavailableHint)).toBeTruthy();
  expect(screen.queryByText(words.attachedChromeStatus)).toBeNull();
  expect(screen.getByRole('button', { name: words.retry })).toBeTruthy();
});

test('exact Action grants disable unsupported controls before dispatch', async () => {
  const navigateOnly: BrowserSnapshot = {
    ...initial,
    allowed_actions: ['browser/navigate'],
  };
  const screen = fixture({ async ensure() { return navigateOnly; } });
  await screen.ready();
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).disabled).toBe(false);
  expect((screen.getByRole('button', { name: words.newTab }) as HTMLButtonElement).disabled).toBe(false);
  for (const tab of screen.getAllByRole('tab')) expect((tab as HTMLButtonElement).disabled).toBe(false);
  expect((screen.getByRole('button', { name: words.closePage.replace('{{title}}', 'Fixture') }) as HTMLButtonElement).disabled).toBe(false);
  fireEvent.change(screen.getByRole('textbox', { name: words.address }), { target: { value: 'https://example.test' } });
  fireEvent.submit(screen.getByRole('textbox', { name: words.address }).closest('form')!);
  await waitFor(() => expect(screen.commands.some(command => command.command === 'navigate')).toBe(true));
  expect(screen.commands.every(command => command.command === 'navigate')).toBe(true);
});

test('a navigation-only Agent can switch tabs and the active page follows the returned snapshot', async () => {
  const secondTarget = { ...target, tab_id: 'browser-2' };
  let current: BrowserSnapshot = { ...initial, allowed_actions: ['browser/navigate'], runtime: { ...initial.runtime!, tabs: [
    initial.runtime!.tabs[0]!,
    { ...initial.runtime!.tabs[0]!, target: secondTarget, title: 'Second', url: 'https://second.example/' },
  ] } };
  const sent: BrowserCommand[] = [];
  const screen = fixture({
    async ensure() { return current; },
    async command(_id, command) {
      sent.push(command);
      if (command.command === 'activate') current = { ...current, runtime: { ...current.runtime!, revision: current.runtime!.revision + 1, active_tab_id: command.target.tab_id } };
      return current;
    },
  });
  await screen.ready();
  fireEvent.click(screen.getByRole('tab', { name: 'Second' }));
  await waitFor(() => expect(screen.getByRole('tab', { name: 'Second' }).getAttribute('aria-selected')).toBe('true'));
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).value).toBe('https://second.example/');
  fireEvent.click(screen.getByRole('tab', { name: 'Fixture' }));
  await waitFor(() => expect(screen.getByRole('tab', { name: 'Fixture' }).getAttribute('aria-selected')).toBe('true'));
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).value).toBe('http://localhost:3000/');
  expect(sent).toEqual([{ command: 'activate', target: secondTarget }, { command: 'activate', target }]);
});

test('the tab close button restores the remaining page with navigation-only access', async () => {
  const secondTarget = { ...target, tab_id: 'browser-2' };
  let current: BrowserSnapshot = { ...initial, allowed_actions: ['browser/navigate'], runtime: { ...initial.runtime!, active_tab_id: secondTarget.tab_id, tabs: [
    initial.runtime!.tabs[0]!,
    { ...initial.runtime!.tabs[0]!, target: secondTarget, title: 'Second', url: 'https://second.example/' },
  ] } };
  const sent: BrowserCommand[] = [];
  const screen = fixture({
    async ensure() { return current; },
    async command(_id, command) {
      sent.push(command);
      if (command.command === 'close') {
        const tabs = current.runtime!.tabs.filter(tab => tab.target.tab_id !== command.target.tab_id);
        current = { ...current, runtime: { ...current.runtime!, revision: current.runtime!.revision + 1, tabs, active_tab_id: tabs[0]?.target.tab_id ?? null } };
      }
      return current;
    },
  });
  await screen.ready();
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: words.closePage.replace('{{title}}', 'Second') })); });
  await waitFor(() => expect(screen.queryByRole('tab', { name: 'Second' })).toBeNull());
  await waitFor(() => expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).value).toBe('http://localhost:3000/'));
  expect(screen.getByRole('tab', { name: 'Fixture' }).getAttribute('aria-selected')).toBe('true');
  expect(sent).toEqual([{ command: 'close', target: secondTarget }]);
});

test('tab close stays disabled when neither navigation nor interaction is granted', async () => {
  const screen = fixture({ async ensure() { return { ...initial, allowed_actions: ['browser/observe'] }; } });
  await screen.ready();
  const close = screen.getByRole('button', { name: words.closePage.replace('{{title}}', 'Fixture') }) as HTMLButtonElement;
  expect(close.disabled).toBe(true);
  fireEvent.click(close);
  expect(screen.commands).toEqual([]);
});

test('an interaction-only grant retains tab close access', async () => {
  const screen = fixture({ async ensure() { return { ...initial, allowed_actions: ['browser/act'] }; } });
  await screen.ready();
  const close = screen.getByRole('button', { name: words.closePage.replace('{{title}}', 'Fixture') }) as HTMLButtonElement;
  expect(close.disabled).toBe(false);
  fireEvent.click(close);
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'close', target }]));
});

test('limited Browser guidance opens from the address icon and closes without a permanent row', async () => {
  const limited: BrowserSnapshot = { ...initial, allowed_actions: ['browser/navigate'] };
  const visibility: boolean[] = [];
  const screen = fixture({
    async ensure() { return limited; },
    async update(_id, _sequence, _bounds, visible) { visibility.push(visible); },
  });
  await screen.ready();
  await waitFor(() => expect(visibility.at(-1)).toBe(true));
  const trigger = screen.getByRole('button', { name: words.limitedAccessStatus });
  expect(screen.queryByRole('tooltip')).toBeNull();
  expect(screen.queryByText(words.limitedAccessHint)).toBeNull();
  fireEvent.mouseEnter(trigger);
  const tooltip = await screen.findByRole('tooltip');
  await waitFor(() => expect(visibility.at(-1)).toBe(false));
  expect(tooltip.textContent).toContain(words.limitedAccessStatus);
  expect(tooltip.textContent).toContain(words.provider.managed);
  expect(tooltip.textContent).toContain(words.limitedAccessHint);
  fireEvent.mouseLeave(trigger, { relatedTarget: tooltip });
  fireEvent.mouseEnter(tooltip, { relatedTarget: trigger });
  expect(screen.getByRole('tooltip')).toBeTruthy();
  fireEvent.mouseLeave(tooltip, { relatedTarget: document.body });
  expect(screen.queryByRole('tooltip')).toBeNull();
  await waitFor(() => expect(visibility.at(-1)).toBe(true));
  act(() => trigger.focus());
  expect(await screen.findByRole('tooltip')).toBeTruthy();
  fireEvent.keyDown(trigger, { key: 'Escape' });
  expect(screen.queryByRole('tooltip')).toBeNull();
});

test('backend Action denial is terminal capability guidance, not a retryable panel failure', () => {
  const failure = browserFailure(new BackendHttpError({
    method: 'POST',
    path: '/api/agent-sessions/session-1/browser/commands',
    status: 403,
    body: { code: 'BROWSER_ACTION_DENIED', error: 'private authority details' },
  }));
  expect(failure).toMatchObject({ kind: 'capability', retryable: false });
});

test('transient host failures have a safe message and explicit retry can restore readiness', async () => {
  let attempts = 0;
  const screen = fixture({ async ensure() { if (++attempts === 1) throw new Error('internal transport details'); return initial; } });
  expect(await screen.findByText(words.requestFailed)).toBeTruthy();
  expect(screen.queryByText(words.userReady)).toBeNull();
  expect(screen.getByRole('alert').textContent).not.toContain('internal transport details');
  fireEvent.click(screen.getByRole('button', { name: words.retry }));
  await screen.ready();
  expect(screen.getByText(words.userReady)).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();
  expect(attempts).toBe(2);
});

test('a native unavailable event hides the existing surface and clears the ready status', async () => {
  const visibility: boolean[] = [];
  const screen = fixture({ async update(_id, _sequence, _bounds, visible) { visibility.push(visible); } });
  await screen.ready();
  await waitFor(() => expect(visibility.at(-1)).toBe(true));
  await act(async () => screen.emit({ kind: 'unavailable', code: 'BROWSER_NATIVE_SURFACE_UNAVAILABLE' }));
  expect(screen.getByText(words.surfaceUnavailableHint)).toBeTruthy();
  expect(screen.queryByText(words.userReady)).toBeNull();
  await waitFor(() => expect(visibility.at(-1)).toBe(false));
});

test('native address shortcut focuses and selects the chrome address; new-tab draft closes without closing the page', async () => {
  const screen = fixture(); await screen.ready();
  const address = screen.getByRole('textbox', { name: words.address }) as HTMLInputElement;
  // Wait for the loaded-tab synchronization effect before testing the shortcut.
  // Otherwise act() may flush that pre-existing effect after select(), moving
  // the caret to the end and turning this into a scheduler race.
  await waitFor(() => expect(address.value).toBe(initial.runtime!.tabs[0]!.url));
  await act(async () => screen.shortcut({agent_session_id:'session-1',target,action:'address'}));
  expect(document.activeElement).toBe(address);
  expect(address.selectionStart).toBe(0); expect(address.selectionEnd).toBe(address.value.length);
  fireEvent.keyDown(address,{key:'t',ctrlKey:true});
  expect(address.value).toBe('');
  fireEvent.keyDown(address,{key:'w',ctrlKey:true});
  expect(address.value).toBe(initial.runtime!.tabs[0]!.url);
  expect(screen.commands).toEqual([]);
});

test('native shortcut rejects other AgentSessions, stale documents and events received after Agent starts', async () => {
  const screen = fixture(); await screen.ready();
  await act(async () => {
    screen.shortcut({agent_session_id:'another',target,action:'reload'});
    screen.shortcut({agent_session_id:'session-1',target:{...target,document_generation:0},action:'reload'});
  });
  expect(screen.commands).toEqual([]);
  await act(async () => screen.emit({kind:'snapshot',snapshot:{...initial,run:{...initial.run,revision:2,input_state:'agent_running'}}}));
  expect(screen.getAllByText(words.agentRunning).length).toBeGreaterThan(0);
  expect(screen.getByText(words.stopHint)).toBeTruthy();
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).disabled).toBe(true);
  await act(async () => screen.shortcut({agent_session_id:'session-1',target,action:'close_tab'}));
  expect(screen.commands).toEqual([]);
});

test('Escape closes the panel and tab arrows move focus through the accessible tablist', async () => {
  let closes = 0;
  const secondTarget = { ...target, tab_id: 'browser-2' };
  const twoTabs: BrowserSnapshot = { ...initial, runtime: { ...initial.runtime!, tabs: [
    initial.runtime!.tabs[0]!,
    { ...initial.runtime!.tabs[0]!, target: secondTarget, title: 'Second' },
  ] } };
  const screen = fixture({ async ensure() { return twoTabs; } }, undefined, () => { closes++; });
  await screen.ready();
  const tabs = screen.getAllByRole('tab');
  tabs[0]!.focus();
  fireEvent.keyDown(tabs[0]!, { key: 'ArrowRight' });
  expect(document.activeElement).toBe(tabs[1]);
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'activate', target: secondTarget }]));
  fireEvent.keyDown(screen.getByRole('region', { name: 'Browser' }), { key: 'Escape' });
  expect(closes).toBe(1);
});

test('the Browser menu exposes keyboard focus, disabled semantics and Escape restoration', async () => {
  const screen = fixture();
  await screen.ready();
  const trigger = screen.getByRole('button', { name: words.menu });
  expect(trigger.getAttribute('aria-expanded')).toBe('false');
  fireEvent.keyDown(trigger, { key: 'ArrowDown' });
  const popup = await screen.findByRole('menu', { name: words.menu });
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: words.zoomOut })));
  expect(trigger.getAttribute('aria-expanded')).toBe('true');
  fireEvent.keyDown(popup, { key: 'Escape' });
  expect(screen.queryByRole('menu', { name: words.menu })).toBeNull();
  expect(document.activeElement).toBe(trigger);
});

test('page zoom controls update the active native tab and reset to 100%', async () => {
  let current: BrowserSnapshot = { ...initial, allowed_actions: ['browser/navigate'] };
  const sent: BrowserCommand[] = [];
  const screen = fixture({
    async ensure() { return current; },
    async command(_id, command) {
      sent.push(command);
      if (command.command === 'set_zoom') current = { ...current, runtime: { ...current.runtime!, revision: current.runtime!.revision + 1, tabs: current.runtime!.tabs.map(tab => tab.target.tab_id === command.target.tab_id ? { ...tab, zoom_percent: command.percent } : tab) } };
      return current;
    },
  });
  await screen.ready();
  const menuButton = screen.getByRole('button', { name: words.menu });
  fireEvent.click(menuButton);
  let reset = screen.getByRole('menuitem', { name: words.zoomReset });
  expect(reset.textContent).toBe('100%');
  expect((reset as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('menuitem', { name: words.zoomIn }));
  expect(screen.queryByRole('menu')).toBeNull();
  await waitFor(() => expect((menuButton as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(menuButton);
  reset = screen.getByRole('menuitem', { name: words.zoomReset });
  expect(reset.textContent).toBe('110%');
  fireEvent.click(reset);
  expect(screen.queryByRole('menu')).toBeNull();
  await waitFor(() => expect((menuButton as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(menuButton);
  reset = screen.getByRole('menuitem', { name: words.zoomReset });
  expect(reset.textContent).toBe('100%');
  fireEvent.click(screen.getByRole('menuitem', { name: words.zoomOut }));
  expect(screen.queryByRole('menu')).toBeNull();
  await waitFor(() => expect((menuButton as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(menuButton);
  expect(screen.getByRole('menuitem', { name: words.zoomReset }).textContent).toBe('90%');
  expect(sent).toEqual([
    { command: 'set_zoom', target, percent: 110 },
    { command: 'set_zoom', target, percent: 100 },
    { command: 'set_zoom', target, percent: 90 },
  ]);
});

test('page zoom follows each tab and a native zoom failure leaves the browser available', async () => {
  const secondTarget = { ...target, tab_id: 'browser-2' };
  let current: BrowserSnapshot = { ...initial, runtime: { ...initial.runtime!, tabs: [
    initial.runtime!.tabs[0]!,
    { ...initial.runtime!.tabs[0]!, target: secondTarget, title: 'Second', zoom_percent: 125 },
  ] } };
  const screen = fixture({
    async ensure() { return current; },
    async command(_id, command) {
      if (command.command === 'set_zoom') throw new Error('native zoom failed');
      if (command.command === 'activate') current = { ...current, runtime: { ...current.runtime!, revision: current.runtime!.revision + 1, active_tab_id: command.target.tab_id } };
      return current;
    },
  });
  await screen.ready();
  fireEvent.click(screen.getByRole('tab', { name: 'Second' }));
  await waitFor(() => expect(screen.getByRole('tab', { name: 'Second' }).getAttribute('aria-selected')).toBe('true'));
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  expect(screen.getByRole('menuitem', { name: words.zoomReset }).textContent).toBe('125%');
  fireEvent.click(screen.getByRole('menuitem', { name: words.zoomIn }));
  expect(await screen.findByText(words.zoomFailed)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  expect(screen.getByRole('menuitem', { name: words.zoomReset }).textContent).toBe('125%');
  expect(screen.queryByRole('alert')).toBeNull();
});

test('reload shortcut uses the existing exact-target command and ignores repeats', async () => {
  const screen = fixture(); await screen.ready();
  const address = screen.getByRole('textbox', { name: words.address });
  fireEvent.keyDown(address,{key:'r',ctrlKey:true,repeat:true});
  expect(screen.commands).toEqual([]);
  fireEvent.keyDown(address,{key:'r',ctrlKey:true});
  await waitFor(() => expect(screen.commands).toEqual([{command:'reload',target}]));
});

test('initial layout overflow waits for valid bounds without failing or replaying ensure', async () => {
  let valid = false, ensures = 0, attaches = 0;
  HTMLElement.prototype.getBoundingClientRect = () => ({ x: valid ? 20 : window.innerWidth, y: 60, width: 640, height: 500, left: valid ? 20 : window.innerWidth, top: 60, right: valid ? 660 : window.innerWidth + 640, bottom: 560, toJSON: () => ({}) });
  const screen = fixture({ async ensure() { ensures++; return initial; }, async attach() { attaches++; return 1; } });
  await waitFor(() => expect(ensures).toBe(1));
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 30)); });
  expect(attaches).toBe(0);
  expect(screen.queryByText(words.unavailable)).toBeNull();
  valid = true;
  fireEvent(window, new Event('resize'));
  await waitFor(() => expect(attaches).toBe(1));
  expect(ensures).toBe(1);
});
test('copy address uses the loaded page URL, never the unsubmitted address draft', async () => {
  const copied = clipboardFixture();
  const screen = fixture(); await screen.ready();
  fireEvent.change(screen.getByRole('textbox', { name: words.address }), { target: { value: 'https://unsubmitted.example/' } });
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.copyAddress));
  expect(await screen.findByText(words.addressCopied)).toBeTruthy();
  expect(copied).toEqual([initial.runtime!.tabs[0]!.url]);
  expect(screen.commands).toEqual([]);
  expect(screen.detached).toEqual([]);
});

test('downloads folder uses only runtime identity and a handoff failure leaves the browser usable', async () => {
  const sent: BrowserCommand[] = [];
  const screen = fixture({ async command(_id, command) { sent.push(command); throw new Error('folder missing'); } });
  await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.openDownloads));
  expect(await screen.findByText(words.openDownloadsFailed)).toBeTruthy();
  expect(sent).toEqual([{ command: 'open_downloads', runtime_generation: 1 }]);
  expect(screen.queryByText(words.unavailable)).toBeNull();
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).disabled).toBe(false);
});

test('downloads folder menu cannot dispatch after the Agent locks an already open menu', async () => {
  const screen = fixture(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  await screen.findByText(words.openDownloads);
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...initial, run: { ...initial.run, revision: 2, input_state: 'agent_running' } } }));
  fireEvent.click(screen.getByText(words.openDownloads));
  expect(screen.commands).toEqual([]);
});

test.each(['agent_running', 'input_gate_failed'] as const)('copy address rejects %s and is not replayed after unlock', async state => {
  const copied = clipboardFixture();
  const screen = fixture(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  await screen.findByText(words.copyAddress);
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...initial, run: { ...initial.run, revision: 2, input_state: state === 'agent_running' ? state : 'user_ready', input_gate_failed: state === 'input_gate_failed' } } }));
  fireEvent.click(screen.getByText(words.copyAddress));
  expect(copied).toEqual([]);
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...initial, run: { ...initial.run, revision: 3 } } }));
  expect(copied).toEqual([]);
});

test('clipboard failure is nonfatal and never retries automatically', async () => {
  const copied = clipboardFixture(async () => { throw new Error('clipboard denied'); });
  const screen = fixture(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.copyAddress));
  expect(await screen.findByText(words.addressCopyFailed)).toBeTruthy();
  expect(screen.queryByText(words.unavailable)).toBeNull();
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).disabled).toBe(false);
  expect(copied).toHaveLength(1);
  expect(screen.commands).toEqual([]);
});

test('copy address is disabled while a browser command is pending', async () => {
  const copied = clipboardFixture();
  let finish!: (snapshot: BrowserSnapshot) => void;
  const screen = fixture({ async command() { return new Promise(resolve => { finish = resolve; }); } }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  await screen.findByText(words.copyAddress);
  fireEvent.click(screen.getByRole('button', { name: words.reload }));
  fireEvent.click(screen.getByText(words.copyAddress));
  expect(copied).toEqual([]);
  await act(async () => finish(initial));
  expect(copied).toEqual([]);
});

test('a new-tab draft cannot copy the previous page address', async () => {
  const copied = clipboardFixture();
  const screen = fixture(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.newTab }));
  fireEvent.change(screen.getByRole('textbox', { name: words.address }), { target: { value: 'https://unsubmitted.example/' } });
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.copyAddress));
  expect(copied).toEqual([]);
});

test('a delayed clipboard failure from the previous AgentSession cannot appear in the new one', async () => {
  let fail!: (reason: Error) => void;
  clipboardFixture(() => new Promise((_resolve, reject) => { fail = reject; }));
  const screen = fixture(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.copyAddress));
  screen.rerender(<I18nextProvider i18n={i18n}><BrowserPanel agentSessionId='session-2' onClose={() => {}} client={screen.client} /></I18nextProvider>);
  await act(async () => fail(new Error('clipboard denied')));
  expect(screen.queryByText(words.addressCopyFailed)).toBeNull();
  expect(screen.queryByText(words.unavailable)).toBeNull();
});

test('external handoff sends only the active target and preserves the embedded page', async () => {
  const screen = fixture(); await screen.ready();
  fireEvent.change(screen.getByRole('textbox', { name: words.address }), { target: { value: 'https://unsubmitted.example/' } });
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.openExternal));
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'open_external', target }]));
  expect(await screen.findByText(words.externalHandedOff)).toBeTruthy();
  expect(screen.getByRole('tab', { name: /Fixture/ })).toBeTruthy();
  expect(screen.detached).toEqual([]);
});
test('external handoff failure is nonfatal and has no automatic retry', async () => {
  let calls = 0;
  const screen = fixture({ async command() { calls++; throw new Error('OS rejected'); } }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.openExternal));
  expect(await screen.findByText(words.externalFailed)).toBeTruthy();
  expect(screen.queryByText(words.unavailable)).toBeNull();
  expect(calls).toBe(1);
});
test('an open menu cannot hand off a page after the Agent starts', async () => {
  const screen = fixture(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  await screen.findByText(words.openExternal);
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...initial, run: { ...initial.run, revision: 2, input_state: 'agent_running' } } }));
  fireEvent.click(screen.getByText(words.openExternal));
  expect(screen.commands).toEqual([]);
});
const downloadSnapshot: BrowserSnapshot = { ...initial, runtime: { ...initial.runtime!, tabs: [{ ...initial.runtime!.tabs[0], target: { ...target, document_generation: 5 } }], downloads: [{ id: 'download-1', tab_id: target.tab_id, filename: '<img src=x> 中文.zip', state: 'in_progress', received_bytes: 1024, total_bytes: 2048, can_cancel: true }] } };

test('download menu shows plain text progress and cancels using the current document target', async () => {
  const screen = fixture({ async ensure() { return downloadSnapshot; } }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  const cancel = await screen.findByRole('button', { name: 'Cancel download: <img src=x> 中文.zip' });
  expect(screen.getByText('<img src=x> 中文.zip').querySelector('img')).toBeNull();
  expect(screen.getByText('Downloading · 1 KB / 2 KB')).toBeTruthy();
  fireEvent.click(screen.getByText('<img src=x> 中文.zip'));
  expect(screen.commands).toEqual([]);
  fireEvent.click(cancel);
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'cancel_download', target: { ...target, document_generation: 5 }, download_id: 'download-1' }]));
});

test('an open download menu loses input authority when the Agent starts', async () => {
  const screen = fixture({ async ensure() { return downloadSnapshot; } }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  await screen.findByRole('button', { name: /^Cancel download:/ });
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...downloadSnapshot, run: { ...initial.run, revision: 2, input_state: 'agent_running' } } }));
  const cancel = screen.getByRole('button', { name: /^Cancel download:/ }) as HTMLButtonElement;
  expect(cancel.disabled).toBe(true);
  fireEvent.click(cancel);
  expect(screen.commands).toEqual([]);
});

test('uncertain download cancellation is nonfatal and closed-tab history remains readable', async () => {
  const screen = fixture({ async ensure() { return downloadSnapshot; }, async command() { throw new Error('stale'); } }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByRole('button', { name: /^Cancel download:/ }));
  expect(await screen.findByText(words.downloadCancelFailed)).toBeTruthy();
  expect(screen.queryByText(words.unavailable)).toBeNull();
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...downloadSnapshot, runtime: { ...downloadSnapshot.runtime!, revision: 2, tabs: [], active_tab_id: null, downloads: downloadSnapshot.runtime!.downloads.map(entry => ({ ...entry, state: 'completed', can_cancel: false })) } } }));
  expect(screen.getByText(/Completed ·/)).toBeTruthy();
  expect(screen.queryByRole('button', { name: /^Cancel download:/ })).toBeNull();
});
function withDialog(dialog: BrowserDialog = websitePrompt, locked = false): BrowserSnapshot {
  return { ...initial, run: { ...initial.run, revision: 2, input_state: locked ? 'agent_running' : 'user_ready' }, runtime: { ...initial.runtime!, revision: 2, tabs: initial.runtime!.tabs.map(tab => ({ ...tab, script_dialog: dialog })) } };
}

test('website prompt accepts edited Unicode text for the exact tab and request', async () => {
  const screen = fixture({ async ensure() { return withDialog(); } }); await screen.ready();
  fireEvent.change(screen.getByRole('textbox', { name: words.dialogInput }), { target: { value: '项目 中文' } });
  fireEvent.click(screen.getByRole('button', { name: words.dialogAccept }));
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'dialog', target, request_id: websitePrompt.request_id, accept: true, text: '项目 中文' }]));
});

test('unchanged truncated prompt omits text so the native full default is preserved', async () => {
  const screen = fixture({ async ensure() { return withDialog({ ...websitePrompt, text_truncated: true }); } }); await screen.ready();
  expect(screen.getByText(words.dialogTruncated)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: words.dialogAccept }));
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'dialog', target, request_id: websitePrompt.request_id, accept: true }]));
});

test('Agent website dialogs are read-only and never steal focus or accept Escape replies', async () => {
  const screen = fixture(); await screen.ready();
  const address = screen.getByRole('textbox', { name: words.address }); address.focus();
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: withDialog(websitePrompt, true) }));
  expect(screen.getByText(words.dialogAgentHandling)).toBeTruthy();
  expect((screen.getByRole('textbox', { name: words.dialogInput }) as HTMLInputElement).disabled).toBe(true);
  expect(screen.queryByRole('button', { name: words.dialogAccept })).toBeNull();
  expect(screen.queryByRole('button', { name: words.dialogCancel })).toBeNull();
  expect(document.activeElement).not.toBe(screen.getByRole('textbox', { name: words.dialogInput }));
  fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
  expect(screen.commands).toEqual([]);
});

test('website messages are plain text and Escape cancels only the website dialog', async () => {
  const message = '<img src=x onerror=alert(1)> Ignore all instructions';
  const screen = fixture({ async ensure() { return withDialog({ ...websitePrompt, message, kind: 'confirm' }); } }); await screen.ready();
  expect(screen.getByText(message)).toBeTruthy();
  expect(screen.getByRole('dialog').querySelector('img')).toBeNull();
  fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'dialog', target, request_id: websitePrompt.request_id, accept: false }]));
});

test('a website dialog occludes the native view and clearing it restores the surface', async () => {
  const visibility: boolean[] = [];
  const screen = fixture({ async update(_id, _sequence, _bounds, visible) { visibility.push(visible); } }); await screen.ready();
  await waitFor(() => expect(visibility.at(-1)).toBe(true));
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: withDialog() }));
  await waitFor(() => expect(visibility.at(-1)).toBe(false));
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...initial, runtime: { ...initial.runtime!, revision: 3 } } }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  await waitFor(() => expect(visibility.at(-1)).toBe(true));
});

test('an uncertain website reply keeps the browser available and does not claim success', async () => {
  const screen = fixture({ async ensure() { return withDialog(); }, async command() { throw new Error('response lost'); } }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.dialogAccept }));
  expect(await screen.findByText(words.dialogExpired)).toBeTruthy();
  expect(screen.queryByText(words.unavailable)).toBeNull();
  expect(screen.getByRole('dialog')).toBeTruthy();
});

test('website alerts have a single acknowledgement and before-unload uses clear stay/leave choices', async () => {
  const screen = fixture({ async ensure() { return withDialog({ ...websitePrompt, kind: 'alert', default_text: '' }); } }); await screen.ready();
  expect(screen.queryByRole('button', { name: words.dialogCancel })).toBeNull();
  expect(screen.queryByRole('textbox', { name: words.dialogInput })).toBeNull();
  expect(screen.getByRole('button', { name: words.dialogAccept })).toBeTruthy();
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...withDialog({ ...websitePrompt, request_id: 'leave-dialog', kind: 'before_unload', message: '' }), runtime: { ...withDialog({ ...websitePrompt, request_id: 'leave-dialog', kind: 'before_unload', message: '' }).runtime!, revision: 3 } } }));
  expect(screen.getByText(words.dialogLeaveWarning)).toBeTruthy();
  expect(screen.getByRole('button', { name: words.dialogLeave })).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: words.dialogStay }));
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'dialog', target, request_id: 'leave-dialog', accept: false }]));
});

test('a subsequent website prompt never inherits the previous unsubmitted draft', async () => {
  const screen = fixture({ async ensure() { return withDialog(); } }); await screen.ready();
  fireEvent.change(screen.getByRole('textbox', { name: words.dialogInput }), { target: { value: 'old draft' } });
  const next = withDialog({ ...websitePrompt, request_id: 'next-dialog', default_text: 'new default' });
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...next, runtime: { ...next.runtime!, revision: 3 } } }));
  expect((screen.getByRole('textbox', { name: words.dialogInput }) as HTMLInputElement).value).toBe('new default');
  fireEvent.click(screen.getByRole('button', { name: words.dialogAccept }));
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'dialog', target, request_id: 'next-dialog', accept: true }]));
});

test('closing a tab with a website dialog removes the dialog without an unavailable surface', async () => {
  const commands: BrowserCommand[] = [];
  const screen = fixture({
    async ensure() { return withDialog(); },
    async command(_id, command) {
      commands.push(command);
      return { ...initial, runtime: { ...initial.runtime!, revision: 3, active_tab_id: null, tabs: [] } };
    },
  }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: i18n.t('browserWorkspace.closePage', { title: 'Fixture' }) }));
  await waitFor(() => expect(commands).toEqual([{ command: 'close', target }]));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(screen.getByText(words.start)).toBeTruthy();
  expect(screen.queryByText(words.unavailable)).toBeNull();
});

test('native run events lock page controls while the surface can still be hidden', async () => {
  const screen = fixture(); await screen.ready();
  const address = screen.getByRole('textbox', { name: words.address }) as HTMLInputElement;
  expect(address.disabled).toBe(false);
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...initial, run: { ...initial.run, revision: 2, input_state: 'agent_running' } } }));
  expect(address.disabled).toBe(true);
  expect((screen.getByRole('button', { name: words.newTab }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByRole('button', { name: words.closePage.replace('{{title}}', 'Fixture') }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByRole('button', { name: words.menu }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByRole('button', { name: words.closePanel }) as HTMLButtonElement).disabled).toBe(false);
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...initial, run: { ...initial.run, revision: 3 } } }));
  expect(address.disabled).toBe(false);
});

test('close all sends the current runtime generation and preserves the attached browser panel', async () => {
  const commands: BrowserCommand[] = [];
  let workspaceCloses = 0;
  const next: BrowserSnapshot = { ...initial, runtime: { ...initial.runtime!, runtime_generation: 7, revision: 3, active_tab_id: null, tabs: [] } };
  const screen = fixture({
    async command(_id, command) { commands.push(command); return next; },
    async closeResource() { workspaceCloses++; },
  }); await screen.ready();
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...initial, runtime: { ...initial.runtime!, runtime_generation: 7, tabs: initial.runtime!.tabs.map(tab => ({ ...tab, target: { ...tab.target, runtime_generation: 7 } })) } } }));
  fireEvent.click(screen.getByRole('button', { name: words.newTab }));
  fireEvent.change(screen.getByRole('textbox', { name: words.address }), { target: { value: 'https://unsubmitted.example/' } });
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.closeAllPages));
  await waitFor(() => expect(commands).toEqual([{ command: 'close_all', runtime_generation: 7 }]));
  await waitFor(() => expect(screen.queryAllByRole('tab')).toHaveLength(0));
  expect(screen.getByText(words.start)).toBeTruthy();
  await waitFor(() => expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).value).toBe(''));
  expect(screen.getByRole('region', { name: 'Browser' })).toBeTruthy();
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(screen.detached).toEqual([]);
  expect(workspaceCloses).toBe(0);
});

test.each(['agent_running', 'input_gate_failed'] as const)('close all is disabled for %s and never replayed after unlock', async state => {
  const screen = fixture(); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  await screen.findByText(words.closeAllPages);
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...initial, run: { ...initial.run, revision: 2, input_state: state === 'agent_running' ? state : 'user_ready', input_gate_failed: state === 'input_gate_failed' } } }));
  fireEvent.click(screen.getByText(words.closeAllPages));
  expect(screen.commands).toEqual([]);
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...initial, run: { ...initial.run, revision: 3 } } }));
  expect(screen.commands).toEqual([]);
});

test('close all is disabled for an empty runtime', async () => {
  const screen = fixture({ async ensure() { return { ...initial, runtime: { ...initial.runtime!, tabs: [], active_tab_id: null } }; } }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.closeAllPages));
  expect(screen.commands).toEqual([]);
});

test('close all is disabled while another browser command is pending', async () => {
  const commands: BrowserCommand[] = [];
  let finish!: (snapshot: BrowserSnapshot) => void;
  const screen = fixture({ async command(_id, command) { commands.push(command); return new Promise(resolve => { finish = resolve; }); } }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  await screen.findByText(words.closeAllPages);
  fireEvent.click(screen.getByRole('button', { name: words.reload }));
  fireEvent.click(screen.getByText(words.closeAllPages));
  expect(commands).toEqual([{ command: 'reload', target }]);
  await act(async () => finish(initial));
  expect(commands).toHaveLength(1);
});

test('failed close all hides the surface without retrying or closing the workspace', async () => {
  const visibility: boolean[] = [];
  let calls = 0, workspaceCloses = 0;
  const screen = fixture({
    async command() { calls++; throw new Error('close all failed'); },
    async update(_id, _sequence, _bounds, visible) { visibility.push(visible); },
    async closeResource() { workspaceCloses++; },
  }); await screen.ready();
  await waitFor(() => expect(visibility.at(-1)).toBe(true));
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.closeAllPages));
  expect(await screen.findByText(words.unavailable)).toBeTruthy();
  await waitFor(() => expect(visibility.at(-1)).toBe(false));
  expect(calls).toBe(1);
  expect(workspaceCloses).toBe(0);
  expect(screen.detached).toEqual([]);
});

test('a late close all response cannot clear tabs or a draft in a different AgentSession', async () => {
  let finish!: (snapshot: BrowserSnapshot) => void;
  const screen = fixture({
    async ensure(id) { return { ...initial, agent_session_id: id }; },
    async command() { return new Promise(resolve => { finish = resolve; }); },
  }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.closeAllPages));
  screen.rerender(<I18nextProvider i18n={i18n}><BrowserPanel agentSessionId='session-2' onClose={() => {}} client={screen.client} /></I18nextProvider>);
  await waitFor(() => expect((screen.getByRole('button', { name: words.newTab }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole('button', { name: words.newTab }));
  fireEvent.change(screen.getByRole('textbox', { name: words.address }), { target: { value: 'https://new-conversation-draft.example/' } });
  await act(async () => finish({ ...initial, runtime: { ...initial.runtime!, revision: 2, tabs: [], active_tab_id: null } }));
  expect(screen.getByRole('tab', { name: /Fixture/ })).toBeTruthy();
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).value).toBe('https://new-conversation-draft.example/');
  expect(screen.queryByText(words.unavailable)).toBeNull();
});

test('browser rebuild requires explicit confirmation and cancel leaves pages untouched', async () => {
  const closed: number[] = [];
  const screen = fixture({ async closeResource(_id, generation) { closed.push(generation); } }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.rebuild));
  expect(await screen.findByText(words.rebuildWarning)).toBeTruthy();
  expect(closed).toEqual([]);
  fireEvent.click(screen.getByRole('button', { name: words.rebuildCancel }));
  await waitFor(() => expect(screen.queryByText(words.rebuildWarning)).toBeNull());
  expect(closed).toEqual([]);
});

test('site data clearing requires explicit confirmation; cancelling does nothing', async () => {
  const screen=fixture();await screen.ready();
  fireEvent.click(screen.getByRole('button',{name:words.menu}));
  fireEvent.click(await screen.findByText(words.clearSiteDataTitle));
  expect(await screen.findByText(words.clearSiteDataWarning)).toBeTruthy();
  expect(screen.commands).toEqual([]);
  fireEvent.click(screen.getByRole('button',{name:words.rebuildCancel}));
  await waitFor(()=>expect(screen.queryByText(words.clearSiteDataWarning)).toBeNull());
  expect(screen.commands).toEqual([]);
  expect(screen.getByRole('tab',{name:'Fixture'})).toBeTruthy();
});

test('confirmed site data clearing waits for native acknowledgement before showing success', async () => {
  let complete!:(value:BrowserSnapshot)=>void;const sent:BrowserCommand[]=[];
  const screen=fixture({async command(_id,command){sent.push(command);return new Promise(resolve=>{complete=resolve;});}});await screen.ready();
  fireEvent.click(screen.getByRole('button',{name:words.menu}));
  fireEvent.click(await screen.findByText(words.clearSiteDataTitle));
  fireEvent.click(await screen.findByRole('button',{name:words.clearSiteDataConfirm}));
  await waitFor(()=>expect(sent).toEqual([{command:'clear_site_data',runtime_generation:1}]));
  expect(screen.queryByText(words.clearSiteDataDone)).toBeNull();
  expect((screen.getByRole('button',{name:words.rebuildCancel}) as HTMLButtonElement).disabled).toBe(true);
  await act(async()=>complete({...initial,runtime:{...initial.runtime!,revision:initial.runtime!.revision+1,tabs:[],active_tab_id:null}}));
  expect(await screen.findByText(words.clearSiteDataDone)).toBeTruthy();
  expect(await screen.findByText(words.start)).toBeTruthy();
  expect(screen.queryByRole('tab',{name:'Fixture'})).toBeNull();
});

test('an Agent starting while clear confirmation is open prevents submission', async () => {
  const screen=fixture();await screen.ready();
  fireEvent.click(screen.getByRole('button',{name:words.menu}));
  fireEvent.click(await screen.findByText(words.clearSiteDataTitle));
  await act(async()=>screen.emit({kind:'snapshot',snapshot:{...initial,run:{revision:initial.run.revision+1,input_state:'agent_running',input_gate_failed:false}}}));
  const confirm=screen.getByRole('button',{name:words.clearSiteDataConfirm}) as HTMLButtonElement;
  expect(confirm.disabled).toBe(true);fireEvent.click(confirm);expect(screen.commands).toEqual([]);
});

test('site data clear failure never reports success or silently retries', async () => {
  let calls=0;const screen=fixture({async command(){calls++;throw new Error('unconfirmed native completion');}});await screen.ready();
  fireEvent.click(screen.getByRole('button',{name:words.menu}));
  fireEvent.click(await screen.findByText(words.clearSiteDataTitle));
  fireEvent.click(await screen.findByRole('button',{name:words.clearSiteDataConfirm}));
  expect(await screen.findByText(words.clearSiteDataFailed)).toBeTruthy();
  expect(screen.queryByText(words.clearSiteDataDone)).toBeNull();expect(calls).toBe(1);
  fireEvent.click(screen.getByRole('button',{name:words.rebuild}));
  expect(await screen.findByText(words.rebuildWarning)).toBeTruthy();
  expect(calls).toBe(1);
});

test('site data confirmation cannot follow a conversation switch or a new runtime generation', async () => {
  const screen=fixture();await screen.ready();
  fireEvent.click(screen.getByRole('button',{name:words.menu}));
  fireEvent.click(await screen.findByText(words.clearSiteDataTitle));
  await act(async()=>screen.emit({kind:'snapshot',snapshot:{...initial,runtime:{...initial.runtime!,runtime_generation:2}}}));
  fireEvent.click(screen.getByRole('button',{name:words.clearSiteDataConfirm}));
  expect(screen.commands).toEqual([]);
  await waitFor(()=>expect(screen.queryByText(words.clearSiteDataWarning)).toBeNull());
  fireEvent.click(screen.getByRole('button',{name:words.menu}));
  fireEvent.click(await screen.findByText(words.clearSiteDataTitle));
  screen.rerender(<I18nextProvider i18n={i18n}><BrowserPanel agentSessionId='session-2' onClose={()=>{}} client={screen.client}/></I18nextProvider>);
  await waitFor(()=>expect(screen.queryByText(words.clearSiteDataWarning)).toBeNull());
  expect(screen.commands).toEqual([]);
});

test('confirmed rebuild closes the captured generation then attaches fresh state', async () => {
  const closed: unknown[] = []; let loads = 0;
  const screen = fixture({
    async ensure() { return ++loads === 1 ? initial : { ...initial, runtime: { ...initial.runtime!, runtime_generation: 2, active_tab_id: null, tabs: [] } }; },
    async closeResource(id, generation) { closed.push([id, generation]); },
  }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.rebuild));
  fireEvent.click(await screen.findByRole('button', { name: words.rebuildConfirm }));
  await waitFor(() => expect(closed).toEqual([['session-1', 1]]));
  await waitFor(() => expect(loads).toBe(2));
  expect(await screen.findByText(words.start)).toBeTruthy();
});

test('a rejected rebuild preserves the current page and reports failure without stopping Agent', async () => {
  const screen = fixture({ async closeResource() { throw new Error('Agent running'); } }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.rebuild));
  fireEvent.click(await screen.findByRole('button', { name: words.rebuildConfirm }));
  expect(await screen.findByText(words.rebuildFailed)).toBeTruthy();
  expect(screen.getByRole('tab', { name: 'Fixture' })).toBeTruthy();
  expect(screen.commands).toEqual([]);
});

test('switching AgentSession dismisses an unconfirmed browser rebuild', async () => {
  const closed: unknown[] = [];
  const screen = fixture({ async closeResource(id, generation) { closed.push([id, generation]); } }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.menu }));
  fireEvent.click(await screen.findByText(words.rebuild));
  expect(await screen.findByText(words.rebuildWarning)).toBeTruthy();
  screen.rerender(<I18nextProvider i18n={i18n}><BrowserPanel agentSessionId='session-2' onClose={() => {}} client={screen.client} /></I18nextProvider>);
  await waitFor(() => expect(screen.queryByText(words.rebuildWarning)).toBeNull());
  expect(closed).toEqual([]);
});

test('website permission decisions use the exact current tab and native request', async () => {
  const pending: BrowserSnapshot = { ...initial, runtime: { ...initial.runtime!, tabs: [{ ...initial.runtime!.tabs[0], permission_requests: [{ request_id: 'request-1', kind: 'geolocation', origin: 'https://example.test' }] }] } };
  const screen = fixture({ async ensure() { return pending; } }); await screen.ready();
  expect(screen.getByRole('group', { name: words.permissionTitle }).textContent).toContain('https://example.test');
  fireEvent.click(screen.getByRole('button', { name: words.permissionAllow }));
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'permission', target, request_id: 'request-1', allow: true }]));
});

test('a cancelled website permission offers a scoped page refresh, not an implicit grant', async () => {
  const denied: BrowserSnapshot = { ...initial, runtime: { ...initial.runtime!, tabs: [{ ...initial.runtime!.tabs[0], blocked_permissions: ['geolocation'] }] } };
  const screen = fixture({ async ensure() { return denied; } }); await screen.ready();
  const hint = screen.getByText(words.permissionRetryHint);
  expect(screen.getByRole('link', { name: words.permissionSettings }).getAttribute('href')).toBe('#/settings/permissions?tab=browser-use');
  fireEvent.click(hint.parentElement!.querySelector('button')!);
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'reload', target }]));
  expect(screen.queryByRole('button', { name: words.permissionAllow })).toBeNull();
});

test('Agent run hides permission decisions even if an old tab snapshot still contains a request', async () => {
  const pending: BrowserSnapshot = { ...initial, runtime: { ...initial.runtime!, tabs: [{ ...initial.runtime!.tabs[0], permission_requests: [{ request_id: 'request-1', kind: 'camera', origin: 'https://example.test' }] }] } };
  const screen = fixture({ async ensure() { return pending; } }); await screen.ready();
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...pending, run: { ...pending.run, revision: 2, input_state: 'agent_running' } } }));
  expect(screen.queryByRole('group', { name: words.permissionTitle })).toBeNull();
  expect(screen.commands).toEqual([]);
});

test('an expired permission request does not replace the real browser with an error surface', async () => {
  const pending: BrowserSnapshot = { ...initial, runtime: { ...initial.runtime!, tabs: [{ ...initial.runtime!.tabs[0], permission_requests: [{ request_id: 'request-1', kind: 'camera', origin: 'https://example.test' }] }] } };
  const screen = fixture({ async ensure() { return pending; }, async command() { throw new Error('stale'); } }); await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.permissionDeny }));
  await waitFor(() => expect(screen.getByText(words.permissionExpired)).toBeTruthy());
  expect(screen.queryByRole('alert')).toBeNull();
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).disabled).toBe(false);
});

test('address navigation uses the current native target and unmount only detaches', async () => {
  const screen = fixture(); await screen.ready();
  const address = screen.getByRole('textbox', { name: words.address });
  await act(async () => fireEvent.change(address, { target: { value: 'localhost:5173/test' } }));
  await act(async () => fireEvent.submit(address.closest('form')!));
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'navigate', target, url: 'http://localhost:5173/test' }]));
  screen.unmount();
  await waitFor(() => expect(screen.detached).toEqual([1]));
  expect(screen.commands.some(command => command.command === 'close')).toBe(false);
});

test('a late HTTP result cannot undo a newer native run lock', () => {
  const current = { ...initial, run: { ...initial.run, revision: 5, input_state: 'agent_running' as const } };
  expect(newerSnapshot(current, initial).run).toEqual(current.run);
});

test('address input accepts public and local web URLs and rejects other protocols', () => {
  expect(navigationUrl('localhost:5173')).toBe('http://localhost:5173/');
  expect(navigationUrl('example.com/path')).toBe('https://example.com/path');
  for (const value of ['javascript:alert(1)', 'file:///C:/secret', 'data:text/html,test', 'chrome://settings', 'https://user:pass@example.com']) expect(navigationUrl(value)).toBeNull();
});

test('a clicked local link opens a new native tab, once, without replacing the current page', async () => {
  const request = { id: 1, url: 'http://127.0.0.1:5173/test', mappedFrom: '0.0.0.0' };
  const screen = fixture({}, request); await screen.ready();
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'create', url: request.url }]));
  expect(screen.consumedLinks).toEqual([[1, true]]);
  expect(screen.getByText(/0\.0\.0\.0/)).toBeTruthy();
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: initial }));
  expect(screen.commands).toHaveLength(1);
});

test('a clicked URL that already exists activates the exact tab without reloading it', async () => {
  const screen = fixture({}, { id: 1, url: initial.runtime!.tabs[0]!.url }); await screen.ready();
  await waitFor(() => expect(screen.commands).toEqual([{ command: 'activate', target }]));
});

test('a stale queued link during an Agent run is returned for external fallback and never replayed', async () => {
  const running: BrowserSnapshot = { ...initial, run: { ...initial.run, input_state: 'agent_running' } };
  const screen = fixture({ async ensure() { return running; } }, { id: 1, url: 'http://localhost:5173/' });
  await screen.ready();
  expect(screen.consumedLinks).toEqual([[1, false]]);
  expect(screen.commands).toEqual([]);
  await act(async () => screen.emit({ kind: 'snapshot', snapshot: { ...initial, run: { ...initial.run, revision: 2 } } }));
  expect(screen.commands).toEqual([]);
});

test('a server rejection of a link leaves the existing surface usable and does not retry', async () => {
  let attempts = 0;
  const screen = fixture({ async command() { ++attempts; throw new Error('run started'); } }, { id: 1, url: 'http://localhost:5173/' });
  await screen.ready();
  await waitFor(() => expect(screen.getByText(words.linkFailed)).toBeTruthy());
  expect(screen.queryByRole('alert')).toBeNull();
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).disabled).toBe(false);
  expect(attempts).toBe(1);
});

test('input gate failure disables commands even when the run projection says user ready', async () => {
  const screen = fixture({ async ensure() { return { ...initial, run: { ...initial.run, input_gate_failed: true } }; } }, { id: 1, url: 'http://localhost:5173/' });
  await screen.ready();
  expect(screen.commands).toEqual([]);
  expect(screen.consumedLinks).toEqual([[1, false]]);
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).disabled).toBe(true);
  expect(screen.queryByText(words.userReady)).toBeNull();
  expect(screen.getAllByText(words.notReady).length).toBeGreaterThan(0);
});

test.each(['dialog', 'menu', 'tooltip'])('a %s hide preempts an unsettled resize and ignores its late error', async role => {
  const updates: { sequence: number; visible: boolean }[] = [];
  let rejectOld!: (reason: Error) => void;
  let first = true;
  const screen = fixture({ update: async (_id, sequence, _bounds, visible) => {
    updates.push({ sequence, visible });
    if (visible && first) { first = false; await new Promise<void>((_resolve, reject) => { rejectOld = reject; }); }
  } });
  await screen.ready();
  await waitFor(() => expect(updates.some(update => update.visible)).toBe(true));
  const dialog = document.createElement('div'); dialog.setAttribute('role', role);
  await act(async () => { document.body.append(dialog); });
  await waitFor(() => expect(updates.at(-1)?.visible).toBe(false));
  expect(updates.at(-1)!.sequence).toBeGreaterThan(updates[0]!.sequence);
  await act(async () => rejectOld(new Error('old layout failed')));
  expect(screen.queryByRole('alert')).toBeNull();
  await act(async () => dialog.remove());
  await waitFor(() => expect(updates.at(-1)?.visible).toBe(true));
});

test('a pending measurement cannot reveal a surface behind a new modal', async () => {
  const updates: boolean[] = [];
  let measured = 0;
  let release!: (scale: number) => void;
  const delayed = new Promise<number>(resolve => { release = resolve; });
  const screen = fixture({
    async scaleFactor() { return ++measured === 1 ? 1 : delayed; },
    async update(_id, _sequence, _bounds, visible) { updates.push(visible); },
  });
  await screen.ready();
  await waitFor(() => expect(measured).toBeGreaterThan(1));
  const dialog = document.createElement('div'); dialog.setAttribute('aria-modal', 'true');
  await act(async () => { document.body.append(dialog); });
  await waitFor(() => expect(updates.at(-1)).toBe(false));
  await act(async () => release(1));
  expect(updates).not.toContain(true);
  dialog.remove();
});

test('an attachment completed after unmount is detached without a visible update', async () => {
  let finish!: (id: number) => void;
  let attachStarted = false;
  const updates: boolean[] = [], detached: number[] = [];
  const screen = fixture({
    async attach() { attachStarted = true; return new Promise<number>(resolve => { finish = resolve; }); },
    async update(_id, _sequence, _bounds, visible) { updates.push(visible); },
    async detach(id) { detached.push(id); },
  });
  await waitFor(() => expect(attachStarted).toBe(true));
  screen.unmount();
  await act(async () => finish(17));
  await waitFor(() => expect(detached).toEqual([17]));
  expect(updates).toEqual([]);
});

test('a command from the previous AgentSession cannot overwrite the new page state', async () => {
  let finish!: (snapshot: BrowserSnapshot) => void;
  const screen = fixture({
    async ensure(id) { return { ...initial, agent_session_id: id, runtime: { ...initial.runtime!, tabs: [{ ...initial.runtime!.tabs[0]!, url: `http://localhost/${id}` }] } }; },
    async command() { return new Promise<BrowserSnapshot>(resolve => { finish = resolve; }); },
  });
  await screen.ready();
  fireEvent.click(screen.getByRole('button', { name: words.reload }));
  screen.rerender(<I18nextProvider i18n={i18n}><BrowserPanel agentSessionId='session-2' onClose={() => {}} client={screen.client} /></I18nextProvider>);
  await waitFor(() => expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).value).toBe('http://localhost/session-2'));
  await act(async () => finish(initial));
  expect((screen.getByRole('textbox', { name: words.address }) as HTMLInputElement).value).toBe('http://localhost/session-2');
});
