import '../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import BrowserResourceField from './BrowserResourceField';
import { attachedChromeClient, type AttachedChromeClient, type AttachedChromeSnapshot } from './attachedChromeClient';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en } } }, interpolation: { escapeValue: false } });
const connected: AttachedChromeSnapshot = { incarnation: 'connection-1', state: 'connected', chromium_major: 144 };
const realFetch = globalThis.fetch;
afterEach(() => { cleanup(); globalThis.fetch = realFetch; });

function setup(client: AttachedChromeClient, initial?: string) {
  const states: boolean[] = [];
  function Harness() {
    const [value, setValue] = useState(initial);
    return <><BrowserResourceField value={value} onChange={setValue} client={client} onReadinessChange={(ready) => states.push(ready)} />
      <output data-testid='selection'>{value ?? 'managed-browser'}</output></>;
  }
  return { ...render(<I18nextProvider i18n={i18n}><Harness /></I18nextProvider>), states };
}

test('reads connection state without connecting or blocking the default managed browser', async () => {
  let resolve!: (snapshot: AttachedChromeSnapshot | null) => void;
  let connects = 0;
  const screen = setup({ snapshot: () => new Promise((done) => { resolve = done; }), connect: async () => { connects += 1; return connected; }, disconnect: async () => {} });
  expect(screen.states.at(-1)).toBe(true);
  await act(async () => resolve(null));
  expect(connects).toBe(0);
  expect(screen.getByTestId('selection').textContent).toBe('managed-browser');
  expect(screen.getByText(en.resources.browser.setup)).toBeTruthy();
});

test('connects explicitly, requires an explicit provider choice and disconnects the exact incarnation', async () => {
  const disconnects: string[] = [];
  let connects = 0;
  const screen = setup({ snapshot: async () => null, connect: async () => { connects += 1; return connected; }, disconnect: async (incarnation) => { disconnects.push(incarnation); } });
  await waitFor(() => expect(screen.getByText(en.resources.browser.connect).closest('button')?.disabled).toBe(false));
  fireEvent.click(screen.getByText(en.resources.browser.connect));
  await screen.findByText(en.resources.browser.states.connected);
  expect(connects).toBe(1);
  expect(screen.getByTestId('selection').textContent).toBe('managed-browser');
  fireEvent.click(screen.getByRole('combobox'));
  fireEvent.click(await screen.findByText(en.resources.browser.attached));
  await waitFor(() => expect(screen.getByTestId('selection').textContent).toBe('attached-chrome'));
  fireEvent.click(screen.getByText(en.resources.browser.disconnect));
  await screen.findByText(en.resources.browser.selectedUnavailable);
  expect(disconnects).toEqual(['connection-1']);
  expect(screen.getByTestId('selection').textContent).toBe('attached-chrome');
  expect(screen.states.at(-1)).toBe(false);
});

test('failed disconnect never reports success, changes the selected binding or automatically retries', async () => {
  let disconnects = 0;
  const screen = setup({ snapshot: async () => connected, connect: async () => connected, disconnect: async () => { disconnects += 1; throw new Error('busy'); } }, 'attached-chrome');
  await screen.findByText(en.resources.browser.states.connected);
  fireEvent.click(screen.getByText(en.resources.browser.disconnect));
  await screen.findByText(en.resources.browser.failed);
  expect(disconnects).toBe(1);
  expect(screen.states.at(-1)).toBe(false);
  expect(screen.getByTestId('selection').textContent).toBe('attached-chrome');
  expect(screen.queryByText(en.resources.browser.states.disconnected)).toBeNull();
  fireEvent.click(screen.getByText(en.resources.browser.refresh));
  await screen.findByText(en.resources.browser.states.connected);
  expect(screen.states.at(-1)).toBe(true);
  expect(disconnects).toBe(1);
});

test('connection loss preserves the explicit choice and lets the user choose the managed browser', async () => {
  const screen = setup({ snapshot: async () => ({ ...connected, state: 'connection_lost' }), connect: async () => connected, disconnect: async () => {} }, 'attached-chrome');
  await screen.findByText(en.resources.browser.selectedUnavailable);
  expect(screen.states.at(-1)).toBe(false);
  fireEvent.click(screen.getByRole('combobox'));
  const options = await screen.findAllByText(en.resources.browser.managed);
  fireEvent.click(options.at(-1)!);
  await waitFor(() => expect(screen.getByTestId('selection').textContent).toBe('managed-browser'));
  expect(screen.states.at(-1)).toBe(true);
});

test('the provider client uses authenticated kind-specific endpoints and only sends the exact disconnect incarnation', async () => {
  const requests: Array<{ path: string; method?: string; body?: unknown }> = [];
  globalThis.fetch = (async (input, init) => {
    requests.push({ path: new URL(String(input), 'http://localhost').pathname, method: init?.method, body: init?.body ? JSON.parse(String(init.body)) : undefined });
    return new Response(JSON.stringify({ success: true, data: init?.method === 'DELETE' ? null : connected }), { headers: { 'Content-Type': 'application/json' } });
  }) as typeof fetch;
  await attachedChromeClient.snapshot();
  await attachedChromeClient.connect();
  await attachedChromeClient.disconnect('connection-1');
  expect(requests).toEqual([
    { path: '/api/browser-providers/attached-chrome', method: 'GET', body: undefined },
    { path: '/api/browser-providers/attached-chrome', method: 'POST', body: {} },
    { path: '/api/browser-providers/attached-chrome', method: 'DELETE', body: { incarnation: 'connection-1' } },
  ]);
});
