import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'bun:test';
import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { HostedApp } from '@/common/types/appHosting';
import locale from '../../services/i18n/locales/en-US/appHosting.json';
import AppPreview from './AppPreview';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { appHosting: locale } } }, interpolation: { escapeValue: false } });
const app: HostedApp = { id: 'app', name: 'Real app', description: '', tags: [], development_session_id: null,
  directory: 'C:/apps/app', import_kind: 'source', settings: { project_directory: '.', environment: {}, web_url: 'https://app.example.com' },
  state: 'running', operation: null, last_error: null, output: '', pid: 44, preview: true, prepared: true };
const wrapper = ({ children }: { children: React.ReactNode }) => <I18nextProvider i18n={i18n}>{children}</I18nextProvider>;
let server: Server;
let webpageRequests = 0;
beforeEach(async () => {
  webpageRequests = 0;
  server = createServer((_request, response) => { webpageRequests++; response.writeHead(200, { 'Content-Type': 'text/html' }); response.end('<!doctype html><title>Fixture webpage</title>'); });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  app.settings.web_url = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});
afterEach(async () => {
  cleanup(); server.closeAllConnections();
  await new Promise<void>((resolve) => server.close(() => resolve()));
});

test('an earlier project Session file observation reloads the same app process while the navigation reference points elsewhere', async () => {
  const current = { ...app, development_session_id: '0190f5fe-7c00-7a00-8000-000000000701' };
  const view = render(<AppPreview app={current} />, { wrapper });
  const original = view.container.querySelector('iframe');
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  const change = { appId: app.id, sessionId: '0190f5fe-7c00-7a00-8000-000000000702', turnId: '0190f5fe-7c00-7a00-8000-000000000703', pid: app.pid, url: app.settings.web_url };
  view.rerender(<AppPreview app={current} previewFileChange={change} />);
  const updated = view.container.querySelector('iframe');
  expect(updated).not.toBe(original);
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(2);
  view.rerender(<AppPreview app={{ ...current }} previewFileChange={{ ...change }} />);
  expect(view.container.querySelector('iframe')).toBe(updated);
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(2);
});

test('opening preview starts in a wide embedded view and resizing preserves the actual webpage', async () => {
  const view = render(<AppPreview app={{ ...app, preview: false }} />, { wrapper });
  expect(view.container.querySelector('iframe')).toBeNull();
  fireEvent.click(view.getByRole('button', { name: locale.workspace.loadPreview }));
  const frame = view.container.querySelector('iframe')!;
  expect(frame.getAttribute('src')).toBe(app.settings.web_url!);
  expect(frame.getAttribute('referrerpolicy')).toBe('no-referrer');
  expect(frame.getAttribute('sandbox')).toBe('allow-scripts allow-forms allow-same-origin');
  expect(frame.getAttribute('srcdoc')).toBeNull();
  let dialog = view.getByRole('dialog', { name: locale.workspace.previewTitle.replace('{{name}}', app.name) });
  expect(dialog.tagName).toBe('DIALOG');
  expect((dialog as HTMLDialogElement).open).toBe(true);
  expect(dialog.contains(frame)).toBe(true);
  expect(within(dialog).getByRole('button', { name: locale.workspace.openWindow })).toBeTruthy();
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(1);
  fireEvent.click(within(dialog).getByRole('button', { name: locale.workspace.collapse }));
  expect(view.queryByRole('dialog')).toBeNull();
  expect(view.container.querySelectorAll('iframe')).toHaveLength(1);
  expect(view.container.querySelector('iframe')).toBe(frame);
  fireEvent.click(view.getByRole('button', { name: locale.workspace.expand }));
  dialog = view.getByRole('dialog');
  const cancel = new Event('cancel', { cancelable: true });
  fireEvent(dialog, cancel);
  expect(cancel.defaultPrevented).toBe(true);
  expect(view.queryByRole('dialog')).toBeNull();
  expect(view.getByRole('region', { name: locale.workspace.previewTitle.replace('{{name}}', app.name) })).toBe(dialog);
  expect((dialog as HTMLDialogElement).open).toBe(true);
  expect(view.container.querySelectorAll('iframe')).toHaveLength(1);
  expect(view.container.querySelector('iframe')).toBe(frame);
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(1);
  view.rerender(<AppPreview app={{ ...app, state: 'stopped', pid: null }} />);
  expect(view.container.querySelector('iframe')).toBeNull();
  expect(view.getByText(locale.workspace.previewStopped)).toBeTruthy();
});

