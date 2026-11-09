import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { configService } from '@/common/config/configService';
import { modelProtocol } from '@/common/adapter/ipcBridge';
import type { IProvider } from '@/common/config/storage';
import { parseProviderId } from '@/common/types/ids';
import type { ModelProtocolManifestResponse } from '@/common/protocolBindings/ModelProtocolManifestResponse';
import type { ProtocolDescriptor } from '@/common/protocolBindings/ProtocolDescriptor';
import type { ProtocolExecutorKind } from '@/common/protocolBindings/ProtocolExecutorKind';
import type { ProtocolTransportKind } from '@/common/protocolBindings/ProtocolTransportKind';
import { useModelsForTask } from './useModelsForTask';
import { COMPANION_SPEECH_MODEL_PROTOCOLS, FILE_SPEECH_MODEL_PROTOCOLS } from './useTaskModelProtocols';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import zhCommon from '@/renderer/services/i18n/locales/zh-CN/common.json';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN',
  resources: { 'zh-CN': { translation: { common: zhCommon, settings: zhSettings } } }, interpolation: { escapeValue: false } });

const id = parseProviderId('0190f5fe-7c00-7a00-8000-000000000001');
const protocol = (protocol_id: string, executor: ProtocolExecutorKind, transport: ProtocolTransportKind): ProtocolDescriptor => ({
  protocol_id, executor, transport, supported_tasks: ['speech_recognition'], requires_output_ceiling: false,
  allowed_auth_schemes: ['bearer'], scopes: ['custom'], platforms: [], default_connections: [], endpoints: [], root_shape: null,
});
const manifest: ModelProtocolManifestResponse = {
  tasks: ['speech_recognition'], requested_task: 'speech_recognition', preset: 'custom', platform: 'custom',
  platform_default_base_url: null, requires_user_input: false, default_auth_scheme: null, auth_schemes: [], recommendation: null,
  protocols: [protocol('test.upload', 'model_invoke', 'http'), protocol('test.stream', 'model_invoke', 'websocket'),
    protocol('test.session', 'realtime_session', 'websocket'), protocol('test.async', 'async_job', 'http')],
};
const provider: IProvider = {
  id, platform: 'custom', name: 'Speech', base_url: 'https://example.test', auth_scheme: 'bearer', has_credentials: true,
  models: ['upload', 'stream', 'session', 'async', 'unknown'].map(name => ({
    provider_id: id, model: name, enabled: true, sort_order: 0, created_at: 1, updated_at: 1,
    capabilities: [{ task: 'speech_recognition', traits: [], protocol: 'test.' + name, connection_role: 'default',
      allow_cross_origin_credentials: false, provider_params: {}, created_at: 1, updated_at: 1 }],
  })),
};

function observe(scoped: boolean, onRefresh?: (refresh: () => void) => void) {
  const result = useModelsForTask('speech_recognition', undefined, undefined, scoped ? FILE_SPEECH_MODEL_PROTOCOLS : undefined);
  onRefresh?.(result.refresh);
  return <output>{result.isLoading ? result.error ? 'unresolved-error' : 'loading' : result.groups.flatMap(group => group.models).join(',')}</output>;
}

function setup() {
  const providers = spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([provider]);
  const changes = spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockReturnValue(() => {});
  const reconnects = new Set<() => void>();
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(callback => {
    const notify = () => callback();
    reconnects.add(notify);
    return () => { reconnects.delete(notify); };
  });
  const reload = spyOn(configService, 'reload').mockResolvedValue(undefined);
  const registry = spyOn(modelProtocol.list, 'invoke').mockResolvedValue(manifest);
  return { registry, reconnects, notifyReconnect: () => { for (const notify of reconnects) notify(); },
    restore() { cleanup(); providers.mockRestore(); changes.mockRestore(); reconnect.mockRestore(); reload.mockRestore(); registry.mockRestore(); } };
}

test('task-only selectors retain their catalog without requesting a protocol manifest', async () => {
  const fixture = setup();
  function Selector() { return observe(false); }
  try {
    const page = render(<SWRConfig value={{ provider: () => new Map() }}><Selector /></SWRConfig>);
    await waitFor(() => expect(page.container.textContent).toBe('upload,stream,session,async,unknown'));
    expect(fixture.registry).not.toHaveBeenCalled();
  } finally { fixture.restore(); }
});

