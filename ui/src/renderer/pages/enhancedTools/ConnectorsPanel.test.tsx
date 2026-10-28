import { afterEach, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { createMemoryRouter, RouterProvider } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { feishuConnector } from '@/common/adapter/feishuConnectorBridge';
import type { FeishuStatus } from '@/common/types/feishuConnector';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import feishu from '@/renderer/services/i18n/locales/en-US/feishu.json';
import knowledge from '@/renderer/services/i18n/locales/en-US/knowledge.json';
import ConnectorsPanel from './ConnectorsPanel';

const locale = createInstance();
const restores: Array<() => void> = [];
const track = <T extends { mockRestore: () => void }>(spy: T): T => { restores.push(() => spy.mockRestore()); return spy; };
const actor = { app_id: 'cli_app', user_id: 'user_1', name: 'Ada' };
const ready = (): FeishuStatus => ({
  provider: 'feishu',
  connection: { id: 'connection', name: 'Work connection', enabled: true, profile: 'work', executable: null, created_at: '2026-10-26T00:00:00Z', updated_at: '2026-10-26T00:00:00Z' },
  runtime: { phase: 'ready', executable: 'C:/tools/lark-cli.exe', version: '1.0.98', source: 'external', message: null, official_install_command: 'npx @larksuite/cli@latest install' },
  phase: 'ready', actor, host_name: 'backend-machine', host_os: 'windows', message: null,
  permissions: ['documents.browse', 'documents.create', 'documents.delete', 'base.read', 'base.write'].map(capability => ({ capability, granted: true, required_scopes: [capability + '-scope'] })),
});

beforeAll(async () => { await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { settings, common, feishu, knowledge } } } }); });
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });

const mount = async () => {
  const router = createMemoryRouter([{ path: '/enhanced-tools', element: <ConnectorsPanel /> }, { path: '/knowledge/feishu', element: <div>Feishu space</div> }], { initialEntries: ['/enhanced-tools'] });
  restores.push(() => router.dispose());
  let view!: ReturnType<typeof render>;
  await act(async () => { view = render(<I18nextProvider i18n={locale}><RouterProvider router={router} /></I18nextProvider>); });
  return { ...view, router };
};

describe('Connectors panel', () => {
  test('shows connector status, account and provided capabilities when connected', async () => {
    track(spyOn(feishuConnector, 'status').mockResolvedValue(ready()));
    const view = await mount();
    const card = await view.findByTestId('connector-card-feishu');
    expect(card.textContent).toContain(settings.connectors.feishu.name);
    expect(card.textContent).toContain(settings.connectors.status.ready);
    expect(card.textContent).toContain(actor.name);
    expect(card.textContent).toContain('lark-cli');
    const knowledgeEntry = card.querySelector<HTMLButtonElement>('button.connector-capability');
    expect(knowledgeEntry?.disabled).toBe(false);
    fireEvent.click(knowledgeEntry!);
    expect(view.router.state.location.pathname).toBe('/knowledge/feishu');
  });

  test('keeps capabilities gated until the connection is configured', async () => {
    track(spyOn(feishuConnector, 'status').mockResolvedValue({ ...ready(), phase: 'needs_login', actor: null }));
    const view = await mount();
    const card = await view.findByTestId('connector-card-feishu');
    expect(card.textContent).toContain(settings.connectors.status.needs_login);
    expect(card.textContent).toContain(settings.connectors.configureFirst);
    expect(card.querySelector<HTMLButtonElement>('button.connector-capability')?.disabled).toBe(true);
  });

  test('offers repair for an unusable component instead of a dead end', async () => {
    track(spyOn(feishuConnector, 'status').mockResolvedValue({ ...ready(), runtime: { ...ready().runtime, phase: 'broken', message: 'component cannot launch' } }));
    const repair = track(spyOn(feishuConnector, 'repair').mockResolvedValue(ready().runtime));
    const view = await mount();
    const card = await view.findByTestId('connector-card-feishu');
    expect(card.textContent).toContain(feishu.runtime.phase.broken);
    fireEvent.click(within(card).getByRole('button', { name: feishu.actions.repair }));
    await waitFor(() => expect(repair).toHaveBeenCalledTimes(1));
  });

  test('opens the shared connection settings dialog from the card', async () => {
    track(spyOn(feishuConnector, 'status').mockResolvedValue(ready()));
    track(spyOn(feishuConnector, 'profiles').mockResolvedValue([{ name: 'work', app_id: actor.app_id, brand: 'feishu', user_name: actor.name, token_status: 'valid' }]));
    const view = await mount();
    await view.findByTestId('connector-card-feishu');
    fireEvent.click(view.getByRole('button', { name: feishu.connection.settings }));
    await waitFor(() => expect(view.getByRole('dialog')).toBeTruthy());
  });
});