test('closing a restored preview removes its webpage and reopening returns to the wide view', async () => {
  const view = render(<AppPreview app={{ ...app, preview: false }} />, { wrapper });
  fireEvent.click(view.getByRole('button', { name: locale.workspace.loadPreview }));
  expect(view.getByRole('dialog')).toBeTruthy();
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  fireEvent.click(view.getByRole('button', { name: locale.workspace.collapse }));
  fireEvent.click(view.getByRole('button', { name: locale.workspace.closePreview }));
  expect(view.queryByRole('dialog')).toBeNull();
  expect(view.container.querySelector('iframe')).toBeNull();
  fireEvent.click(view.getByRole('button', { name: locale.workspace.loadPreview }));
  expect(view.getByRole('dialog')).toBeTruthy();
  expect(view.container.querySelectorAll('iframe')).toHaveLength(1);
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(2);
});

test.each([false, true])('stop and production rebuild preserve preview intent without overriding dismissal (dismissed=%s)', async dismissed => {
  const view = render(<AppPreview app={app} />, { wrapper });
  const initialFrame = view.container.querySelector('iframe')!;
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(1);
  if (dismissed) fireEvent.click(view.getByRole('button', { name: locale.workspace.closePreview }));
  else {
    fireEvent.click(view.getByRole('button', { name: locale.workspace.expand }));
    expect(view.getByRole('dialog').contains(initialFrame)).toBe(true);
  }
  view.rerender(<AppPreview app={{ ...app, state: 'stopped', pid: null, preview: false }} />);
  expect(view.container.querySelector('iframe')).toBeNull();
  expect(view.queryByRole('dialog')).toBeNull();
  view.rerender(<AppPreview app={{ ...app, pid: app.pid! + 1, preview: false }} />);
  expect(view.queryByRole('dialog')).toBeNull();
  if (dismissed) {
    expect(view.container.querySelector('iframe')).toBeNull();
    expect(view.getByRole('button', { name: locale.workspace.loadPreview })).toBeTruthy();
  } else {
    const rebuiltFrame = view.container.querySelector('iframe')!;
    expect(rebuiltFrame).not.toBe(initialFrame);
    expect(rebuiltFrame.getAttribute('src')).toBe(app.settings.web_url!);
    expect(view.getByRole('region', { name: locale.workspace.previewTitle.replace('{{name}}', app.name) }).contains(rebuiltFrame)).toBe(true);
  }
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(dismissed ? 1 : 2);
});

test('the real Agent preview owner loads inline automatically and polling preserves iframe state and dismissal', async () => {
  const view = render(<AppPreview app={{ ...app, state: 'stopped', preview: false, pid: null }} />, { wrapper });
  expect(view.container.querySelector('iframe')).toBeNull();
  view.rerender(<AppPreview app={app} />);
  const frame = view.container.querySelector('iframe')!;
  expect(frame.getAttribute('src')).toBe(app.settings.web_url!);
  expect(view.queryByRole('dialog')).toBeNull();
  expect(view.getByRole('region', { name: locale.workspace.previewTitle.replace('{{name}}', app.name) }).contains(frame)).toBe(true);
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(1);
  view.rerender(<AppPreview app={{ ...app, name: 'Updated metadata' }} />);
  expect(view.container.querySelector('iframe')).toBe(frame);
  view.rerender(<AppPreview app={{ ...app, pid: 45 }} />);
  const restartedFrame = view.container.querySelector('iframe')!;
  expect(restartedFrame).not.toBe(frame);
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(2);
  view.rerender(<AppPreview app={{ ...app, pid: 45, output: 'A polled line' }} />);
  expect(view.container.querySelector('iframe')).toBe(restartedFrame);
  fireEvent.click(view.getByRole('button', { name: locale.workspace.refreshPreview }));
  const refreshedFrame = view.container.querySelector('iframe')!;
  expect(refreshedFrame).not.toBe(restartedFrame);
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(3);
  fireEvent.click(view.getByRole('button', { name: locale.workspace.expand }));
  expect(view.getByRole('dialog').contains(refreshedFrame)).toBe(true);
  fireEvent.click(view.getByRole('button', { name: locale.workspace.collapse }));
  fireEvent.click(view.getByRole('button', { name: locale.workspace.closePreview }));
  view.rerender(<AppPreview app={{ ...app, pid: 45, output: 'A new command output line' }} />);
  expect(view.container.querySelector('iframe')).toBeNull();
  view.rerender(<AppPreview app={{ ...app, pid: 46 }} />);
  expect(view.container.querySelector('iframe')).not.toBeNull();
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(4);
});

