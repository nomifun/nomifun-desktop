import '../../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import type { IProvider, ModelTask } from '@/common/config/storage';
import type { HealthStatus } from '@/common/protocolBindings/HealthStatus';
import type { ProviderModelCapabilityResponse } from '@/common/types/provider/providerModel';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import { aliasProtocolManifest, aliasProviderId } from '../../../../../../test/fixtures/modelAliasEditor';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
const testFetch = (async () => new Response(JSON.stringify({ success: true, data: [] }), {
  headers: { 'Content-Type': 'application/json' },
})) as typeof fetch;
const installTransport = () => { globalThis.fetch = testFetch; window.fetch = testFetch; };
const restoreTransport = () => { globalThis.fetch = originalFetch; window.fetch = originalWindowFetch; };
installTransport();
const { ipcBridge } = await import('@/common');
const { configService } = await import('@/common/config/configService');
const { ThemeProvider } = await import('@/renderer/hooks/context/ThemeContext');
const { default: ModelModalContent } = await import('./ModelModalContent');
restoreTransport();
beforeEach(installTransport);
const restorers: Array<() => void> = [];
afterEach(() => { cleanup(); restorers.splice(0).forEach(restore => restore()); restoreTransport(); });

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US',
  resources: { 'en-US': { translation: { settings, common } } }, interpolation: { escapeValue: false },
});
const MODEL = 'health-fixture-model';
const PROVIDER_NAME = 'Health fixture provider';
const CHECKED_AT = 1_760_000_000_000;
const protocols: Partial<Record<ModelTask, string>> = {
  chat: 'openai.chat_text', video_generation: 'openai.videos', realtime_conversation: 'stepfun.realtime_s2s',
};
const capability = (task: ModelTask, status?: HealthStatus): ProviderModelCapabilityResponse => ({
  task, traits: [], protocol: protocols[task]!, connection_role: 'default', allow_cross_origin_credentials: false,
  provider_params: {}, ...(status ? { health: { status, latency: 12 }, health_checked_at: CHECKED_AT } : {}),
  created_at: 1, updated_at: 1,
});
function watch<T extends { mockRestore: () => void }>(spy: T): T {
  restorers.push(() => spy.mockRestore()); return spy;
}

function mount(capabilities: ProviderModelCapabilityResponse[], resultStatus: HealthStatus = 'unknown') {
  watch(spyOn(configService, 'whenReady').mockResolvedValue(undefined));
  watch(spyOn(ipcBridge.application.applyKeepAwake, 'invoke').mockResolvedValue(undefined));
  watch(spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation(() => () => {}));
  watch(spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(() => () => {}));
  watch(spyOn(ipcBridge.providerConnection.list, 'invoke').mockResolvedValue([]));
  watch(spyOn(ipcBridge.modelProtocol.list, 'invoke').mockImplementation(async ({ task = 'chat' }) => ({
    ...aliasProtocolManifest, preset: 'openai', platform: 'openai', tasks: [task], requested_task: task,
    recommendation: null, protocols: [{ ...aliasProtocolManifest.protocols[0]!,
      protocol_id: protocols[task]!, supported_tasks: [task], endpoints: [],
    }],
  })));
  let provider: IProvider = { id: aliasProviderId, platform: 'openai', name: PROVIDER_NAME,
    base_url: 'https://example.invalid/v1', auth_scheme: 'bearer', has_credentials: true, enabled: true, sort_order: 0,
    models: [{ provider_id: aliasProviderId, model: MODEL, enabled: true, sort_order: 0,
      capabilities, created_at: 1, updated_at: 1 }],
  };
  const list = watch(spyOn(ipcBridge.mode.listProviders, 'invoke').mockImplementation(async () => [structuredClone(provider)]));
  const check = watch(spyOn(ipcBridge.agentConversation.checkProviderHealth, 'invoke').mockImplementation(async request => {
    provider = { ...provider, models: provider.models.map(row => ({ ...row, capabilities: row.capabilities.map(cap =>
      cap.task === request.task ? { ...cap, health: { status: resultStatus, latency: 12,
        ...(resultStatus === 'unhealthy' ? { error: 'Local fixture failure' } : {}) }, health_checked_at: CHECKED_AT } : cap),
    })) };
    return { provider_id: provider.id, platform: provider.platform, model: request.model, task: request.task,
      status: resultStatus, elapsed_ms: 12, ...(resultStatus === 'unhealthy' ? { message: 'Local fixture failure' } : {}),
    };
  }));
  const info = watch(spyOn(Message, 'info').mockReturnValue(() => {}));
  const success = watch(spyOn(Message, 'success').mockReturnValue(() => {}));
  const error = watch(spyOn(Message, 'error').mockReturnValue(() => {}));
  const page = render(<I18nextProvider i18n={i18n}><ThemeProvider><MemoryRouter>
    <SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><ModelModalContent /></SWRConfig>
  </MemoryRouter></ThemeProvider></I18nextProvider>);
  const open = async () => {
    fireEvent.click(await page.findByText(PROVIDER_NAME));
    const label = await page.findByText(MODEL, { exact: true });
    const row = label.closest('.model-provider-sortable-row') as HTMLElement;
    expect(Boolean(row)).toBe(true);
    return row;
  };
  return { page, open, check, list, info, success, error };
}

