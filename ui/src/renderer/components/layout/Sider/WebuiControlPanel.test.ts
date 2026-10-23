/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { createElement, useState } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import * as webuiContext from '@/renderer/hooks/context/WebuiServerContext';
import * as themeContext from '@/renderer/hooks/context/ThemeContext';
import * as clipboard from '@/renderer/utils/ui/clipboard';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import WebuiControlPanel from './WebuiControlPanel';
import WebuiQrLoginPanel from './WebuiQrLoginPanel';
import { buildWebuiQrLoginUrl } from './webuiQrLinks';

const readSource = (url: URL) => readFileSync(url, 'utf8');
const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: { settings, common } } } });
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach((dispose) => dispose()); });

const userResponse = (status: number, body: unknown) => new Response(JSON.stringify(body), {
  status, headers: { 'Content-Type': 'application/json' },
});

function accountFixture(native = false, mode: 'page' | 'popover' = 'page', initialPassword: string | null = null) {
  const theme = spyOn(themeContext, 'useThemeContext').mockReturnValue({
    theme: 'light', setTheme: async () => {}, colorScheme: 'default', setColorScheme: async () => {}, fontScale: 1, setFontScale: async () => {},
  });
  const context = spyOn(webuiContext, 'useWebuiServer').mockReturnValue({
    // A browser must ignore even a default status value that says "admin".
    status: { running: false, port: 25808, allowRemote: false, localUrl: 'http://127.0.0.1:25808', adminUsername: native ? 'desktop-owner' : 'admin' },
    running: false, starting: false, lifecycleSupported: native, initialPassword,
    accessUrls: [], start: async () => {}, stop: async () => {}, refresh: async () => {}, clearInitialPassword: () => {},
  });
  let finish!: (response: Response) => void;
  const pending = new Promise<Response>((resolve) => { finish = resolve; });
  const fetching = spyOn(globalThis, 'fetch').mockReturnValue(pending);
  restore.push(() => theme.mockRestore(), () => context.mockRestore(), () => fetching.mockRestore());
  const view = render(createElement(I18nextProvider, { i18n }, createElement(WebuiControlPanel, { mode })));
  const row = within(view.getByText('Username:').parentElement!);
  return { ...view, row, fetching, finish: async (status: number, body: unknown) => {
    await act(async () => { finish(userResponse(status, body)); });
  } };
}

describe('WebuiControlPanel authenticated browser account', () => {
  test('reads the authenticated username and uses it to initialize editing instead of the desktop admin default', async () => {
    const desktopInitialPassword = 'desktop-only-initial-fixture';
    const view = accountFixture(false, 'page', desktopInitialPassword);
    expect(Boolean(view.queryByText(settings.webui, { exact: true }))).toBe(false);
    expect(Boolean(view.queryByText(settings['webui.featureRemoteDesc']))).toBe(false);
    expect(Boolean(view.queryByText(window.location.origin, { exact: true }))).toBe(false);
    const passwordRow = within(view.getByText(`${settings['webui.password']}:`).parentElement!.parentElement!);
    expect(passwordRow.getByText(settings['webui.passwordConfigured'])).toBeTruthy();
    expect(view.getByText(settings['webui.passwordUnavailableHint'])).toBeTruthy();
    expect(Boolean(view.queryByText(settings['webui.passwordHidden']))).toBe(false);
    expect(Boolean(view.queryByText(desktopInitialPassword))).toBe(false);
    expect(Boolean(passwordRow.queryByRole('button', { name: settings['webui.showPassword'] }))).toBe(false);
    expect(Boolean(passwordRow.queryByRole('button', { name: settings['webui.hidePassword'] }))).toBe(false);
    expect(Boolean(passwordRow.queryByRole('button', { name: settings['webui.copyPasswordTooltip'] }))).toBe(false);
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
    const view = accountFixture(true, 'popover');
    expect(view.getByText(settings.webui, { exact: true })).toBeTruthy();
    expect(view.getByText(settings['webui.featureRemoteDesc'])).toBeTruthy();
    expect(view.row.getByText('desktop-owner')).toBeTruthy();
    const passwordRow = within(view.getByText(`${settings['webui.password']}:`).parentElement!.parentElement!);
    expect(passwordRow.getByText(settings['webui.passwordConfigured'])).toBeTruthy();
    expect(Boolean(view.queryByText(settings['webui.passwordHidden']))).toBe(false);
    expect(Boolean(passwordRow.queryByRole('button', { name: settings['webui.showPassword'] }))).toBe(false);
    expect(Boolean(passwordRow.queryByRole('button', { name: settings['webui.copyPasswordTooltip'] }))).toBe(false);
    expect(view.fetching).not.toHaveBeenCalled();
    expect((view.row.getByRole('button', { name: settings['webui.editUsernameTooltip'] }) as HTMLButtonElement).disabled).toBe(false);
  });
});

