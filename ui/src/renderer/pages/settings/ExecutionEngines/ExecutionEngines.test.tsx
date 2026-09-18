import '../../../../../test/setup-dom.ts';
import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import type { RuntimeEngineDescriptor } from '@/common/types/agentPlatform';
import en from '../../../services/i18n/locales/en-US/settings.json';
import ExecutionEngineSettings from './index';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US', resources: { 'en-US': { translation: { settings: en } } },
});
const copy = en.executionEngines;
const engine = (family_id: string, build_id = 'build-42'): RuntimeEngineDescriptor => ({
  family_id, build_id, build_digest: 'a'.repeat(64), display_name: family_id,
  host_contract_version: 1, supported_profiles: ['default', 'review'],
});
let list: ReturnType<typeof spyOn<typeof ipcBridge.agentPlatform.runtimeEngines.list, 'invoke'>>;
afterEach(() => { cleanup(); list?.mockRestore(); });

const renderPage = () => render(
  <I18nextProvider i18n={i18n}>
    <SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0, shouldRetryOnError: false, revalidateOnFocus: false }}>
      <MemoryRouter initialEntries={['/settings/execution-engines']}>
        <Routes>
          <Route path='/settings/execution-engines' element={<ExecutionEngineSettings />} />
          <Route path='/settings/javascript-runtime' element={<h1>Node.js settings</h1>} />
          <Route path='/agent' element={<h1>Agent workbench</h1>} />
        </Routes>
      </MemoryRouter>
    </SWRConfig>
  </I18nextProvider>
);

describe('Nomi Runtime diagnostics', () => {
  test('shows one official Runtime and never presents another family as a selector', async () => {
    list = spyOn(ipcBridge.agentPlatform.runtimeEngines.list, 'invoke').mockResolvedValue([
      engine('nomifun.nomi'), engine('nomifun.coding'), engine('customer.workflow'),
    ]);
    const screen = renderPage();
    const runtime = within(await screen.findByRole('region', { name: 'Nomi Runtime' }));
    expect(runtime.getByText(copy.healthy)).toBeTruthy();
    expect(runtime.getByText('build-42')).toBeTruthy();
    expect(runtime.getByText(copy.recoveryReady)).toBeTruthy();
    expect(screen.queryByText('nomifun.coding')).toBeNull();
    expect(screen.queryByRole('combobox')).toBeNull();
    fireEvent.click(screen.getByRole('link', { name: copy.configureLink }));
    expect(await screen.findByRole('heading', { name: 'Agent workbench' })).toBeTruthy();
  });

  test('shows loading, error, unavailable and recovery states truthfully', async () => {
    let reject!: (reason: Error) => void;
    list = spyOn(ipcBridge.agentPlatform.runtimeEngines.list, 'invoke').mockImplementation(
      () => new Promise((_resolve, fail) => { reject = fail; })
    );
    const screen = renderPage();
    expect(screen.getByText(copy.loading)).toBeTruthy();
    expect(screen.queryByRole('region', { name: 'Nomi Runtime' })).toBeNull();
    await act(async () => { reject(new Error('offline')); });
    expect(await screen.findByText(copy.loadError)).toBeTruthy();

    list.mockResolvedValue([]);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: copy.refresh })); });
    await waitFor(() => expect(screen.queryByText(copy.loadError)).toBeNull());
    expect(screen.getByText(copy.unavailable)).toBeTruthy();
    expect(screen.getByText(copy.unavailableHint)).toBeTruthy();
    fireEvent.click(screen.getByRole('link', { name: copy.javascriptLink }));
    expect(await screen.findByRole('heading', { name: 'Node.js settings' })).toBeTruthy();
  });

  test('marks cached diagnostics stale after a failed refresh and recovers on retry', async () => {
    list = spyOn(ipcBridge.agentPlatform.runtimeEngines.list, 'invoke').mockResolvedValue([engine('nomifun.nomi')]);
    const screen = renderPage();
    await screen.findByText(copy.healthy);
    list.mockRejectedValue(new Error('offline'));
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: copy.refresh })); });
    expect(await screen.findByText(copy.stale)).toBeTruthy();
    expect(screen.getByText('build-42')).toBeTruthy();
    list.mockResolvedValue([engine('nomifun.nomi', 'build-44')]);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: copy.refresh })); });
    expect(await screen.findByText('build-44')).toBeTruthy();
    expect(screen.queryByText(copy.stale)).toBeNull();
  });
});
