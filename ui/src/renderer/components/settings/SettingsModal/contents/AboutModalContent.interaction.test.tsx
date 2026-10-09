import '../../../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import AboutModalContent from './AboutModalContent';
import { NOMIFUN_PUBLIC_LINKS } from './FeedbackReportModal';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings } } } });
const restores: Array<() => void> = [];
beforeEach(() => {
  window.__backendPort = 13400;
  const fetch = spyOn(globalThis, 'fetch').mockResolvedValue(new Response(JSON.stringify({ success: true, data: { version: '0.8.2-test' } }), {
    headers: { 'Content-Type': 'application/json' },
  }));
  const open = spyOn(ipcBridge.shell.openExternal, 'invoke').mockResolvedValue(undefined);
  restores.push(() => fetch.mockRestore(), () => open.mockRestore());
});
afterEach(() => { cleanup(); restores.splice(0).forEach((restore) => restore()); delete window.__backendPort; });
const renderPage = () => render(<I18nextProvider i18n={i18n}><AboutModalContent /></I18nextProvider>);

test('desktop About shows the backend version and opens the existing update flow and public contact link', async () => {
  const update = spyOn(window, 'dispatchEvent');
  restores.push(() => update.mockRestore());
  const page = renderPage();
  expect(await page.findByText('v0.8.2-test')).toBeTruthy();
  fireEvent.click(page.getByRole('button', { name: settings.checkForUpdates }));
  const event = update.mock.calls.map(([event]) => event).find((event) => event.type === 'nomifun-open-update-modal') as CustomEvent;
  expect(event?.detail).toEqual({ source: 'about' });
  fireEvent.click(page.getByRole('button', { name: new RegExp(settings.contactMe) }));
  await waitFor(() => expect(ipcBridge.shell.openExternal.invoke).toHaveBeenCalledWith(NOMIFUN_PUBLIC_LINKS.contact));
});

test('WebUI retains public resources while hiding native update and manual download controls', async () => {
  delete window.__backendPort;
  const page = renderPage();
  expect(await page.findByText('v0.8.2-test')).toBeTruthy();
  expect(page.queryByRole('button', { name: settings.checkForUpdates })).toBeNull();
  expect(page.queryByRole('button', { name: settings.baiduManualDownload })).toBeNull();
  expect(page.getByRole('button', { name: /^GitHub$/ })).toBeTruthy();
  expect(page.getByRole('button', { name: new RegExp(settings.helpDocumentation) })).toBeTruthy();
});