describe('WebuiControlPanel QR login URL selection', () => {
  test('builds QR URLs from all access URLs instead of only status.networkUrl', () => {
    const source = readSource(new URL('./WebuiControlPanel.tsx', import.meta.url));

    expect(source.includes('getWebuiQrBaseUrls(status, accessUrls, port)')).toBe(true);
    expect(source.includes('buildWebuiQrLoginUrl(selectedQrBaseUrl, qrToken)')).toBe(true);
    expect(source.includes('webui.generateQRToken.invoke()')).toBe(true);
    expect(source.includes('4 * 60 * 1000')).toBe(true);
    expect(source.includes('status.allowRemote && status.networkUrl')).toBe(false);
  });

  test('copies the selected login address and disables copy and refresh while loading', async () => {
    const baseUrls = ['http://192.168.1.50:25808', 'http://10.0.0.50:25808'];
    const copy = spyOn(clipboard, 'copyText').mockResolvedValue(undefined);
    const success = spyOn(Message, 'success').mockReturnValue(() => {});
    restore.push(() => copy.mockRestore(), () => success.mockRestore());
    let refreshes = 0;
    function ControlledPanel({ loading }: { loading: boolean }) {
      const [selectedBaseUrl, setSelectedBaseUrl] = useState(baseUrls[0]);
      return createElement(WebuiQrLoginPanel, {
        baseUrls,
        selectedBaseUrl,
        onBaseUrlChange: setSelectedBaseUrl,
        qrUrl: buildWebuiQrLoginUrl(selectedBaseUrl, 'synthetic-token'),
        expiresAt: Date.now() + 60_000,
        loading,
        onRefresh: () => { refreshes += 1; },
      });
    }
    const panel = (loading: boolean) => createElement(I18nextProvider, { i18n }, createElement(ControlledPanel, { loading }));
    const view = render(panel(false));
    const select = view.getByRole('combobox', { name: settings['webui.loginAddress'] });
    fireEvent.click(select);
    fireEvent.click(view.getByRole('option', { name: baseUrls[1] }));
    await waitFor(() => expect(select.textContent).toContain(baseUrls[1]));
    const copyButton = view.getByRole('button', { name: settings['webui.copyQrLink'] }) as HTMLButtonElement;
    const refreshButton = view.getByRole('button', { name: settings['webui.refreshQr'] }) as HTMLButtonElement;
    expect(copyButton.disabled).toBe(false);
    expect(refreshButton.disabled).toBe(false);
    fireEvent.click(copyButton);
    await waitFor(() => expect(copy).toHaveBeenCalledWith('http://10.0.0.50:25808/qr-login?token=synthetic-token'));
    fireEvent.click(refreshButton);
    expect(refreshes).toBe(1);
    view.rerender(panel(true));
    const disabledCopy = view.getByRole('button', { name: settings['webui.copyQrLink'] }) as HTMLButtonElement;
    const disabledRefresh = view.getByRole('button', { name: settings['webui.refreshQr'] }) as HTMLButtonElement;
    expect(disabledCopy.disabled).toBe(true);
    expect(disabledRefresh.disabled).toBe(true);
    fireEvent.click(disabledCopy);
    fireEvent.click(disabledRefresh);
    expect(copy).toHaveBeenCalledTimes(1);
    expect(refreshes).toBe(1);
  });
});

describe('WebuiControlPanel credential controls', () => {
  test('the native initial password stays masked until revealed and can be copied without a duplicate plaintext block', async () => {
    const initialPassword = 'generated-initial-password-fixture';
    const copy = spyOn(clipboard, 'copyText').mockResolvedValue(undefined);
    const success = spyOn(Message, 'success').mockReturnValue(() => {});
    restore.push(() => copy.mockRestore(), () => success.mockRestore());
    const view = accountFixture(true, 'page', initialPassword);
    const passwordRow = within(view.getByText(`${settings['webui.password']}:`).parentElement!.parentElement!);
    expect(Boolean(view.queryByText(initialPassword))).toBe(false);
    expect(passwordRow.getByText(settings['webui.passwordHidden'])).toBeTruthy();
    expect(view.getByText(settings['webui.initialPasswordHint'])).toBeTruthy();
    fireEvent.click(passwordRow.getByRole('button', { name: settings['webui.showPassword'] }));
    expect(view.getAllByText(initialPassword)).toHaveLength(1);
    expect(Boolean(passwordRow.queryByRole('button', { name: settings['webui.showPassword'] }))).toBe(false);
    fireEvent.click(passwordRow.getByRole('button', { name: settings['webui.hidePassword'] }));
    expect(Boolean(view.queryByText(initialPassword))).toBe(false);
    fireEvent.click(passwordRow.getByRole('button', { name: settings['webui.copyPasswordTooltip'] }));
    await waitFor(() => expect(copy).toHaveBeenCalledWith(initialPassword));
    expect(Boolean(view.queryByText(initialPassword))).toBe(false);
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
    expect(webuiTabSource.includes('<WebuiAccessAddresses')).toBe(true);
    expect(webuiTabSource.indexOf('<WebuiAccessAddresses')).toBeGreaterThan(webuiTabSource.indexOf('<InfoPanel'));
    expect(webuiTabSource.includes('<EndpointBlock')).toBe(false);
    expect(webuiTabSource.includes('currentAccessBase')).toBe(false);
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
    fireEvent.click(view.getByRole('button', { name: settings['webui.changePassword'] }));
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
