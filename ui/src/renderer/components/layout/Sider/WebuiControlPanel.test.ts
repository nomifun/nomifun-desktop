/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { createElement } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import * as webuiContext from '@/renderer/hooks/context/WebuiServerContext';
import * as themeContext from '@/renderer/hooks/context/ThemeContext';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import WebuiControlPanel from './WebuiControlPanel';

const readSource = (url: URL) => readFileSync(url, 'utf8');
const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: { settings, common } } } });
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach((dispose) => dispose()); });

const userResponse = (status: number, body: unknown) => new Response(JSON.stringify(body), {
  status, headers: { 'Content-Type': 'application/json' },
});

function accountFixture(native = false) {
  const theme = spyOn(themeContext, 'useThemeContext').mockReturnValue({
    theme: 'light', setTheme: async () => {}, colorScheme: 'default', setColorScheme: async () => {}, fontScale: 1, setFontScale: async () => {},
  });
  const context = spyOn(webuiContext, 'useWebuiServer').mockReturnValue({
    // A browser must ignore even a default status value that says "admin".
    status: { running: false, port: 25808, allowRemote: false, localUrl: 'http://127.0.0.1:25808', adminUsername: native ? 'desktop-owner' : 'admin' },
    running: false, starting: false, lifecycleSupported: native, initialPassword: null,
    accessUrls: [], start: async () => {}, stop: async () => {}, refresh: async () => {}, clearInitialPassword: () => {},
  });
  let finish!: (response: Response) => void;
  const pending = new Promise<Response>((resolve) => { finish = resolve; });
  const fetching = spyOn(globalThis, 'fetch').mockReturnValue(pending);
  restore.push(() => theme.mockRestore(), () => context.mockRestore(), () => fetching.mockRestore());
  const view = render(createElement(I18nextProvider, { i18n }, createElement(WebuiControlPanel, { mode: 'page' })));
  const row = within(view.getByText('Username:').parentElement!);
  return { ...view, row, fetching, finish: async (status: number, body: unknown) => {
    await act(async () => { finish(userResponse(status, body)); });
  } };
}