test('file speech selectors share registry metadata and reject session, async, and unknown protocols', async () => {
  const fixture = setup();
  function Selector() { return observe(true); }
  try {
    const page = render(<SWRConfig value={{ provider: () => new Map() }}><Selector /><Selector /></SWRConfig>);
    await waitFor(() => expect(page.container.textContent).toBe('uploadupload'));
    expect(fixture.registry).toHaveBeenCalledTimes(1);
    expect(fixture.registry).toHaveBeenCalledWith({ preset: 'custom', task: 'speech_recognition' });
  } finally { fixture.restore(); }
});

test('failed execution scope stays unresolved and can recover through explicit refresh', async () => {
  const fixture = setup();
  let refresh: (() => void) | undefined;
  fixture.registry.mockRejectedValueOnce(new Error('registry unavailable'));
  function Selector() { return observe(true, value => { refresh = value; }); }
  try {
    const page = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><Selector /></SWRConfig>);
    await waitFor(() => expect(page.container.textContent).toBe('unresolved-error'));
    expect(provider.models.map(model => model.model)).toContain('session');
    await act(async () => { refresh?.(); });
    await waitFor(() => expect(page.container.textContent).toBe('upload'));
  } finally { fixture.restore(); }
});

test('companion speech retains its incremental executor while excluding other sessions and async jobs', async () => {
  const fixture = setup();
  function Selector() {
    const result = useModelsForTask('speech_recognition', undefined, undefined, COMPANION_SPEECH_MODEL_PROTOCOLS);
    return <output>{result.isLoading ? 'loading' : result.groups.flatMap(group => group.models).join(',')}</output>;
  }
  try {
    const page = render(<SWRConfig value={{ provider: () => new Map() }}><Selector /></SWRConfig>);
    await waitFor(() => expect(page.container.textContent).toBe('upload,stream'));
  } finally { fixture.restore(); }
});

test('a backend reconnect recovers every scoped selector through one shared subscription', async () => {
  const fixture = setup();
  fixture.registry.mockRejectedValueOnce(new Error('backend restarting'));
  function Selector() { return observe(true); }
  try {
    const page = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><Selector /><Selector /></SWRConfig>);
    await waitFor(() => expect(page.container.textContent).toBe('unresolved-errorunresolved-error'));
    // Providers and protocol registry each own one shared subscription.
    expect(fixture.reconnects.size).toBe(2);
    expect(fixture.registry).toHaveBeenCalledTimes(1);
    await act(async () => { fixture.notifyReconnect(); });
    await waitFor(() => expect(page.container.textContent).toBe('uploadupload'));
    expect(fixture.registry).toHaveBeenCalledTimes(2);
    page.unmount();
    expect(fixture.reconnects.size).toBe(0);
  } finally { fixture.restore(); }
});

test('the real model selector preserves the saved reference and exposes retry after a registry failure', async () => {
  const fixture = setup();
  fixture.registry.mockRejectedValueOnce(new Error('registry unavailable'));
  const selection = { provider_id: id, model: 'upload' };
  const writes: unknown[] = [];
  try {
    const page = render(<I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}>
      <TaskModelSelect task='speech_recognition' requiredProtocols={FILE_SPEECH_MODEL_PROTOCOLS}
        value={selection} onChange={value => { writes.push(value); }} hideHint />
    </SWRConfig></I18nextProvider>);
    await waitFor(() => expect(page.getByRole('alert').textContent).toContain(zhSettings.taskModel.loadFailed));
    expect(page.container.textContent).toContain('Speech');
    expect(page.container.textContent).toContain('upload');
    expect(page.container.textContent).not.toContain('不可用');
    expect(writes).toEqual([]);
    fireEvent.click(page.getByRole('button', { name: zhCommon.retry }));
    await waitFor(() => expect(page.queryByRole('alert')).toBeNull());
    expect(page.container.textContent).toContain('upload');
    expect(fixture.registry).toHaveBeenCalledTimes(2);
    expect(writes).toEqual([]);
    expect(selection).toEqual({ provider_id: id, model: 'upload' });
  } finally { fixture.restore(); }
});
