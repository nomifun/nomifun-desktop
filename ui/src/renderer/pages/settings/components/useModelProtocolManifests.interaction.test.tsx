import '../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'bun:test';
import { useState } from 'react';
import { SWRConfig } from 'swr';
import { aliasProtocolManifest } from '../../../../../test/fixtures/modelAliasEditor';
import useModelProtocolManifests from './useModelProtocolManifests';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
let requests: URL[] = [];
let resolveModel: Record<string, (response: Response) => void> = {};
const response = (model: string) => new Response(JSON.stringify({ success: true, data: {
  ...aliasProtocolManifest, model_defaults: { model, protocol_id: 'openai.chat_text',
    provider_params: {}, reasoning_efforts: [], sources: [], checked_at: '2026-10-15' },
} }), { headers: { 'Content-Type': 'application/json' } });

beforeEach(() => {
  requests = []; resolveModel = {};
  const fetcher = (async (input: string | URL | Request) => {
    const url = new URL(String(input), 'http://127.0.0.1');
    if (url.pathname !== '/api/model-protocols') return new Response('{"success":true,"data":{}}');
    requests.push(url);
    return await new Promise<Response>(resolve => { resolveModel[url.searchParams.get('model') ?? ''] = resolve; });
  }) as typeof fetch;
  globalThis.fetch = fetcher; window.fetch = fetcher;
});
afterEach(() => { cleanup(); globalThis.fetch = originalFetch; window.fetch = originalWindowFetch; });
const tasks = ['chat'] as const;
function Harness() {
  const [model, setModel] = useState('vendor/a');
  const state = useModelProtocolManifests({ preset: 'custom', tasks, baseUrlHint: 'https://vendor.example/v1', modelHint: model });
  return <><input aria-label='model' value={model} onChange={event => setModel(event.target.value)} />
    <output data-testid='manifest-model'>{state.manifests.chat?.model_defaults?.model ?? ''}</output>
    <output data-testid='loading'>{state.loadingTasks.join(',')}</output></>;
}

test('sends each exact model hint and hides both previous and late responses after model changes', async () => {
  const page = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><Harness /></SWRConfig>);
  await waitFor(() => expect(resolveModel['vendor/a']).toBeDefined());
  await act(async () => resolveModel['vendor/a'](response('vendor/a')));
  await waitFor(() => expect(page.getByTestId('manifest-model').textContent).toBe('vendor/a'));
  fireEvent.change(page.getByLabelText('model'), { target: { value: 'vendor/b' } });
  expect(page.getByTestId('manifest-model').textContent).toBe('');
  expect(page.getByTestId('loading').textContent).toBe('chat');
  await waitFor(() => expect(resolveModel['vendor/b']).toBeDefined());
  fireEvent.change(page.getByLabelText('model'), { target: { value: 'vendor/c' } });
  await waitFor(() => expect(resolveModel['vendor/c']).toBeDefined());
  await act(async () => resolveModel['vendor/b'](response('vendor/b')));
  expect(page.getByTestId('manifest-model').textContent).toBe('');
  await act(async () => resolveModel['vendor/c'](response('vendor/c')));
  await waitFor(() => expect(page.getByTestId('manifest-model').textContent).toBe('vendor/c'));
  expect(requests.map(url => url.searchParams.get('model'))).toEqual(['vendor/a', 'vendor/b', 'vendor/c']);
  expect(requests.every(url => url.searchParams.get('base_url') === 'https://vendor.example/v1')).toBe(true);
});
