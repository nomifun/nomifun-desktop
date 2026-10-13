import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { StrictMode } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import { configService } from '@/common/config/configService';
import type { HostedApp } from '@/common/types/appHosting';
import type { OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { uuidv7 } from '@/common/utils';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import * as themeContext from '@/renderer/hooks/context/ThemeContext';
import locale from '../../services/i18n/locales/en-US/appHosting.json';
import AppWorkspaceRoute from './AppWorkspaceRoute';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { appHosting: locale } } }, interpolation: { escapeValue: false } });
const imported: HostedApp = {
  id: '0190f5fe-7c00-7a00-8000-000000000411', name: 'Manual application', description: '', tags: [], development_session_id: null,
  directory: 'C:/NomiFun/apps/manual-application', import_kind: 'source', settings: { project_directory: '.', environment: {} },
  state: 'stopped', operation: null, last_error: null, output: '', pid: null, preview: false, prepared: false,
};
const template = { template_key: 'app.developer', seed: { required_resource_kinds: ['workspace', 'process_session', 'project_memory'], enabled_capabilities: [] } } as unknown as OfficialPresetTemplate;

beforeEach(() => {
  configService.reset(); setBrowserStorageGeneration(uuidv7()); sessionStorage.clear();
  spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.conversation.listChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.conversation.turnStarted, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.conversation.turnCompleted, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.conversation.turnPaused, 'on').mockImplementation(() => () => {});
  spyOn(appHosting.developmentSessions, 'invoke').mockResolvedValue({ items: [], total: 0, owned_session_count: 0, has_more: false, next_cursor: null, project_directory: imported.directory, current_session_id: null });
  spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.mcpService.listServers, 'invoke').mockResolvedValue([]);
  spyOn(appHosting.mcpSources, 'invoke').mockResolvedValue([]);
  spyOn(appHosting.get, 'invoke').mockResolvedValue(imported);
  spyOn(themeContext, 'useThemeContext').mockReturnValue({ theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {} });
});
afterEach(() => { cleanup(); mock.restore(); configService.reset(); sessionStorage.clear(); });

function Location() {
  const location = useLocation();
  return <output data-testid='route'>{location.pathname}{location.search}</output>;
}

test('opening an application without a Session mounts the real composer and retains its draft when App details refresh', async () => {
  const openSession = spyOn(appHosting.developmentSession, 'invoke');
  const createApp = spyOn(appHosting.create, 'invoke');
  const updated = { ...imported, name: 'Refreshed application details', description: 'A metadata refresh preserves the project draft' };
  const getApp = spyOn(appHosting.get, 'invoke').mockResolvedValue(updated);
  const view = render(<MemoryRouter initialEntries={[`/apps/${imported.id}`]}>
    <I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), revalidateOnMount: false, shouldRetryOnError: false, dedupingInterval: 0,
      fallback: { providers: [], [`app/${imported.id}`]: imported, 'agent-presets.library': { official_templates: [template], user_presets: [], active_bindings: [] } },
    }}><StrictMode><Location /><Routes>
      <Route path='/apps/:id' element={<AppWorkspaceRoute />} />
    </Routes></StrictMode></SWRConfig></I18nextProvider>
  </MemoryRouter>);
  await view.findByRole('heading', { name: imported.name });
  const draft = view.getByTestId('guid-input') as HTMLTextAreaElement;
  expect(draft.tagName).toBe('TEXTAREA');
  expect(view.getByTestId('fixed-app-agent').textContent).toBe(locale.workspace.fixedAgent);
  fireEvent.change(draft, { target: { value: 'Keep developing this application after its details refresh' } });
  fireEvent.click(view.getByRole('button', { name: locale.actions.refresh }));
  await waitFor(() => expect(getApp).toHaveBeenCalledWith({ id: imported.id }));
  await view.findByRole('heading', { name: updated.name });
  expect(view.getByTestId('guid-input')).toBe(draft);
  expect(draft.value).toBe('Keep developing this application after its details refresh');
  expect(view.getByTestId('route').textContent).toBe(`/apps/${imported.id}`);
  expect(createApp).not.toHaveBeenCalled();
  expect(openSession).not.toHaveBeenCalled();
});

test('manual owner admission navigates away from import while the real fixed-Agent Guid unmounts', async () => {
  let complete!: (app: HostedApp) => void;
  const importCall = spyOn(appHosting.import, 'invoke').mockImplementation(() => new Promise(resolve => { complete = resolve; }));
  const openSession = spyOn(appHosting.developmentSession, 'invoke');
  const createApp = spyOn(appHosting.create, 'invoke');
  const view = render(<MemoryRouter initialEntries={['/apps/import']}>
    <I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), revalidateOnMount: false, shouldRetryOnError: false,
      fallback: { providers: [], [`app/${imported.id}`]: imported, 'agent-presets.library': { official_templates: [template], user_presets: [], active_bindings: [] } },
    }}><StrictMode><Location /><Routes>
      <Route path='/apps/import' element={<AppWorkspaceRoute />} />
      <Route path='/apps/:id' element={<AppWorkspaceRoute />} />
    </Routes></StrictMode></SWRConfig></I18nextProvider>
  </MemoryRouter>);
  expect(view.getByTestId('fixed-app-agent').textContent).toBe(locale.workspace.fixedAgent);
  const draft = view.getByTestId('guid-input') as HTMLTextAreaElement;
  fireEvent.change(draft, { target: { value: 'Continue developing this imported application' } });
  fireEvent.change(view.getByRole('textbox', { name: locale.import.hostPath }), { target: { value: 'C:/original/project' } });
  fireEvent.change(view.getByRole('textbox', { name: locale.name }), { target: { value: imported.name } });
  fireEvent.click(view.getByRole('button', { name: locale.import.action }));
  await waitFor(() => expect(importCall).toHaveBeenCalledTimes(1));
  expect((view.getByTestId('guid-input') as HTMLTextAreaElement).value).toBe('Continue developing this imported application');
  await act(async () => { complete(imported); });
  await waitFor(() => expect(view.getByTestId('route').textContent).toBe(`/apps/${imported.id}?intent=import`));
  await view.findByRole('heading', { name: imported.name });
  expect((view.getByTestId('guid-input') as HTMLTextAreaElement).value).toBe('Continue developing this imported application');
  expect(view.getByTestId('fixed-app-agent').textContent).toBe(locale.workspace.fixedAgent);
  expect(createApp).not.toHaveBeenCalled();
  expect(openSession).not.toHaveBeenCalled();
});
