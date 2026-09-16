import '../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { Message } from '@arco-design/web-react';
import { useState, type ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import type { IProvider } from '@/common/config/storage';
import { parseAgentPresetId, parseConversationId, parseProviderId } from '@/common/types/ids';
import type { OfficialPresetTemplate } from '@/common/types/agentPlatform';
import type { GuidAgentSelection } from '@/renderer/pages/guid/types';
import { useGuidCreation } from './useGuidCreation';
import { prepareOfficialAgent } from '@/renderer/pages/guid/hooks/officialAgentLaunch';
import { emptyCreationDraft } from './useCreationDraft';
import { buildCreationRequest, creationAttempt, acknowledgeCreationAttempt } from './submission';
import { validateCreationTasks } from './client';
import { createQueuedCommandItem, normalizeQueueState } from '@/renderer/pages/conversation/platforms/useConversationCommandQueue';
import { creationParameterPolicy, creationVideoInputRoles } from './parameterPolicy';
import { initialGenerationModel } from './useGenerationModel';
import { recallCreationTask } from './recallTask';
import type { ConversationCreationTask, CreationMode } from './types';
import type { CreativeAsset } from '@/renderer/pages/creativeStudio/assets/types';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { creationDraftStorageKey } from './useCreationDraft';

const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000105');
const presetId = parseAgentPresetId('0190f5fe-7c00-7a00-8000-000000000104');
const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000102');
const messageId = '0190f5fe-7c00-7a00-8000-000000000106';
const model = { providerId, model: 'image-exact' };
const provider = { id: providerId, name: 'Provider', platform: 'openai', enabled: true, models: [{ model: model.model, enabled: true, capabilities: [{ task: 'image_generation', traits: [], protocol: 'openai.images' }, { task: 'image_edit', traits: [], protocol: 'openai.images' }] }] } as unknown as IProvider;
const template = { template_key: 'creative-studio.default', seed: { required_resource_kinds: ['asset_library'] } } as OfficialPresetTemplate;
const realFetch = globalThis.fetch;
beforeEach(() => setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000107'));
afterEach(() => { cleanup(); sessionStorage.clear(); globalThis.fetch = realFetch; });

function mount(generationProvider = provider) {
  const cache = new Map();
  const wrapper = ({ children }: { children: ReactNode }) => <MemoryRouter><SWRConfig value={{ provider: () => cache, fallback: { providers: [generationProvider] }, revalidateOnMount: false, shouldRetryOnError: false }}>{children}</SWRConfig></MemoryRouter>;
  return renderHook(() => {
    const [selection, setSelection] = useState<GuidAgentSelection>({ kind: 'template', templateKey: 'chat.minimal' });
    const [input, setInput] = useState('一只橘猫');
    const creation = useGuidCreation({ selection, setSelection, officialTemplates: [template], selectedPreset: undefined, selectedTemplate: template, presets: [], draftPresets: [], isLoading: false, isLoaded: true, loadError: undefined, refreshPresets: async () => {} }, input, [], '', () => setInput(''));
    return { creation, selection, input, setInput };
  }, { wrapper });
}

describe('conversation creation admission and draft behavior', () => {
  test('task selection switches to creative Agent and chat selects general assistant without clearing prompt, references or per-mode settings', async () => {
    const hook = mount();
    act(() => hook.result.current.creation.selectMode('image'));
    await waitFor(() => expect(hook.result.current.creation.ready).toBe(true));
    act(() => hook.result.current.creation.update(draft => ({ ...draft, parameters: { ...draft.parameters, image: { quality: 'high', count: 2 } }, references: [{ asset_id: 'asset-one', kind: 'image', role: 'reference', title: '参考' }] })));
    act(() => hook.result.current.creation.selectMode('video'));
    expect(hook.result.current.selection).toEqual({ kind: 'template', templateKey: 'creative-studio.default' });
    expect(hook.result.current.creation.draft.parameters.image).toEqual({ quality: 'high', count: 2 });
    act(() => hook.result.current.creation.exit());
    expect(hook.result.current.selection).toEqual({ kind: 'template', templateKey: 'assistant.general' });
    expect(hook.result.current.creation.draft.mode).toBeNull();
    expect(hook.result.current.creation.draft.references).toHaveLength(1);
    expect(hook.result.current.input).toBe('一只橘猫');
  });

  test.each([
    ['image', 'image_generation', 'openai.images', 't2i'],
    ['video', 'video_generation', 'openai.videos', 't2v'],
    ['music', 'music_generation', 'minimax.music', 'music'],
  ] as const)('%s binds the required asset library before admitting generation without a chat model', async (mode, task, protocol, capability) => {
    const calls: Array<{ url: string; body: Record<string, unknown> }> = [];
    globalThis.fetch = (async (url: RequestInfo | URL, init?: RequestInit) => {
      const path = String(url), body = init?.body ? JSON.parse(String(init.body)) : {};
      calls.push({ url: path, body });
      let data: unknown;
      if (path.endsWith('/from-template/creative-studio.default')) data = { preset: { preset_id: presetId, current_stable_revision: { revision: 1 } } };
      else if (path.endsWith('/api/agent-sessions')) {
        if (!Array.isArray(body.resource_selections) || !body.resource_selections.some((resource: { resource_kind: string; resource_id: string }) => resource.resource_kind === 'asset_library' && resource.resource_id === 'creative-studio-assets')) {
          return new Response(JSON.stringify({ success: false, error: 'the Agent requires additional product resources', code: 'RESOURCE_SELECTION_REQUIRED', details: { missing_resource_kinds: ['asset_library'] } }), { status: 422 });
        }
        data = { agent_session_id: conversationId };
      }
      else if (path.endsWith('/creation-tasks')) data = { message_id: messageId, tasks: [{ creation_task_id: 'task', owner: { kind: 'conversation_turn', conversation_id: conversationId, message_id: messageId }, status: 'queued', result_asset_ids: [] }] };
      else if (path.endsWith(`/api/conversations/${conversationId}`)) data = { conversation_id: conversationId, name: '一只橘猫', type: 'nomi', created_at: 1, modified_at: 2, extra: { workspace: '' } };
      else throw new Error(`Unexpected request ${path}`);
      return new Response(JSON.stringify({ success: true, data }), { headers: { 'Content-Type': 'application/json' } });
    }) as typeof fetch;
    await prepareOfficialAgent(template, '创意工坊', { id: providerId, use_model: 'previous-chat-model' });
    const hook = mount({ ...provider, models: [{ model: model.model, enabled: true, capabilities: [{ task, traits: [], protocol }] }] } as unknown as IProvider);
    act(() => hook.result.current.creation.selectMode(mode as CreationMode));
    await waitFor(() => expect(hook.result.current.creation.ready).toBe(true));
    await act(async () => { await hook.result.current.creation.send(); });
    expect(hook.result.current.input).toBe('');
    expect(hook.result.current.creation.draft.references).toEqual([]);
    expect(calls.find(call => call.url.endsWith('/api/agent-sessions'))?.body).toEqual({ preset_id: presetId, title: '一只橘猫', resource_selections: [{ resource_kind: 'asset_library', resource_id: 'creative-studio-assets' }] });
    const presetCalls = calls.filter(call => call.url.endsWith('/from-template/creative-studio.default'));
    expect(presetCalls).toHaveLength(2);
    for (const call of presetCalls) expect(call.body).not.toHaveProperty('model');
    expect(calls.find(call => call.url.endsWith('/creation-tasks'))?.body).toMatchObject({ preset_id: presetId, provider_id: providerId, model: 'image-exact', capability, params: { prompt: '一只橘猫' } });
    expect(calls.some(call => call.url.endsWith('/messages') || call.url.includes('switch-preset'))).toBe(false);
  });

  test('failed generation admission retains the editable prompt and references', async () => {
    const toast = spyOn(Message, 'error').mockReturnValue(() => {});
    try {
      globalThis.fetch = (async () => new Response(JSON.stringify({ success: false, error: 'Service unavailable' }), { status: 503 })) as typeof fetch;
      const hook = mount();
      act(() => hook.result.current.creation.selectMode('image'));
      await waitFor(() => expect(hook.result.current.creation.ready).toBe(true));
      act(() => hook.result.current.creation.update(draft => ({ ...draft, references: [{ asset_id: 'reference-one', kind: 'image', title: '参考图', role: 'reference' }] })));
      await act(async () => { await hook.result.current.creation.send(); });
      expect(hook.result.current.input).toBe('一只橘猫');
      expect(hook.result.current.creation.draft.references).toHaveLength(1);
      expect(hook.result.current.creation.draft.mode).toBe('image');
      expect(toast).toHaveBeenCalledTimes(1);
    } finally { toast.mockRestore(); }
  });

  test('freezes provider settings, ignores inactive music references, and reuses uncertain admission keys', () => {
    const draft = emptyCreationDraft();
    draft.mode = 'image'; draft.models.image = model;
    const request = buildCreationRequest(draft, 'draw', presetId, ['C:/authorized/reference.png']);
    expect(request.capability).toBe('i2i');
    expect(request.params).toMatchObject({ prompt: 'draw', count: 1 });
    const key = creationAttempt('test-admission', request);
    expect(creationAttempt('test-admission', structuredClone(request))).toBe(key);
    creationAttempt('test-admission', { ...request, params: { ...request.params, prompt: 'changed while uncertain' } });
    expect(creationAttempt('test-admission', request)).toBe(key);
    draft.parameters.image.quality = 'high';
    expect(request.params.quality).toBeUndefined();
    acknowledgeCreationAttempt('test-admission', key);
    expect(creationAttempt('test-admission', request)).not.toBe(key);
    draft.mode = 'music'; draft.models.music = model;
    draft.parameters.music = { instrumental: true, lyrics: 'stored lyrics' };
    const music = buildCreationRequest(draft, 'piano', presetId, ['C:/authorized/reference.png']);
    expect(music.files).toBeUndefined(); expect(music.inputs).toEqual([]); expect(music.params).not.toHaveProperty('lyrics'); expect(music.params).not.toHaveProperty('seconds');
    expect(draft.parameters.music.lyrics).toBe('stored lyrics');
  });

  test('speech synthesis cannot make an unconfigured music mode available', async () => {
    const speechProvider = { ...provider, models: [{ model: 'speech-only', enabled: true, capabilities: [{ task: 'speech_synthesis', traits: [], protocol: 'openai.audio_speech' }] }] } as unknown as IProvider;
    const hook = mount(speechProvider);
    act(() => hook.result.current.creation.selectMode('music'));
    await waitFor(() => expect(hook.result.current.creation.draft.mode).toBe('music'));
    expect(hook.result.current.creation.ready).toBe(false);
    expect(hook.result.current.creation.draft.models.music).toBeNull();
  });

  test('rejects foreign ownership and success without result assets', () => {
    const task = { creation_task_id: 'task', owner: { kind: 'conversation_turn', conversation_id: conversationId, message_id: messageId }, status: 'succeeded', result_asset_ids: [] };
    expect(() => validateCreationTasks([task], conversationId)).toThrow('no media');
    expect(() => validateCreationTasks([{ ...task, owner: { ...task.owner, conversation_id: 'foreign' } }], conversationId)).toThrow('does not belong');
  });

  test('queued ordinary messages retain their admitted Agent selection through storage normalization', () => {
    const item = createQueuedCommandItem({ input: 'Continue', files: [], preset_id: presetId });
    expect(normalizeQueueState({ items: [item], isPaused: false }).items[0].preset_id).toBe(presetId);
  });

  test('protocol policies do not submit OpenAI video duration or quality values to incompatible models', () => {
    const draft = emptyCreationDraft();
    draft.mode = 'video'; draft.models.video = { providerId, model: 'sora-2' };
    draft.parameters.video = { seconds: 5, size: '1280x720', resolution: '1080p', count: 2 };
    const sora = { providerId, model: 'sora-2', label: 'Sora', protocol: 'openai.videos' };
    expect(creationParameterPolicy(sora).video.seconds).toEqual([4, 8, 12]);
    const video = buildCreationRequest(draft, 'waves', presetId, [], sora);
    expect(video.params.seconds).toBeUndefined();
    expect(video.params.resolution).toBeUndefined();
    expect(video.params.size).toBe('1280x720');
    draft.mode = 'image'; draft.models.image = model; draft.parameters.image = { quality: 'high', count: 4 };
    const ark = buildCreationRequest(draft, 'cat', presetId, [], { ...model, label: 'Seedream', protocol: 'ark.images' });
    expect(ark.params.quality).toBeUndefined(); expect(ark.params.count).toBe(1);
    const dalle = buildCreationRequest(draft, 'cat', presetId, [], { ...model, model: 'dall-e-3', label: 'DALL E', protocol: 'openai.images' });
    expect(dalle.params.quality).toBeUndefined(); expect(dalle.params.count).toBe(1);
    expect(creationParameterPolicy({ ...model, model: 'dall-e-3', protocol: 'openai.images' }).qualities).toEqual(['standard', 'hd']);
    draft.parameters.image = { aspect: 'auto' };
    const automatic = buildCreationRequest(draft, 'cat', presetId, [], { ...model, model: 'gpt-image-1', label: 'GPT Image', protocol: 'openai.images' });
    expect(automatic.params.size).toBeUndefined();
    expect(automatic.params.width).toBeUndefined();
  });

  test('generation model recommendation uses an exact configured default or a sole candidate', () => {
    const other = { ...model, model: 'other-image' };
    expect(initialGenerationModel([model, other])).toBeUndefined();
    expect(initialGenerationModel([model, other], { provider_id: providerId, model: 'other-image' })).toBe(other);
    expect(initialGenerationModel([model, other], { provider_id: 'unavailable', model: 'other-image' })).toBeUndefined();
    expect(initialGenerationModel([model])).toBe(model);
  });

  test('draft and uncertain submission keys cannot attach to a restored backend dataset', () => {
    const draft = emptyCreationDraft(); draft.mode = 'image'; draft.models.image = model;
    const request = buildCreationRequest(draft, 'draw', presetId);
    const storageKey = creationDraftStorageKey('guid');
    const key = creationAttempt('restored-dataset', request);
    setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000108');
    expect(creationDraftStorageKey('guid')).not.toBe(storageKey);
    expect(creationAttempt('restored-dataset', request)).not.toBe(key);
  });

  test('only compatible image attachments select image-edit/video inputs; inactive originals remain in the draft', () => {
    const draft = emptyCreationDraft(); draft.mode = 'image'; draft.models.image = model;
    const files = ['C:/brief.pdf', 'C:/notes.txt', 'C:/song.mp3', 'C:/clip.mp4'];
    expect(buildCreationRequest(draft, 'draw', presetId, files).capability).toBe('t2i');
    const mixed = [...files, 'C:/reference.PNG'];
    const request = buildCreationRequest(draft, 'draw', presetId, mixed);
    expect(request.files).toEqual(['C:/reference.PNG']); expect(request.capability).toBe('i2i');
    expect(mixed).toHaveLength(5);
    draft.mode = 'video'; draft.models.video = model;
    draft.references = [{ asset_id: 'document', kind: 'text', role: 'reference', title: '文档' }, { asset_id: 'mask', kind: 'image', role: 'mask', title: '蒙版' }];
    const video = buildCreationRequest(draft, 'waves', presetId, files);
    expect(video.capability).toBe('t2v'); expect(video.inputs).toEqual([]); expect(video.files).toBeUndefined();
    expect(draft.references).toHaveLength(2);
  });

  test('video role options match supported protocols and incompatible recalled roles require adjustment', () => {
    const sora = { ...model, model: 'sora-2', protocol: 'openai.videos', label: 'Sora' };
    expect(creationVideoInputRoles(sora)).not.toContain('last_frame');
    expect(creationVideoInputRoles({ ...model, model: 'grok-imagine-video-1.5', protocol: 'xai.video_jobs' })).toContain('last_frame');
    const draft = emptyCreationDraft(); draft.mode = 'video'; draft.models.video = model;
    draft.references = [{ asset_id: 'last', kind: 'image', role: 'last_frame', title: '尾帧' }];
    expect(() => buildCreationRequest(draft, 'waves', presetId, [], sora)).toThrow('素材角色');
    expect(draft.references[0].role).toBe('last_frame');
    draft.references = [];
    expect(() => buildCreationRequest(draft, 'waves', presetId, ['one.png', 'two.png'], sora)).toThrow('一张');
  });

  test('failed task recall strips internal metadata and retains mask/frame semantics', () => {
    const task = { provider_id: providerId, model: 'image-exact', capability: 'inpaint', params: { prompt: 'fix face', size: '1024x1024', _nomifun_creation_agent: { snapshot: 'private' }, _nomifun_creation_batch: 3, api_key: 'never-copy' }, inputs: [{ asset_id: 'original', kind: 'image', role: 'reference' }, { asset_id: 'mask', kind: 'image', role: 'mask' }] } as unknown as ConversationCreationTask;
    const assets = ['original', 'mask'].map(id => ({ id, kind: 'image', title: id, originalUrl: `/media/${id}` } as CreativeAsset));
    const recalled = recallCreationTask(emptyCreationDraft(), task, assets, 'image');
    recalled.mode = 'image';
    const request = buildCreationRequest(recalled, recalled.pendingPrompt!, presetId);
    expect(request.capability).toBe('inpaint');
    expect(request.inputs.map(input => input.role)).toEqual(['reference', 'mask']);
    expect(Object.keys(request.params).some(key => key.startsWith('_nomifun') || key === 'api_key')).toBe(false);
    const video = { ...task, capability: 'i2v', inputs: [{ asset_id: 'original', kind: 'image', role: 'first_frame' }, { asset_id: 'mask', kind: 'image', role: 'last_frame' }] } as ConversationCreationTask;
    expect(recallCreationTask(emptyCreationDraft(), video, assets, 'video').references.map(ref => ref.role)).toEqual(['first_frame', 'last_frame']);
    expect(recallCreationTask(emptyCreationDraft(), task, assets, 'video', true).references.map(ref => ref.role)).toEqual(['first_frame']);
    expect(() => recallCreationTask(emptyCreationDraft(), { ...task, capability: 'tts' }, assets, 'music')).toThrow('不属于音乐');
  });
});
