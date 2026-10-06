import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import type { ReactNode } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { FetchModelsResponse } from '@/common/protocolBindings/FetchModelsResponse';
import { parseProviderId } from '@/common/types/ids';
import useModeModeList from './useModeModeList';

afterEach(cleanup);

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en', resources: {} });

const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000001');
const otherProviderId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000002');

const catalog = (...ids: string[]): FetchModelsResponse => ({
  models: ids.map((id) => ({ id, name: null, tasks: [], traits: [] })),
});

const createWrapper = (cache = new Map()) => {
  return ({ children }: { children: ReactNode }) => (
    <I18nextProvider i18n={i18n}>
      <SWRConfig value={{ provider: () => cache, dedupingInterval: 0, revalidateOnFocus: false }}>
        {children}
      </SWRConfig>
    </I18nextProvider>
  );
};

test('refresh displays newly returned IDs even when the catalog declares no tasks or traits', async () => {
  const fetch = spyOn(ipcBridge.mode.fetchProviderModels, 'invoke')
    .mockResolvedValueOnce(catalog('old-model'))
    .mockResolvedValueOnce(catalog('old-model', 'future-unknown-model'));
  try {
    const hook = renderHook(() => useModeModeList({ platform: 'stepfun', providerId }), { wrapper: createWrapper() });
    await waitFor(() => expect(hook.result.current.data?.models.map((model) => model.value)).toEqual(['old-model']));
    await act(async () => { await hook.result.current.mutate(); });
    expect(hook.result.current.data?.models).toEqual([
      { value: 'old-model', label: 'old-model', tasks: [], traits: [] },
      { value: 'future-unknown-model', label: 'future-unknown-model', tasks: [], traits: [] },
    ]);
    expect(fetch).toHaveBeenCalledTimes(2);
  } finally {
    fetch.mockRestore();
  }
});

test('a late catalog response from the previous provider does not replace the current list', async () => {
  let resolvePrevious!: (value: FetchModelsResponse) => void;
  const previous = new Promise<FetchModelsResponse>((resolve) => { resolvePrevious = resolve; });
  const fetch = spyOn(ipcBridge.mode.fetchProviderModels, 'invoke').mockImplementation(async (request) =>
    request.provider_id === providerId ? previous : catalog('current-future-model')
  );
  try {
    const hook = renderHook(({ id }) => useModeModeList({ platform: 'custom', providerId: id }), {
      wrapper: createWrapper(), initialProps: { id: providerId },
    });
    await waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
    hook.rerender({ id: otherProviderId });
    await waitFor(() => expect(hook.result.current.data?.models[0]?.value).toBe('current-future-model'));
    await act(async () => { resolvePrevious(catalog('previous-model')); await previous; });
    expect(hook.result.current.data?.models.map((model) => model.value)).toEqual(['current-future-model']);
  } finally {
    fetch.mockRestore();
  }
});

test('refreshing an existing catalog reports validation progress and keeps its options on rejection', async () => {
  let rejectRefresh!: (error: Error) => void;
  const refreshing = new Promise<FetchModelsResponse>((_, reject) => { rejectRefresh = reject; });
  const fetch = spyOn(ipcBridge.mode.fetchProviderModels, 'invoke')
    .mockResolvedValueOnce(catalog('cached-model'))
    .mockImplementationOnce(() => refreshing)
    .mockResolvedValueOnce(catalog('cached-model', 'latest-model'));
  try {
    const hook = renderHook(() => useModeModeList({ platform: 'stepfun', providerId }), { wrapper: createWrapper() });
    await waitFor(() => expect(hook.result.current.data?.models[0]?.value).toBe('cached-model'));
    let refresh!: ReturnType<typeof hook.result.current.mutate>;
    act(() => { refresh = hook.result.current.mutate(); });
    await waitFor(() => expect(hook.result.current.isValidating).toBe(true));
    expect(hook.result.current.isLoading).toBe(false);
    expect(hook.result.current.data?.models[0]?.value).toBe('cached-model');
    await act(async () => {
      rejectRefresh(new BackendHttpError({
        method: 'POST', path: '/api/providers/models/fetch', status: 400,
        body: { code: 'BAD_REQUEST', error: 'Remote API rejected the model-list request (400 Bad Request)' },
      }));
      await refresh;
    });
    expect(hook.result.current.isValidating).toBe(false);
    expect(hook.result.current.error?.message).toBe('settings.modelCatalogBadRequest');
    expect(hook.result.current.data?.models.map((model) => model.value)).toEqual(['cached-model']);
    await act(async () => { await hook.result.current.mutate(); });
    expect(hook.result.current.error).toBeUndefined();
    expect(hook.result.current.data?.models.map((model) => model.value)).toEqual(['cached-model', 'latest-model']);
    expect(fetch).toHaveBeenCalledTimes(3);
  } finally {
    fetch.mockRestore();
  }
});

