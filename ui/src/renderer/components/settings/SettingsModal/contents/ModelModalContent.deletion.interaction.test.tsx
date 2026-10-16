import '../../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { SWRConfig } from 'swr';
import type { IProvider } from '@/common/config/storage';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import { aliasCapability, aliasProtocolManifest, aliasProviderId } from '../../../../../../test/fixtures/modelAliasEditor';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
const testFetch = (async () => new Response(JSON.stringify({ success: true, data: [] }), {
  headers: { 'Content-Type': 'application/json' },
})) as typeof fetch;
const installTransport = () => { globalThis.fetch = testFetch; window.fetch = testFetch; };
const restoreTransport = () => { globalThis.fetch = originalFetch; window.fetch = originalWindowFetch; };
installTransport();
const { ipcBridge } = await import('@/common');
const { BackendHttpError } = await import('@/common/adapter/httpBridge');
const { configService } = await import('@/common/config/configService');
const { ThemeProvider } = await import('@/renderer/hooks/context/ThemeContext');
const { default: ModelModalContent } = await import('./ModelModalContent');
restoreTransport();
beforeEach(installTransport);
const restorers: Array<() => void> = [];
afterEach(() => { cleanup(); restorers.splice(0).forEach(restore => restore()); restoreTransport(); });

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings, common } } }, interpolation: { escapeValue: false } });
const MODEL = 'exact-model-id';
const ALIAS = 'Friendly model';
const PROVIDER = 'Friendly provider';
const AGENT_ID = '0190f5fe-7c00-7a00-8000-000000000057';
function watch<T extends { mockRestore: () => void }>(spy: T): T { restorers.push(() => spy.mockRestore()); return spy; }
function LocationProbe() { const location = useLocation(); return <output data-testid='location'>{location.pathname}</output>; }

function mount({ hold = false, failOnce, realSettings = false }: { hold?: boolean; failOnce?: Error; realSettings?: boolean } = {}) {
  watch(spyOn(configService, 'whenReady').mockResolvedValue(undefined));
  const reload = watch(spyOn(configService, 'reload'));
  if (!realSettings) reload.mockResolvedValue(undefined);
  else {
    const settingsFetch = (async () => new Response(JSON.stringify({ success: true, data: {} }), {
      headers: { 'Content-Type': 'application/json' },
    })) as typeof fetch;
    globalThis.fetch = settingsFetch; window.fetch = settingsFetch;
  }
  watch(spyOn(ipcBridge.application.applyKeepAwake, 'invoke').mockResolvedValue(undefined));
  watch(spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation(() => () => {}));
  watch(spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(() => () => {}));
  watch(spyOn(ipcBridge.providerConnection.list, 'invoke').mockResolvedValue([]));
  watch(spyOn(ipcBridge.modelProtocol.list, 'invoke').mockResolvedValue(aliasProtocolManifest));
  let providers: IProvider[] = [{ id: aliasProviderId, platform: 'openai', name: PROVIDER,
    base_url: 'https://example.invalid/v1', auth_scheme: 'bearer', has_credentials: true, enabled: true, sort_order: 0,
    models: [MODEL, 'second-model'].map((model, index) => ({ provider_id: aliasProviderId, model,
      ...(index === 0 ? { display_name: ALIAS } : {}), enabled: true, sort_order: index,
      capabilities: realSettings ? [{ ...aliasCapability, task: 'image_generation', protocol: 'openai.images',
        endpoint: '/images/generations', traits: [], provider_params: {} }] : [aliasCapability], created_at: 1, updated_at: 1 })),
  }];
  const list = watch(spyOn(ipcBridge.mode.listProviders, 'invoke').mockImplementation(async () => structuredClone(providers)));
  let complete!: () => void;
  const pending = new Promise<void>(resolve => { complete = resolve; });
  let failed = false;
  const beforeDelete = async () => {
    if (failOnce && !failed) { failed = true; throw failOnce; }
    if (hold) await pending;
  };
  const removeModel = watch(spyOn(ipcBridge.providerModel.remove, 'invoke').mockImplementation(async request => {
    await beforeDelete();
    providers = providers.map(provider => ({ ...provider, models: provider.models.filter(row => row.model !== request.model) }));
  }));
  const removeProvider = watch(spyOn(ipcBridge.mode.deleteProvider, 'invoke').mockImplementation(async () => {
    await beforeDelete(); providers = [];
  }));
  const saveModel = watch(spyOn(ipcBridge.providerModel.save, 'invoke'));
  const writeSettings = watch(spyOn(configService, 'set'));
  const clearSettings = watch(spyOn(configService, 'remove'));
  const page = render(<I18nextProvider i18n={i18n}><ThemeProvider><MemoryRouter initialEntries={['/models']}>
    <SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><ModelModalContent /><LocationProbe /></SWRConfig>
  </MemoryRouter></ThemeProvider></I18nextProvider>);
  const open = async (kind: 'provider' | 'model') => {
    if (kind === 'provider') {
      fireEvent.click(await page.findByRole('button', { name: settings.deleteProvider }));
    } else {
      fireEvent.click(await page.findByText(PROVIDER));
      const label = await page.findByText(ALIAS, { exact: true });
      const row = label.closest('.model-provider-sortable-row') as HTMLElement;
      fireEvent.click(within(row).getByRole('button', { name: settings.modelDeletion.deleteModel }));
    }
    return within(await page.findByRole('dialog'));
  };
  return { page, open, removeModel, removeProvider, list, reload, complete, saveModel, writeSettings, clearSettings };
}

