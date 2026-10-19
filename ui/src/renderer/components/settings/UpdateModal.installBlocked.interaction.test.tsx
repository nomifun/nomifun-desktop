/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import {
  AutoInstallUnsupportedError,
  type UpdaterInstallContext,
} from '@/common/adapter/tauriUpdateInstall';
import update from '@/renderer/services/i18n/locales/en-US/update.json';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import {
  getUpdateAvailabilitySnapshot,
  reportNoUpdateAvailable,
  reportUpdateAvailable,
} from '@/renderer/hooks/system/useUpdateAvailability';
import UpdateModal from './UpdateModal';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  resources: { 'en-US': { translation: { update, settings, common } } },
  interpolation: { escapeValue: false },
});

const RELEASE_URL = 'https://github.com/nomifun/nomifun-desktop/releases/tag/v0.8.13';

const blockedContext: UpdaterInstallContext = {
  platform: 'macos',
  appBundlePath: '/Volumes/NomiFun/NomiFun.app',
  tempDir: '/var/folders/x/T/',
  appDeviceId: 9,
  tempDeviceId: 7,
  autoInstallSupported: false,
  reason: 'mounted_volume',
};

const supportedContext: UpdaterInstallContext = {
  platform: 'macos',
  appBundlePath: '/Applications/NomiFun.app',
  tempDir: '/var/folders/x/T/',
  appDeviceId: 7,
  tempDeviceId: 7,
  autoInstallSupported: true,
  reason: null,
};

type AutoCheckData = {
  updateInfo?: { version: string; releaseDate?: string; releaseNotes?: string };
  retainedVersion?: string | null;
  packageState?: import('@/common/adapter/tauriShell').TauriUpdatePackageState | null;
  packageVersion?: string | null;
  installContext?: UpdaterInstallContext | null;
};

let checkData: Partial<AutoCheckData> = {};
let updateAvailable = true;
let checkError: Error | null = null;
let installError: unknown = null;
let messageError: ReturnType<typeof spyOn>;

const restores: Array<() => void> = [];

beforeEach(() => {
  const autoCheck = spyOn(ipcBridge.autoUpdate.check, 'invoke').mockImplementation(async () => ({
    success: true,
    data: {
      updateInfo: updateAvailable ? { version: '0.8.13', releaseNotes: 'notes' } : undefined,
      retainedVersion: null,
      packageState: null,
      packageVersion: null,
      installContext: null,
      ...checkData,
    },
  }));
  const updateCheck = spyOn(ipcBridge.update.check, 'invoke').mockImplementation(async () => {
    if (checkError) throw checkError;
    return {
      success: true,
      data: {
        currentVersion: '0.8.12',
        updateAvailable,
        latest: updateAvailable ? {
          tagName: 'v0.8.13',
          version: '0.8.13',
          htmlUrl: RELEASE_URL,
          prerelease: false,
          draft: false,
        } : undefined,
      },
    };
  });
  const quit = spyOn(ipcBridge.autoUpdate.quitAndInstall, 'invoke').mockImplementation(async () => {
    if (installError) throw installError;
  });
  const open = spyOn(ipcBridge.shell.openExternal, 'invoke').mockResolvedValue(undefined);
  messageError = spyOn(Message, 'error').mockImplementation(() => () => {});
  restores.push(
    () => autoCheck.mockRestore(),
    () => updateCheck.mockRestore(),
    () => quit.mockRestore(),
    () => open.mockRestore(),
    () => messageError.mockRestore()
  );
});

afterEach(() => {
  cleanup();
  restores.splice(0).forEach((restore) => restore());
  checkData = {};
  updateAvailable = true;
  checkError = null;
  installError = null;
  reportNoUpdateAvailable();
});

const renderModal = async () => {
  const page = render(
    <I18nextProvider i18n={i18n}>
      <UpdateModal />
    </I18nextProvider>
  );
  await act(async () => {
    window.dispatchEvent(new CustomEvent('nomifun-open-update-modal'));
  });
  return page;
};

const expectNoUpdateActions = (page: ReturnType<typeof render>) => {
  expect(page.queryByText(update.errorTitle)).toBeNull();
  expect(page.queryByRole('button', { name: common.retry })).toBeNull();
  expect(page.queryByRole('button', { name: update.updateNow })).toBeNull();
  expect(page.queryByRole('button', { name: update.downloadButton })).toBeNull();
  expect(page.queryByRole('button', { name: update.installNow })).toBeNull();
};

