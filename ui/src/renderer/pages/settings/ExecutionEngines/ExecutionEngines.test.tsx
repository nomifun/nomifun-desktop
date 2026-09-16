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
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings: en } } } });
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

describe('execution engine settings', () => {
  test('loads real builds, refreshes additional families, and opens Agent configuration', async () => {
    list = spyOn(ipcBridge.agentPlatform.runtimeEngines.list, 'invoke').mockResolvedValue([
      engine('nomifun.nomi'), engine('nomifun.coding'), engine('nomifun.coding', 'build-43'),
    ]);
    const screen = renderPage();
    const coding = within(await screen.findByRole('region', { name: 'Coding Runtime' }));
    expect(coding.getByText(copy.available)).toBeTruthy();
    expect(coding.getByText('build-42')).toBeTruthy();
    expect(coding.getByText('build-43')).toBeTruthy();
    expect(coding.getAllByText('review').length).toBe(2);
    expect(coding.getAllByText('a'.repeat(64)).length).toBe(2);
    expect(list).toHaveBeenCalledTimes(1);

    list.mockResolvedValue([engine('nomifun.nomi'), engine('customer.workflow')]);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: copy.refresh })); });
    expect(await screen.findByRole('region', { name: 'customer.workflow' })).toBeTruthy();
    expect(coding.getByText(copy.unavailable)).toBeTruthy();
    expect(coding.queryByText('build-42')).toBeNull();
    fireEvent.click(screen.getByRole('link', { name: copy.configureLink }));
    expect(await screen.findByRole('heading', { name: 'Agent workbench' })).toBeTruthy();
  });

  test('does not claim missing engines when loading fails; retry can return an empty catalog', async () => {
    let reject!: (reason: Error) => void;
    list = spyOn(ipcBridge.agentPlatform.runtimeEngines.list, 'invoke').mockImplementation(() => new Promise((_resolve, fail) => { reject = fail; }));
    const screen = renderPage();
    expect(screen.getByText(copy.loading)).toBeTruthy();
    expect(screen.queryByText(copy.unavailable)).toBeNull();
    await act(async () => { reject(new Error('offline')); });
    expect(await screen.findByText(copy.loadError)).toBeTruthy();
    expect(screen.queryByRole('region', { name: 'Nomi Runtime' })).toBeNull();

    list.mockResolvedValue([]);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: copy.refresh })); });
    await waitFor(() => expect(screen.queryByText(copy.loadError)).toBeNull());
    expect(screen.getAllByText(copy.unavailable).length).toBe(2);
    expect(screen.queryByText(copy.available)).toBeNull();
    fireEvent.click(screen.getByRole('link', { name: copy.javascriptLink }));
    expect(await screen.findByRole('heading', { name: 'Node.js settings' })).toBeTruthy();
  });

  test('marks cached builds stale when refresh fails and recovers on retry', async () => {
    list = spyOn(ipcBridge.agentPlatform.runtimeEngines.list, 'invoke').mockResolvedValue([engine('nomifun.nomi')]);
    const screen = renderPage();
    await screen.findByText(copy.available);
    list.mockRejectedValue(new Error('offline'));
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: copy.refresh })); });
    expect(await screen.findByText(copy.loadError)).toBeTruthy();
    expect(screen.getByText('build-42')).toBeTruthy();
    expect(screen.queryByText(copy.available)).toBeNull();
    expect(screen.getAllByText(copy.stale).length).toBe(2);
    list.mockResolvedValue([engine('nomifun.nomi', 'build-44')]);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: copy.refresh })); });
    expect(await screen.findByText('build-44')).toBeTruthy();
    expect(screen.queryByText(copy.stale)).toBeNull();
    expect(screen.queryByText(copy.loadError)).toBeNull();
  });
});