describe('model health presentation for checked and unverified tasks', () => {
  for (const task of ['video_generation', 'realtime_conversation'] as const) {
    test(`${task} Unknown health uses only a neutral information toast`, async () => {
      const { open, check, list, info, success, error } = mount([capability(task)]);
      const row = await open();
      fireEvent.click(within(row).getByRole('button', { name: settings.healthCheck }));
      await waitFor(() => expect(info).toHaveBeenCalledTimes(1));
      expect(check.mock.calls).toEqual([[{ provider_id: aliasProviderId, model: MODEL, task }]]);
      expect(list.mock.calls.length >= 2).toBe(true);
      expect(success).not.toHaveBeenCalled();
      expect(error).not.toHaveBeenCalled();
      expect(info.mock.calls[0]![0]).toMatchObject({ content: expect.stringContaining(
        task === 'realtime_conversation' ? settings.healthRealtimeUnverified : settings.healthAcceptedUnverified,
      ) });
      expect(row.querySelector('[data-model-health-status]')?.getAttribute('data-model-health-status')).toBe('unknown');
    });
  }

  test('a healthy chat plus checked Unknown video stays neutral and both tooltips explain verification', async () => {
    const { page, open } = mount([capability('chat', 'healthy'), capability('video_generation', 'unknown')]);
    const row = await open();
    const indicator = row.querySelector<HTMLElement>('[data-model-health-status]')!;
    expect(indicator.getAttribute('data-model-health-status')).toBe('unknown');
    expect(indicator.classList.contains('bg-green-500')).toBe(false);
    expect(row.querySelector('[data-capability-health-task="chat"]')?.getAttribute('data-capability-health-status')).toBe('healthy');
    const video = row.querySelector<HTMLElement>('[data-capability-health-task="video_generation"]')!;
    expect(video.getAttribute('data-capability-health-status')).toBe('unknown');
    expect(video.classList.contains('arco-tag-green')).toBe(false);
    fireEvent.mouseEnter(indicator);
    await waitFor(() => expect(Array.from(page.baseElement.querySelectorAll('.arco-tooltip-content'))
      .some(element => element.textContent?.includes(settings.healthUnverified) && element.textContent.includes(settings.healthAcceptedUnverified))).toBe(true));
    fireEvent.mouseLeave(indicator);
    fireEvent.mouseEnter(video);
    await waitFor(() => expect(Boolean(page.baseElement.querySelector('[data-capability-health-tooltip="video_generation"]'))).toBe(true));
    const tooltip = page.baseElement.querySelector('[data-capability-health-tooltip="video_generation"]')!.textContent!;
    expect(tooltip).toContain(settings.healthUnverified);
    expect(tooltip).toContain(settings.healthAcceptedUnverified);
    expect(tooltip).not.toContain(settings.healthNotChecked);
    expect(tooltip).not.toContain(common.failed);
  });

  for (const status of ['healthy', 'unhealthy'] as const) {
    test(`${status} health keeps its existing toast and row color`, async () => {
      const { open, check, info, success, error } = mount([capability('chat')], status);
      const row = await open();
      fireEvent.click(within(row).getByRole('button', { name: settings.healthCheck }));
      await waitFor(() => expect(status === 'healthy' ? success : error).toHaveBeenCalledTimes(1));
      expect(check).toHaveBeenCalledTimes(1);
      expect(info).not.toHaveBeenCalled();
      expect(status === 'healthy' ? error : success).not.toHaveBeenCalled();
      const indicator = row.querySelector<HTMLElement>('[data-model-health-status]')!;
      expect(indicator.getAttribute('data-model-health-status')).toBe(status);
      expect(indicator.classList.contains(status === 'healthy' ? 'bg-green-500' : 'bg-red-500')).toBe(true);
    });
  }
});
