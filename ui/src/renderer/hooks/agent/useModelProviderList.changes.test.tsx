import '../../../../test/setup-dom.ts';
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { expect, spyOn, test } from 'bun:test';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { parseProviderId } from '@/common/types/ids';
import { useProvidersQuery } from './useModelProviderList';

test('conversation imports refresh every selector through one shared subscription', async () => {
  let notify: (() => void) | undefined;
  let unsubscribed = 0;
  const subscribe = spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation((callback) => {
    notify = () => callback({ provider_id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000001') });
    return () => { unsubscribed++; };
  });
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
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
    await waitFor(() => expect(page.container.textContent).toBe('11'));
    page.unmount();
    expect(unsubscribed).toBe(1);
  } finally {
    cleanup();
    subscribe.mockRestore();
    reconnect.mockRestore();
    fetch.mockRestore();
  }
});