describe('model and provider deletion confirmation', () => {
  for (const kind of ['provider', 'model'] as const) {
    test(`${kind} shows names and reference-cleanup consequences before one confirmation; cancel has no side effects`, async () => {
      const fixture = mount();
      const dialog = await fixture.open(kind);
      expect(dialog.getByText(settings.modelDeletion.effects)).toBeTruthy();
      expect(dialog.getByText(settings.modelDeletion.preserve)).toBeTruthy();
      expect(fixture.page.getAllByRole('dialog')).toHaveLength(1);
      if (kind === 'model') {
        expect(dialog.getByText(`Delete model “${ALIAS}”?`)).toBeTruthy();
        expect(dialog.getByText(`${PROVIDER} · ${MODEL}`)).toBeTruthy();
      } else {
        expect(dialog.getByText(`Delete provider “${PROVIDER}”?`)).toBeTruthy();
        expect(dialog.getByText('This deletes the provider and all its models (2).')).toBeTruthy();
      }
      expect(fixture.removeModel).not.toHaveBeenCalled();
      expect(fixture.removeProvider).not.toHaveBeenCalled();
      fireEvent.click(dialog.getByRole('button', { name: common.cancel }));
      await waitFor(() => expect(fixture.page.queryByRole('dialog') === null).toBe(true));
      expect(fixture.removeModel).not.toHaveBeenCalled();
      expect(fixture.removeProvider).not.toHaveBeenCalled();
      expect(fixture.reload).not.toHaveBeenCalled();
      expect(fixture.page.getByText(PROVIDER, { exact: true })).toBeTruthy();
    });

    test(`${kind} waits for DELETE success, suppresses duplicate requests, then refreshes settings without picking replacements`, async () => {
      const fixture = mount({ hold: true });
      const dialog = await fixture.open(kind);
      const confirm = dialog.getByRole('button', { name: settings.modelDeletion.confirm }) as HTMLButtonElement;
      fireEvent.click(confirm); fireEvent.click(confirm);
      const remove = kind === 'model' ? fixture.removeModel : fixture.removeProvider;
      await waitFor(() => expect(remove).toHaveBeenCalledTimes(1));
      expect(confirm.disabled).toBe(true);
      expect(confirm.classList.contains('arco-btn-loading')).toBe(true);
      expect((dialog.getByRole('button', { name: common.cancel }) as HTMLButtonElement).disabled).toBe(true);
      expect(fixture.page.getByText(PROVIDER, { exact: true })).toBeTruthy();
      if (kind === 'model') expect(fixture.page.getByText(ALIAS, { exact: true })).toBeTruthy();
      expect(fixture.reload).not.toHaveBeenCalled();
      await act(async () => fixture.complete());
      await waitFor(() => expect(fixture.page.queryByRole('dialog') === null).toBe(true));
      await waitFor(() => expect(fixture.reload).toHaveBeenCalledTimes(1));
      if (kind === 'model') {
        expect(remove.mock.calls[0]).toEqual([{ provider_id: aliasProviderId, model: MODEL }]);
        await waitFor(() => expect(fixture.page.queryByText(ALIAS, { exact: true }) === null).toBe(true));
        expect(fixture.page.getByText('second-model', { exact: true })).toBeTruthy();
      } else {
        expect(remove.mock.calls[0]).toEqual([{ provider_id: aliasProviderId }]);
        await waitFor(() => expect(fixture.page.queryByText(PROVIDER, { exact: true }) === null).toBe(true));
      }
      expect(fixture.list.mock.calls.length >= 2).toBe(true);
      expect(fixture.saveModel).not.toHaveBeenCalled();
      expect(fixture.writeSettings).not.toHaveBeenCalled();
      expect(fixture.clearSettings).not.toHaveBeenCalled();
    });
  }

  test('an actionable failure stays in the same dialog for retry without removing the row', async () => {
    const detail = 'Permission check failed. Sign in again, then retry.';
    const fixture = mount({ failOnce: new BackendHttpError({ method: 'DELETE', path: '/api/provider-models', status: 403,
      body: { code: 'FORBIDDEN', error: detail } }) });
    const dialog = await fixture.open('model');
    fireEvent.click(dialog.getByRole('button', { name: settings.modelDeletion.confirm }));
    expect((await fixture.page.findByRole('alert')).textContent).toBe(detail);
    expect(fixture.page.getAllByRole('dialog')).toHaveLength(1);
    expect(fixture.page.getByText(ALIAS, { exact: true })).toBeTruthy();
    expect(fixture.reload).not.toHaveBeenCalled();
    fireEvent.click(dialog.getByRole('button', { name: settings.modelDeletion.retry }));
    await waitFor(() => expect(fixture.removeModel).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(fixture.page.queryByRole('dialog') === null).toBe(true));
    expect(fixture.reload).toHaveBeenCalledTimes(1);
  });

  test('the real settings reload leaves a detached default empty even when another model is available', async () => {
    const key = 'models.default.imageGeneration';
    configService.setLocal(key, { provider_id: aliasProviderId, model: MODEL });
    expect(configService.get(key)?.model).toBe(MODEL);
    const fixture = mount({ realSettings: true });
    const dialog = await fixture.open('model');
    fireEvent.click(dialog.getByRole('button', { name: settings.modelDeletion.confirm }));
    await waitFor(() => expect(fixture.reload).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(configService.get(key)).toBeUndefined());
    expect(fixture.page.getByText('second-model', { exact: true })).toBeTruthy();
    expect(fixture.saveModel).not.toHaveBeenCalled();
    expect(fixture.writeSettings).not.toHaveBeenCalled();
    expect(fixture.clearSettings).not.toHaveBeenCalled();
  });

  test('a legacy in-use failure never exposes agent IDs, opens a second confirmation, or navigates to unbind', async () => {
    const fixture = mount({ failOnce: new BackendHttpError({ method: 'DELETE', path: `/api/providers/${aliasProviderId}`, status: 409,
      body: { code: 'PROVIDER_IN_USE', error: `Agent ${AGENT_ID} still references this provider`,
        details: { usages: [{ feature: 'agent', label: AGENT_ID, targetId: AGENT_ID }] } } }) });
    const dialog = await fixture.open('provider');
    fireEvent.click(dialog.getByRole('button', { name: settings.modelDeletion.confirm }));
    await fixture.page.findByText(settings.modelDeletion.referencesFailed);
    expect(fixture.page.getAllByRole('dialog')).toHaveLength(1);
    expect(fixture.page.queryByText(new RegExp(AGENT_ID))).toBeNull();
    expect(fixture.page.queryByText(/Go fix it|Unbind or switch models/)).toBeNull();
    expect(fixture.page.getByTestId('location').textContent).toBe('/models');
    expect(fixture.page.getByText(PROVIDER, { exact: true })).toBeTruthy();
    expect(fixture.reload).not.toHaveBeenCalled();
    expect(dialog.getByRole('button', { name: settings.modelDeletion.retry })).toBeTruthy();
  });
});