test('a successful empty catalog is distinguishable from a fetch failure', async () => {
  const fetch = spyOn(ipcBridge.mode.fetchProviderModels, 'invoke').mockResolvedValue(catalog());
  try {
    const hook = renderHook(() => useModeModeList({ platform: 'custom', providerId }), { wrapper: createWrapper() });
    await waitFor(() => expect(hook.result.current.data?.models).toEqual([]));
    expect(hook.result.current.canFetch).toBe(true);
    expect(hook.result.current.error).toBeUndefined();
    expect(hook.result.current.isValidating).toBe(false);
  } finally {
    fetch.mockRestore();
  }
});

test('catalog metadata preserves whether models came from a remote list or official documentation', async () => {
  const fetch = spyOn(ipcBridge.mode.fetchProviderModels, 'invoke')
    .mockResolvedValueOnce({ ...catalog('documented-model'), catalog_source: 'official_documentation' })
    .mockResolvedValueOnce({ ...catalog('live-model'), catalog_source: 'remote' })
    .mockResolvedValueOnce(catalog('unspecified-source-model'));
  try {
    const hook = renderHook(() => useModeModeList({ platform: 'coding-plan', providerId }), { wrapper: createWrapper() });
    await waitFor(() => expect(hook.result.current.data?.catalogSource).toBe('official_documentation'));
    expect(hook.result.current.data?.models[0]?.value).toBe('documented-model');
    await act(async () => { await hook.result.current.mutate(); });
    expect(hook.result.current.data?.catalogSource).toBe('remote');
    expect(hook.result.current.data?.models[0]?.value).toBe('live-model');
    await act(async () => { await hook.result.current.mutate(); });
    expect(hook.result.current.data?.catalogSource).toBeUndefined();
    expect(hook.result.current.data?.models[0]?.value).toBe('unspecified-source-model');
  } finally {
    fetch.mockRestore();
  }
});

test('anonymous catalog automatically loads when valid credentials become available', async () => {
  const fetch = spyOn(ipcBridge.mode.fetchModelList, 'invoke').mockResolvedValue(catalog('official-model'));
  try {
    const hook = renderHook(({ credentials }) => useModeModeList({
      platform: 'stepfun', baseUrl: 'https://api.stepfun.com/v1', authScheme: 'bearer', credentials,
    }), { wrapper: createWrapper(), initialProps: { credentials: undefined as Record<string, unknown> | undefined } });
    expect(hook.result.current.canFetch).toBe(true);
    expect(hook.result.current.isValidating).toBe(false);
    expect(fetch).not.toHaveBeenCalled();
    hook.rerender({ credentials: { api_keys: ['test-key'] } });
    await waitFor(() => expect(hook.result.current.data?.models[0]?.value).toBe('official-model'));
    expect(hook.result.current.canFetch).toBe(true);
    expect(fetch).toHaveBeenCalledTimes(1);
  } finally {
    fetch.mockRestore();
  }
});

test('anonymous discovery scopes catalogs to the credential-bearing hook instance in a shared cache', async () => {
  const cache = new Map();
  const firstCredentials = { api_keys: ['first-account-secret'] };
  const secondCredentials = { api_keys: ['second-account-secret'] };
  const fetch = spyOn(ipcBridge.mode.fetchModelList, 'invoke').mockImplementation(async (request) =>
    catalog(request.credentials === firstCredentials ? 'first-account-future-model' : 'second-account-future-model')
  );
  try {
    const hook = renderHook(() => ({
      first: useModeModeList({
        platform: 'custom', baseUrl: 'https://api.example/v1', authScheme: 'bearer', credentials: firstCredentials,
      }),
      second: useModeModeList({
        platform: 'custom', baseUrl: 'https://api.example/v1', authScheme: 'bearer', credentials: secondCredentials,
      }),
    }), { wrapper: createWrapper(cache) });
    await waitFor(() => {
      expect(hook.result.current.first.data?.models[0]?.value).toBe('first-account-future-model');
      expect(hook.result.current.second.data?.models[0]?.value).toBe('second-account-future-model');
    });
    expect(fetch).toHaveBeenCalledTimes(2);
    const cacheKeys = [...cache.keys()].join(' ');
    expect(cacheKeys).not.toContain('first-account-secret');
    expect(cacheKeys).not.toContain('second-account-secret');
    expect(cacheKeys).not.toContain('api_keys');
  } finally {
    fetch.mockRestore();
  }
});

