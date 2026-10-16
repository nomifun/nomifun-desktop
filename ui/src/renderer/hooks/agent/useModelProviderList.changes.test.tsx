import '../../../../test/setup-dom.ts';
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { expect, spyOn, test } from 'bun:test';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { configService } from '@/common/config/configService';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import { asAgentPresetId, type AgentPresetLibraryResponse } from '@/common/types/agentPlatform';
import { parseProviderId } from '@/common/types/ids';
import { useProvidersQuery } from './useModelProviderList';
import { AGENT_PRESET_LIBRARY_SWR_KEY, useAgentPresets } from './useAgentPresets';

test('conversation imports refresh every selector through one shared subscription', async () => {
  let notify: (() => void) | undefined;
  let unsubscribed = 0;
  const subscribe = spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation((callback) => {
    notify = () => callback({ provider_id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000001') });
    return () => { unsubscribed++; };
  });
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  const reload = spyOn(configService, 'reload').mockResolvedValue(undefined);
  const fetch = spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([{
    id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000001'),
    platform: 'custom', name: 'Imported provider', base_url: 'https://models.example/v1',
    auth_scheme: 'bearer', has_credentials: true, models: [],
  }]);
  function Selector() {
    const { data } = useProvidersQuery();
    return <span>{data?.length ?? 'loading'}</span>;
  }
  try {
    const page = render(<SWRConfig value={{ provider: () => new Map(), fallback: { providers: [] }, revalidateOnMount: false }}>
      <Selector /><Selector />
    </SWRConfig>);
    expect(subscribe).toHaveBeenCalledTimes(1);
    expect(fetch).toHaveBeenCalledTimes(0);
    await act(async () => { notify?.(); });
    await waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
    expect(reload).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(page.container.textContent).toBe('11'));
    page.unmount();
    expect(unsubscribed).toBe(1);
  } finally {
    cleanup();
    subscribe.mockRestore();
    reconnect.mockRestore();
    fetch.mockRestore();
    reload.mockRestore();
  }
});

test('provider changes refresh client preferences and the shared Agent library without writing defaults', async () => {
  let notify: (() => void) | undefined;
  const subscribe = spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation(callback => {
    notify = () => callback({ provider_id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000001') });
    return () => {};
  });
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  const providers = spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([]);
  const reload = spyOn(configService, 'reload').mockResolvedValue(undefined);
  const write = spyOn(configService, 'set');
  const empty: AgentPresetLibraryResponse = { official_templates: [], official_presets: {}, user_presets: [], active_bindings: [],
    fresh_start: { data_generation: 6, legacy_data_imported: false, official_template_count: 0, user_preset_count: 0 } };
  const library = spyOn(agentPlatform.library, 'invoke').mockResolvedValue({ ...empty, user_presets: [{
    preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000002'), display_name: 'Updated Agent', source: 'user', bound_target_count: 0,
  }] });
  function Observer() {
    useProvidersQuery();
    const { library } = useAgentPresets();
    return <output>{library?.user_presets.length}</output>;
  }
  try {
    const page = render(<SWRConfig value={{ provider: () => new Map(), revalidateOnMount: false,
      fallback: { providers: [], [AGENT_PRESET_LIBRARY_SWR_KEY]: empty } }}><Observer /></SWRConfig>);
    expect(page.container.textContent).toBe('0');
    expect(library).not.toHaveBeenCalled();
    await act(async () => { notify?.(); });
    await waitFor(() => expect(page.container.textContent).toBe('1'));
    expect(reload).toHaveBeenCalledTimes(1);
    expect(library).toHaveBeenCalledTimes(1);
    expect(providers).toHaveBeenCalledTimes(1);
    expect(write).not.toHaveBeenCalled();
  } finally {
    cleanup(); subscribe.mockRestore(); reconnect.mockRestore(); providers.mockRestore(); reload.mockRestore(); write.mockRestore(); library.mockRestore();
  }
});
