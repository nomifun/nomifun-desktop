import '../../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ProbeProviderConnectionResponse } from '@/common/protocolBindings/ProbeProviderConnectionResponse';
import { aliasProtocolManifest, aliasProviderId } from '../../../../../test/fixtures/modelAliasEditor';
import { applyProviderAutoConfiguration } from './providerAutoConfiguration';
import { emptyCapabilityDraft, type ModelProtocolManifest } from './providerModelAdvanced';
import useProviderAutoConfiguration from './useProviderAutoConfiguration';

type Options = Parameters<typeof useProviderAutoConfiguration>[0];
const restorers: Array<() => void> = [];
afterEach(() => { cleanup(); restorers.splice(0).forEach(restore => restore()); });
const pause = async () => act(async () => { await new Promise(resolve => setTimeout(resolve, 550)); });
const manifest = (task: ModelTask = 'chat'): ModelProtocolManifest => ({
  ...aliasProtocolManifest, platform: 'custom', preset: 'custom', requested_task: task, tasks: [task],
  protocols: ['openai.chat_text', 'anthropic.messages'].map((protocol_id, index) => ({
    ...aliasProtocolManifest.protocols[0], protocol_id, supported_tasks: [task],
    allowed_auth_schemes: index === 0 ? ['bearer'] : ['header_key:x-api-key'],
    endpoints: [{ ...aliasProtocolManifest.protocols[0].endpoints[0], task }],
  })),
});
const options = (): Options => ({
  platform: 'custom', baseUrl: 'https://vendor.example', authScheme: 'bearer',
  credentials: { api_keys: ['key-a'] }, definition: { model: '', capabilities: [emptyCapabilityDraft('chat')] },
  manifests: { chat: manifest() },
});
const response = (protocol: string, overrides: Partial<ProbeProviderConnectionResponse> = {}): ProbeProviderConnectionResponse => ({
  protocol, task: 'chat', reachability: 'reachable', root_shape: 'versioned_root',
  attempted_url: 'https://vendor.example/chat', elapsed_ms: 1, ...overrides,
});
const mockProbe = () => {
  const probe = spyOn(ipcBridge.mode.probeConnection, 'invoke').mockImplementation(async request => response(request.protocol!));
  restorers.push(() => probe.mockRestore());
  return probe;
};