test('renderer-origin addresses cannot be embedded even if they are valid HTTP', () => {
  const view = render(<AppPreview app={{ ...app, settings: { ...app.settings, web_url: `${window.location.origin}/apps` } }} />, { wrapper });
  expect(view.container.querySelector('iframe')).toBeNull();
  expect(view.queryByRole('button', { name: locale.workspace.loadPreview })).toBeNull();
});

test('an exact completed file change reloads the already loaded same process once and never reopens a dismissed preview', async () => {
  const owner = { ...app, development_session_id: 'application-session' };
  const notice = { appId: owner.id, sessionId: 'application-session', turnId: 'changed-turn', pid: owner.pid, url: owner.settings.web_url };
  const view = render(<AppPreview app={owner} />, { wrapper });
  const initialFrame = view.container.querySelector('iframe')!;
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(1);
  view.rerender(<AppPreview app={owner} previewFileChange={notice} />);
  const changedFrame = view.container.querySelector('iframe')!;
  expect(changedFrame).not.toBe(initialFrame);
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(2);
  view.rerender(<AppPreview app={{ ...owner, name: 'Polled metadata', output: 'Polled output' }} previewFileChange={{ ...notice }} />);
  expect(view.container.querySelector('iframe')).toBe(changedFrame);
  fireEvent.click(view.getByRole('button', { name: locale.workspace.closePreview }));
  view.rerender(<AppPreview app={owner} previewFileChange={{ ...notice, turnId: 'another-changed-turn' }} />);
  expect(view.container.querySelector('iframe')).toBeNull();
  expect(view.queryByRole('dialog')).toBeNull();
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(2);
});

test('a file observation arriving after a new PID preview does not reload that new webpage again', async () => {
  const owner = { ...app, development_session_id: 'application-session' };
  const view = render(<AppPreview app={owner} />, { wrapper });
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  const restarted = { ...owner, pid: owner.pid! + 1 };
  view.rerender(<AppPreview app={restarted} />);
  const restartedFrame = view.container.querySelector('iframe')!;
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(2);
  view.rerender(<AppPreview app={restarted} previewFileChange={{ appId: owner.id, sessionId: 'application-session',
    turnId: 'changed-turn', pid: owner.pid, url: owner.settings.web_url }} />);
  expect(view.container.querySelector('iframe')).toBe(restartedFrame);
  await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  expect(webpageRequests).toBe(2);
});

test('remote WebUI never embeds or opens the Host loopback as the browser computer', () => {
  const browser = (window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM;
  const previous = window.location.href;
  browser.setURL('https://remote.example.com/apps');
  try {
    const view = render(<AppPreview app={app} />, { wrapper });
    expect(view.container.querySelector('iframe')).toBeNull();
    expect(view.queryByRole('button', { name: locale.workspace.loadPreview })).toBeNull();
    expect(view.queryByRole('button', { name: locale.workspace.openWindow })).toBeNull();
    expect(view.getByText(locale.remoteAddress)).toBeTruthy();
  } finally { browser.setURL(previous); }
});