test('a configured HTTP catalog can be explicitly requested without credentials', async () => {
  const fetch = spyOn(ipcBridge.mode.fetchModelList, 'invoke')
    .mockResolvedValue({ ...catalog('public-model'), catalog_source: 'remote' });
  try {
    const hook = renderHook(() => useModeModeList({
      platform: 'openrouter', baseUrl: 'https://openrouter.ai/api/v1', authScheme: 'bearer',
    }), { wrapper: createWrapper() });
    expect(hook.result.current.canFetch).toBe(true);
    expect(hook.result.current.isValidating).toBe(false);
    expect(fetch).not.toHaveBeenCalled();
    await act(async () => { await hook.result.current.mutate(); });
    expect(fetch).toHaveBeenCalledWith({
      platform: 'openrouter', base_url: 'https://openrouter.ai/api/v1', auth_scheme: 'bearer',
      credentials: {}, bedrock_config: undefined, try_fix: undefined,
    });
    expect(hook.result.current.data?.models[0]?.value).toBe('public-model');
    expect(hook.result.current.data?.catalogSource).toBe('remote');
  } finally {
    fetch.mockRestore();
  }
});

test('anonymous HTTP discovery remains unavailable when auth scheme or base URL is missing', async () => {
  const fetch = spyOn(ipcBridge.mode.fetchModelList, 'invoke').mockResolvedValue(catalog());
  try {
    const hook = renderHook(({ baseUrl, authScheme }) => useModeModeList({ platform: 'custom', baseUrl, authScheme }), {
      wrapper: createWrapper(), initialProps: { baseUrl: '', authScheme: '' },
    });
    expect(hook.result.current.canFetch).toBe(false);
    await act(async () => { await hook.result.current.mutate(); });
    hook.rerender({ baseUrl: 'https://api.example/v1', authScheme: '' });
    expect(hook.result.current.canFetch).toBe(false);
    await act(async () => { await hook.result.current.mutate(); });
    hook.rerender({ baseUrl: '', authScheme: 'bearer' });
    expect(hook.result.current.canFetch).toBe(false);
    await act(async () => { await hook.result.current.mutate(); });
    expect(fetch).not.toHaveBeenCalled();
  } finally {
    fetch.mockRestore();
  }
});

test('Bedrock SDK discovery requires an explicit credential configuration', async () => {
  const fetch = spyOn(ipcBridge.mode.fetchModelList, 'invoke').mockResolvedValue(catalog('bedrock-model'));
  try {
    const hook = renderHook(({ credentials }) => useModeModeList({
      platform: 'bedrock', baseUrl: 'https://bedrock.us-east-1.amazonaws.com', authScheme: 'bedrock', credentials,
      bedrockConfig: { auth_method: 'defaultChain', region: 'us-east-1' },
    }), { wrapper: createWrapper(), initialProps: { credentials: undefined as Record<string, unknown> | undefined } });
    expect(hook.result.current.canFetch).toBe(false);
    await act(async () => { await hook.result.current.mutate(); });
    expect(fetch).not.toHaveBeenCalled();
    hook.rerender({ credentials: {} });
    await waitFor(() => expect(hook.result.current.data?.models[0]?.value).toBe('bedrock-model'));
    expect(hook.result.current.canFetch).toBe(true);
  } finally {
    fetch.mockRestore();
  }
});

test('local discovery validation errors are not described as remote HTTP rejection', async () => {
  const fetch = spyOn(ipcBridge.mode.fetchModelList, 'invoke').mockRejectedValue(new BackendHttpError({
    method: 'POST', path: '/api/providers/models/fetch', status: 400,
    body: { code: 'BAD_REQUEST', error: 'api_keys required for bearer authentication' },
  }));
  try {
    const hook = renderHook(() => useModeModeList({
      platform: 'custom', baseUrl: 'https://api.example/v1', authScheme: 'bearer',
    }), { wrapper: createWrapper() });
    await act(async () => { await hook.result.current.mutate(); });
    expect(hook.result.current.error?.message).toBe('settings.modelCatalogInvalidConfiguration');
    expect(hook.result.current.data).toBeUndefined();
  } finally {
    fetch.mockRestore();
  }
});