describe('form-local provider interface discovery', () => {
  test('probes once without a model and reuses every candidate when typing, applying, or reordering recommendations', async () => {
    const probe = mockProbe();
    const initial = options();
    const hook = renderHook(props => useProviderAutoConfiguration(props), { initialProps: initial });
    await waitFor(() => expect(hook.result.current.data?.candidates).toHaveLength(2));
    expect(probe).toHaveBeenCalledTimes(2);
    expect(probe.mock.calls.every(([request]) => !Object.hasOwn(request, 'model'))).toBe(true);
    expect(hook.result.current.data?.candidates.every(candidate => candidate.confidence === 'reachable')).toBe(true);
    const applied = applyProviderAutoConfiguration(initial.definition, hook.result.current.data!.detections);
    hook.rerender({ ...initial, definition: { ...applied, model: 'one' } });
    hook.rerender({ ...initial, definition: { ...applied, model: 'another' } });
    const reversed = { ...manifest(), recommendation: { ...manifest().recommendation!, protocol_id: 'anthropic.messages' } };
    hook.rerender({ ...initial, definition: { ...initial.definition, model: 'another' }, manifests: { chat: reversed } });
    expect(hook.result.current.data?.detections[0].protocol).toBe('anthropic.messages');
    await pause();
    expect(probe).toHaveBeenCalledTimes(2);
  });

  test('retains all results and reuses a non-default candidate root and auth after a manual choice', async () => {
    const probe = mockProbe();
    probe.mockImplementation(async request => response(request.protocol!, request.protocol === 'anthropic.messages' ? {
      reachability: 'unreachable', suggested_base_url: 'https://vendor.example/claude',
      candidates: [{ base_url: 'https://vendor.example/claude', attempted_url: 'https://vendor.example/claude/messages', reachability: 'reachable' }],
    } : {}));
    const initial = options();
    const hook = renderHook(props => useProviderAutoConfiguration(props), { initialProps: initial });
    await waitFor(() => expect(hook.result.current.data?.candidates).toHaveLength(2));
    const chosen = hook.result.current.data!.candidates.find(candidate => candidate.protocol === 'anthropic.messages')!;
    expect(chosen).toMatchObject({ confidence: 'reachable', authScheme: 'header_key:x-api-key', suggestedBaseUrl: 'https://vendor.example/claude' });
    hook.rerender({ ...initial, baseUrl: chosen.suggestedBaseUrl!, authScheme: chosen.authScheme, definition: {
      model: 'selected-model', capabilities: [{ ...emptyCapabilityDraft('chat'), protocol: chosen.protocol, transportSource: 'user', protocolSelectionSource: 'manual' }],
    } });
    expect(hook.result.current.isLoading).toBe(false);
    expect(hook.result.current.data?.candidates).toHaveLength(2);
    await pause();
    expect(probe).toHaveBeenCalledTimes(2);
  });

  test('keeps rejected and unreachable candidates visible alongside the automatically selected result', async () => {
    const probe = mockProbe();
    probe.mockImplementation(async request => response(request.protocol!, {
      reachability: request.protocol === 'openai.chat_text' ? 'credentials_rejected' : 'unreachable',
    }));
    const hook = renderHook(() => useProviderAutoConfiguration(options()));
    await waitFor(() => expect(hook.result.current.data?.candidates).toHaveLength(2));
    expect(hook.result.current.data?.candidates.map(candidate => candidate.confidence)).toEqual(['credentials_rejected', 'fallback']);
    expect(hook.result.current.data?.detections[0].protocol).toBe('openai.chat_text');
  });

  test('only probes a newly added task and survives a temporarily loading manifest', async () => {
    const probe = mockProbe();
    const initial = options();
    const hook = renderHook(props => useProviderAutoConfiguration(props), { initialProps: initial });
    await waitFor(() => expect(hook.result.current.data).toBeDefined());
    hook.rerender({ ...initial, manifests: {}, loadingTasks: ['chat'] });
    hook.rerender(initial);
    expect(hook.result.current.data?.candidates).toHaveLength(2);
    await pause();
    expect(probe).toHaveBeenCalledTimes(2);
    hook.rerender({ ...initial, definition: { ...initial.definition, capabilities: [
      ...initial.definition.capabilities, emptyCapabilityDraft('image_generation'),
    ] }, manifests: { ...initial.manifests, image_generation: manifest('image_generation') } });
    await waitFor(() => expect(hook.result.current.data?.detections).toHaveLength(2));
    expect(probe).toHaveBeenCalledTimes(4);
    expect(probe.mock.calls.slice(2).every(([request]) => request.task === 'image_generation')).toBe(true);
  });

  test('a model manifest refresh preserves the same pending interface requests', async () => {
    const probe = mockProbe();
    const finish: Array<(value: ProbeProviderConnectionResponse) => void> = [];
    probe.mockImplementation(() => new Promise(resolve => finish.push(resolve)));
    const initial = options();
    const hook = renderHook(props => useProviderAutoConfiguration(props), { initialProps: initial });
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(2));
    const signal = probe.mock.calls[0][1]!;
    hook.rerender({ ...initial, definition: { ...initial.definition, model: 'another' }, manifests: {}, loadingTasks: ['chat'] });
    expect(signal.aborted).toBe(false);
    await pause();
    expect(probe).toHaveBeenCalledTimes(2);
    hook.rerender({ ...initial, definition: { ...initial.definition, model: 'another' } });
    expect(signal.aborted).toBe(false);
    await act(async () => { finish[0](response('openai.chat_text')); finish[1](response('anthropic.messages')); });
    await waitFor(() => expect(hook.result.current.data?.candidates).toHaveLength(2));
    expect(probe).toHaveBeenCalledTimes(2);
  });

  test('keeps each task candidate tied to its tested root when new tasks join reused results', async () => {
    const probe = mockProbe();
    const initial = options();
    const correctedRoot = 'https://vendor.example/v1';
    probe.mockImplementation(async request => response(request.protocol!, request.task === 'chat' ? {
      reachability: 'unreachable', suggested_base_url: correctedRoot,
      candidates: [{ base_url: correctedRoot, attempted_url: `${correctedRoot}/chat`, reachability: 'reachable' }],
    } : { task: 'image_generation' }));
    const hook = renderHook(props => useProviderAutoConfiguration(props), { initialProps: initial });
    await waitFor(() => expect(hook.result.current.data).toBeDefined());
    const corrected = { ...initial, baseUrl: correctedRoot };
    hook.rerender(corrected);
    expect(hook.result.current.isLoading).toBe(false);
    hook.rerender({ ...corrected, definition: { model: '', capabilities: [
      ...initial.definition.capabilities, emptyCapabilityDraft('image_generation'),
    ] }, manifests: { ...initial.manifests, image_generation: manifest('image_generation') } });
    await waitFor(() => expect(hook.result.current.data?.detections).toHaveLength(2));
    expect(probe).toHaveBeenCalledTimes(4);
    expect(hook.result.current.data?.candidates.filter(candidate => candidate.task === 'chat')
      .every(candidate => candidate.requestBaseUrl === initial.baseUrl)).toBe(true);
    expect(hook.result.current.data?.candidates.filter(candidate => candidate.task === 'image_generation')
      .every(candidate => candidate.requestBaseUrl === correctedRoot && candidate.suggestedBaseUrl === undefined)).toBe(true);
    expect(hook.result.current.data?.detections.find(candidate => candidate.task === 'image_generation')?.requestBaseUrl).toBe(correctedRoot);
  });

  test('marks connection and credential changes stale and only rechecks on request in each form', async () => {
    const probe = mockProbe();
    const initial = options();
    const hook = renderHook(props => useProviderAutoConfiguration(props), { initialProps: initial });
    await waitFor(() => expect(hook.result.current.data).toBeDefined());
    let changed = { ...initial, baseUrl: 'https://another.example' };
    hook.rerender(changed);
    expect(hook.result.current.stale).toBe(true);
    expect(hook.result.current.data?.detections).toEqual([]);
    expect(hook.result.current.data?.candidates).toHaveLength(2);
    expect(hook.result.current.isLoading).toBe(false);
    await pause();
    expect(probe).toHaveBeenCalledTimes(2);
    act(() => hook.result.current.recheck());
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(4));
    await waitFor(() => expect(hook.result.current.stale).toBe(false));
    changed = { ...changed, authScheme: 'query_key:token' };
    hook.rerender(changed);
    await pause();
    expect(probe).toHaveBeenCalledTimes(4);
    expect(hook.result.current.stale).toBe(true);
    act(() => hook.result.current.recheck());
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(6));
    await waitFor(() => expect(hook.result.current.stale).toBe(false));
    hook.rerender({ ...changed, credentials: { api_keys: ['key-b'] } });
    await pause();
    expect(probe).toHaveBeenCalledTimes(6);
    expect(hook.result.current.stale).toBe(true);
    act(() => hook.result.current.recheck());
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(8));
    const second = renderHook(() => useProviderAutoConfiguration({ ...initial, credentials: { api_keys: ['key-c'] } }));
    await waitFor(() => expect(second.result.current.data).toBeDefined());
    expect(probe).toHaveBeenCalledTimes(10);
    expect(probe.mock.calls.slice(-2).map(([request]) => request.credentials)).toEqual([
      { api_keys: ['key-c'] }, { api_keys: ['key-c'] },
    ]);
  });

  test('aborts replaced and unmounted checks and ignores late results from an old connection', async () => {
    const probe = mockProbe();
    const finish: Array<(value: ProbeProviderConnectionResponse) => void> = [];
    probe.mockImplementation(() => new Promise(resolve => finish.push(resolve)));
    const initial = options();
    const hook = renderHook(props => useProviderAutoConfiguration(props), { initialProps: initial });
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(2));
    const oldSignal = probe.mock.calls[0][1]!;
    hook.rerender({ ...initial, baseUrl: 'https://new.example' });
    expect(oldSignal.aborted).toBe(true);
    await act(async () => { finish[0](response('openai.chat_text', { suggested_base_url: 'https://stale.example' })); finish[1](response('anthropic.messages')); });
    expect(hook.result.current.data?.detections).toEqual([]);
    expect(hook.result.current.stale).toBe(true);
    expect(hook.result.current.isLoading).toBe(false);
    await pause();
    expect(probe).toHaveBeenCalledTimes(2);
    act(() => hook.result.current.recheck());
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(4));
    await act(async () => { finish[2](response('openai.chat_text')); finish[3](response('anthropic.messages')); });
    await waitFor(() => expect(hook.result.current.data?.requestBaseUrl).toBe('https://new.example'));
    hook.rerender({ ...initial, baseUrl: 'https://last.example' });
    await pause();
    expect(probe).toHaveBeenCalledTimes(4);
    act(() => hook.result.current.recheck());
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(6));
    const lastSignal = probe.mock.calls[4][1]!;
    hook.unmount();
    expect(lastSignal.aborted).toBe(true);
  });

  test('stored provider checks omit the model and a reopened form starts a fresh check', async () => {
    const probe = spyOn(ipcBridge.mode.probeProviderConnection, 'invoke').mockImplementation(async request => response(request.protocol!));
    restorers.push(() => probe.mockRestore());
    const initial = { ...options(), providerId: aliasProviderId, credentials: undefined, enabled: true };
    const hook = renderHook(props => useProviderAutoConfiguration(props), { initialProps: initial });
    await waitFor(() => expect(hook.result.current.data).toBeDefined());
    expect(probe).toHaveBeenCalledTimes(1);
    expect(probe.mock.calls[0][0]).not.toHaveProperty('model');
    expect(probe.mock.calls[0][1]).toBeInstanceOf(AbortSignal);
    hook.rerender({ ...initial, enabled: false });
    hook.rerender(initial);
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(2));
  });

  test('an initially manual task does not start interface discovery', async () => {
    const probe = mockProbe();
    const initial = options();
    renderHook(() => useProviderAutoConfiguration({ ...initial, definition: { model: 'manual', capabilities: [
      { ...emptyCapabilityDraft('chat'), protocol: 'openai.chat_text', transportSource: 'user' },
    ] } }));
    await pause();
    expect(probe).not.toHaveBeenCalled();
  });

  test('initial debounce edits use the current connection without consuming its automatic attempt', async () => {
    const probe = mockProbe();
    const initial = options();
    const hook = renderHook(props => useProviderAutoConfiguration(props), { initialProps: initial });
    hook.rerender({ ...initial, baseUrl: 'https://latest.example', credentials: { api_keys: ['latest-key'] } });
    await waitFor(() => expect(hook.result.current.data?.candidates).toHaveLength(2));
    expect(probe).toHaveBeenCalledTimes(2);
    expect(probe.mock.calls.map(([request]) => [request.base_url, request.credentials])).toEqual([
      ['https://latest.example', { api_keys: ['latest-key'] }], ['https://latest.example', { api_keys: ['latest-key'] }],
    ]);
    expect(hook.result.current.stale).toBe(false);
  });

  test('model limits and parameters remain editable without starting another interface check', async () => {
    const probe = mockProbe();
    const initial = options();
    const hook = renderHook(props => useProviderAutoConfiguration(props), { initialProps: initial });
    await waitFor(() => expect(hook.result.current.data).toBeDefined());
    hook.rerender({ ...initial, definition: { model: 'another-model', capabilities: [{
      ...initial.definition.capabilities[0], outputLimit: 1234, contextLimit: 5678,
      providerParamsJson: '{"temperature":0.2}',
    }] } });
    await pause();
    expect(probe).toHaveBeenCalledTimes(2);
    expect(hook.result.current.stale).toBe(false);
    expect(hook.result.current.isLoading).toBe(false);
  });

  test('a changed candidate set becomes stale and removing then restoring a task cannot grant another automatic attempt', async () => {
    const probe = mockProbe();
    const initial = options();
    const hook = renderHook(props => useProviderAutoConfiguration(props), { initialProps: initial });
    await waitFor(() => expect(hook.result.current.data).toBeDefined());
    const changed = { ...initial, protocolPreferences: { chat: 'anthropic.messages' } };
    hook.rerender(changed);
    expect(hook.result.current.stale).toBe(true);
    expect(hook.result.current.data?.detections).toEqual([]);
    await pause();
    expect(probe).toHaveBeenCalledTimes(2);
    hook.rerender({ ...changed, definition: { model: '', capabilities: [] } });
    hook.rerender(changed);
    await pause();
    expect(probe).toHaveBeenCalledTimes(2);
    act(() => hook.result.current.recheck());
    await waitFor(() => expect(hook.result.current.stale).toBe(false));
    expect(probe).toHaveBeenCalledTimes(3);
    expect(probe.mock.calls[2][0].protocol).toBe('anthropic.messages');
  });
});