test('a check without an update for this platform shows a neutral result and clears an earlier update badge', async () => {
  updateAvailable = false;
  reportUpdateAvailable('0.8.13');
  const page = await renderModal();

  expect(await page.findByText(update.upToDateTitle)).toBeTruthy();
  expect(page.getByText('Current version: v0.8.12')).toBeTruthy();
  expect(getUpdateAvailabilitySnapshot()).toEqual({ available: false });
  expectNoUpdateActions(page);
});

test('a not-available event replaces an earlier update offer with the neutral platform result', async () => {
  const page = await renderModal();
  expect(await page.findByRole('button', { name: update.updateNow })).toBeTruthy();
  expect(getUpdateAvailabilitySnapshot()).toEqual({ available: true, version: '0.8.13' });

  await act(async () => {
    ipcBridge.autoUpdate.status.emit({ status: 'not-available' });
  });

  expect(await page.findByText(update.upToDateTitle)).toBeTruthy();
  expect(page.getByText('Current version: v0.8.12')).toBeTruthy();
  expect(getUpdateAvailabilitySnapshot()).toEqual({ available: false });
  expectNoUpdateActions(page);
});

test('a failed check still shows the error and retry action', async () => {
  updateAvailable = false;
  checkError = new Error('Network request timed out');
  const logError = spyOn(console, 'error').mockImplementation(() => {});
  restores.push(() => logError.mockRestore());
  const page = await renderModal();

  expect(await page.findByText(update.errorTitle)).toBeTruthy();
  expect(page.getByText(checkError.message)).toBeTruthy();
  expect(page.getByRole('button', { name: common.retry })).toBeTruthy();
  expect(page.queryByText(update.upToDateTitle)).toBeNull();
});

test('a refused install context names the cause and path and offers manual downloads only', async () => {
  checkData = { installContext: blockedContext };
  const page = await renderModal();

  expect(await page.findByText(update.installBlockedTitle)).toBeTruthy();
  expect(await page.findByText(/installer disk image or an external drive/)).toBeTruthy();
  expect(await page.findByText(`Running from: ${blockedContext.appBundlePath}`)).toBeTruthy();
  expect(page.queryByRole('button', { name: update.updateNow })).toBeNull();
  expect(page.queryByRole('button', { name: update.installNow })).toBeNull();

  fireEvent.click(page.getByRole('button', { name: update.goToRelease }));
  await waitFor(() => expect(ipcBridge.shell.openExternal.invoke).toHaveBeenCalledWith(RELEASE_URL));

  fireEvent.click(page.getByRole('button', { name: settings.officialDownload }));
  await waitFor(() =>
    expect(ipcBridge.shell.openExternal.invoke).toHaveBeenCalledWith('https://www.nomifun.com/download')
  );
});

test('a supported install context keeps the ordinary update affordance', async () => {
  checkData = { installContext: supportedContext };
  const page = await renderModal();

  expect(await page.findByRole('button', { name: update.updateNow })).toBeTruthy();
  expect(page.queryByText(update.installBlockedTitle)).toBeNull();
});

test('an install-time refusal lands on the same blocked surface without a toast', async () => {
  checkData = {
    installContext: supportedContext,
    retainedVersion: '0.8.13',
    packageState: 'ready',
    packageVersion: '0.8.13',
  };
  const translocated: UpdaterInstallContext = {
    ...supportedContext,
    appBundlePath: '/private/var/folders/x/AppTranslocation/ABC/d/NomiFun.app',
    autoInstallSupported: false,
    reason: 'app_translocation',
  };
  installError = new AutoInstallUnsupportedError(translocated);

  const page = await renderModal();
  const installButton = await page.findByRole('button', { name: update.installNow });
  await act(async () => {
    fireEvent.click(installButton);
  });

  expect(await page.findByText(update.installBlockedTitle)).toBeTruthy();
  expect(await page.findByText(/App Translocation/)).toBeTruthy();
  expect(await page.findByText(`Running from: ${translocated.appBundlePath}`)).toBeTruthy();
  expect(page.queryByRole('button', { name: update.installNow })).toBeNull();
  expect(messageError).not.toHaveBeenCalled();
});