describe('WebuiControlPanel authenticated browser account', () => {
  test('reads the authenticated username and uses it to initialize editing instead of the desktop admin default', async () => {
    const view = accountFixture();
    expect(view.row.getByText(common.loading)).toBeTruthy();
    expect(view.queryByText('admin')).toBeNull();
    expect((view.row.getByRole('button', { name: common.copy }) as HTMLButtonElement).disabled).toBe(true);
    expect((view.row.getByRole('button', { name: settings['webui.editUsernameTooltip'] }) as HTMLButtonElement).disabled).toBe(true);
    await view.finish(200, { success: true, user: { user_id: '0190f5fe-7c00-7a00-8000-000000000064', username: 'renamed-owner' } });
    await waitFor(() => expect(view.row.getByText('renamed-owner')).toBeTruthy());
    expect(view.fetching).toHaveBeenCalledWith('/api/auth/user', expect.objectContaining({ method: 'GET', credentials: 'include', cache: 'no-store' }));
    expect(view.queryByText('admin')).toBeNull();
    expect((view.row.getByRole('button', { name: common.copy }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(view.row.getByRole('button', { name: settings['webui.editUsernameTooltip'] }));
    expect((view.getByPlaceholderText(settings['webui.newUsernamePlaceholder']) as HTMLInputElement).value).toBe('renamed-owner');
  });

  test.each([
    [503, { success: false }],
    [200, { success: true, user: { username: 'missing-user-id' } }],
  ])('an unavailable or malformed account response (%s) cannot display or edit a fabricated username', async (status, body) => {
    const warning = spyOn(console, 'warn').mockImplementation(() => {});
    restore.push(() => warning.mockRestore());
    const view = accountFixture();
    await view.finish(status, body);
    await waitFor(() => expect(view.row.getByText(common.unknownUser)).toBeTruthy());
    expect(view.queryByText('admin')).toBeNull();
    expect(view.queryByText('missing-user-id')).toBeNull();
    expect((view.row.getByRole('button', { name: common.copy }) as HTMLButtonElement).disabled).toBe(true);
    const editing = view.row.getByRole('button', { name: settings['webui.editUsernameTooltip'] });
    expect((editing as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(editing);
    expect(view.queryByPlaceholderText(settings['webui.newUsernamePlaceholder'])).toBeNull();
  });

  test('native status keeps the desktop account without probing browser authentication', () => {
    const view = accountFixture(true);
    expect(view.row.getByText('desktop-owner')).toBeTruthy();
    expect(view.fetching).not.toHaveBeenCalled();
    expect((view.row.getByRole('button', { name: settings['webui.editUsernameTooltip'] }) as HTMLButtonElement).disabled).toBe(false);
  });
});

describe('WebuiControlPanel QR login URL selection', () => {
  test('builds QR URLs from all access URLs instead of only status.networkUrl', () => {
    const source = readSource(new URL('./WebuiControlPanel.tsx', import.meta.url));

    expect(source.includes('getWebuiQrBaseUrls(status, accessUrls, port)')).toBe(true);
    expect(source.includes('status.allowRemote && status.networkUrl')).toBe(false);
  });

  test('makes the QR base URL selector a visible address decision', () => {
    const source = readSource(new URL('./WebuiControlPanel.tsx', import.meta.url));

    expect(source.includes('settings.webui.qrAddressPickerTitle')).toBe(true);
    expect(source.includes('settings.webui.qrAddressPickerDesc')).toBe(true);
    expect(source.includes('qr-address-picker')).toBe(true);
    expect(source.includes('border-[rgba(var(--primary-6),0.30)]')).toBe(true);
  });
});

describe('WebuiControlPanel credential controls', () => {
  test('keeps credential pills compact while separating text from grouped icons', () => {
    const source = readSource(new URL('./WebuiControlPanel.tsx', import.meta.url));

    expect(source.includes("px-8px py-2px'")).toBe(true);
    expect(source.includes("'ml-6px inline-flex shrink-0 items-center gap-0'")).toBe(true);
    expect(source.includes('!h-20px !w-20px !min-w-20px !p-0')).toBe(true);
    expect(source.includes('<Copy size={13} />')).toBe(true);
    expect(source.includes('<EditTwo size={13} />')).toBe(true);
  });

  test('uses content-height small modals with the current shared header contract', () => {
    const source = readSource(new URL('./WebuiControlPanel.tsx', import.meta.url));
    const modalSource = readSource(new URL('../../base/NomiModal.tsx', import.meta.url));

    expect(modalSource.includes("small: { width: '400px' }")).toBe(true);
    expect(modalSource.includes("small: { width: '400px', height: '300px' }")).toBe(false);
    expect(source.match(/header=\{\{ title: t\('settings\.webui\.setNew/g)?.length).toBe(2);
    expect(source.match(/layout='vertical' className='pt-12px'/g)?.length).toBe(2);
  });
});

describe('Open Capabilities WebUI entry', () => {
  test('moves the full WebUI control out of the crowded footer into the Open Capabilities page', () => {
    const footerSource = readSource(new URL('./SiderFooter.tsx', import.meta.url));
    const pageSource = readSource(new URL('../../../pages/openCapabilities/index.tsx', import.meta.url));
    const webuiTabStart = pageSource.indexOf("<Tabs.TabPane key='browser'");
    const mcpTabStart = pageSource.indexOf("<Tabs.TabPane key='tools'");
    const webuiTabSource = pageSource.slice(webuiTabStart, mcpTabStart);

    expect(footerSource.includes('SiderWebuiControl')).toBe(false);
    expect(webuiTabStart).toBeGreaterThan(-1);
    expect(mcpTabStart).toBeGreaterThan(webuiTabStart);
    expect(webuiTabSource.includes("<WebuiControlPanel mode='page' />")).toBe(true);
    expect(webuiTabSource.includes('RegisterKnowledgeButton')).toBe(false);
    expect(webuiTabSource.includes('projectRegisterTitle')).toBe(false);
  });

  test('splits WebUI and MCP into subtabs instead of coupling both surfaces on one page', () => {
    const pageSource = readSource(new URL('../../../pages/openCapabilities/index.tsx', import.meta.url));

    expect(pageSource.includes("Tabs.TabPane key='phone'")).toBe(true);
    expect(pageSource.includes("Tabs.TabPane key='browser'")).toBe(true);
    expect(pageSource.includes("Tabs.TabPane key='tools'")).toBe(true);
    expect(pageSource.includes('activeOpenCapabilityTab')).toBe(true);
  });

  test('lets users choose NomiFun Remote MCP capability domains on this page', () => {
    const pageSource = readSource(new URL('../../../pages/openCapabilities/index.tsx', import.meta.url));

    expect(pageSource.includes('MCP_DOMAIN_OPTIONS')).toBe(true);
    expect(pageSource.includes('selectedMcpDomains')).toBe(true);
    expect(pageSource.includes('domainsQuery')).toBe(true);
    expect(pageSource.includes('<Checkbox')).toBe(true);
    expect(pageSource.includes("id: 'system'")).toBe(true);
    expect(pageSource.includes("id: 'mcp'")).toBe(true);
    expect(pageSource.includes("id: 'channel'")).toBe(true);
  });

  test('does not send users to the unrelated MCP server management page', () => {
    const pageSource = readSource(new URL('../../../pages/openCapabilities/index.tsx', import.meta.url));

    expect(pageSource.includes("navigate('/mcp')")).toBe(false);
    expect(pageSource.includes('openMcpManager')).toBe(false);
    expect(pageSource.includes('LinkCloud')).toBe(false);
  });

  test('keeps the installation access token in the MCP capability tab instead of the WebUI panel', () => {
    const webuiPanelSource = readSource(new URL('./WebuiControlPanel.tsx', import.meta.url));
    const pageSource = readSource(new URL('../../../pages/openCapabilities/index.tsx', import.meta.url));

    expect(webuiPanelSource.includes('InstanceAccessTokenPanel')).toBe(false);
    expect(webuiPanelSource.includes('RelayPairingPanel')).toBe(false);
    expect(pageSource.includes('InstanceAccessTokenPanel')).toBe(true);
  });
});

describe('WebuiControlPanel password change scope warning', () => {
  const openSetPasswordModal = (view: ReturnType<typeof accountFixture>) => {
    const row = within(view.getByText(`${settings['webui.initialPassword']}:`).parentElement!);
    fireEvent.click(row.getByRole('button'));
  };

  test('a browser session warns that the password change signs out every paired phone', async () => {
    const view = accountFixture();
    openSetPasswordModal(view);
    await waitFor(() => expect(view.getByText(settings.deviceConnections.passwordChangeSignsOutDevices)).toBeTruthy());
    expect(view.getByPlaceholderText(settings['webui.currentPasswordPlaceholder'])).toBeTruthy();
  });

  test('the desktop shell opens the same modal without the phone sign-out warning', () => {
    const view = accountFixture(true);
    openSetPasswordModal(view);
    expect(view.getByPlaceholderText(settings['webui.newPasswordPlaceholder'])).toBeTruthy();
    expect(view.queryByPlaceholderText(settings['webui.currentPasswordPlaceholder'])).toBeNull();
    expect(view.queryByText(settings.deviceConnections.passwordChangeSignsOutDevices)).toBeNull();
  });
});
