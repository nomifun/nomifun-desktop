import '../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { useState } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { parseAgentPresetId, parseProviderId } from '@/common/types/ids';
import type { IProvider } from '@/common/config/storage';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import type { GuidAgentSelection } from '@/renderer/pages/guid/types';
import { useGuidCreation } from './useGuidCreation';
import { creationDraftStorageKey } from './useCreationDraft';
import { creativeAssetClient } from '@/renderer/pages/creativeStudio/assets/client';
import { CreationComposerContext } from './CreationComposerContext';
import CreationControls, { CreationModelSelector } from './CreationControls';
import ComposerSceneSelector from './ComposerSceneSelector';
import creationMessages from '@/renderer/services/i18n/locales/zh-CN/creation.json';
import { emptyCreationDraft } from './useCreationDraft';
import { buildCreationRequest } from './submission';
import type { CreationDraft } from './types';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { creation: creationMessages } } } });
let assetList: ReturnType<typeof spyOn<typeof creativeAssetClient, 'list'>>;
beforeEach(() => {
  setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000105');
  assetList = spyOn(creativeAssetClient, 'list').mockResolvedValue({ items: [], total: 0 });
});
afterEach(() => { cleanup(); assetList.mockRestore(); });
const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000105');
const presetId = parseAgentPresetId('0190f5fe-7c00-7a00-8000-000000000104');

test.each(['image', 'video'] as const)('%s quantity offers at most four and submits a capped count', async mode => {
  const protocol = mode === 'video' ? 'agnes.video_jobs' : 'openai.images';
  const model = mode === 'video' ? 'agnes-video-v2.0' : 'gpt-image-1';
  const provider = { id: providerId, name: 'Test', platform: mode === 'video' ? 'agnes' : 'openai', enabled: true,
    models: [{ model, enabled: true, capabilities: [{ task: mode === 'video' ? 'video_generation' : 'image_generation', traits: [], protocol }] }],
  } as unknown as IProvider;
  let latest!: CreationDraft;
  function Harness() {
    const [draft, setDraft] = useState<CreationDraft>(() => ({ ...emptyCreationDraft(), mode,
      models: { image: null, video: null, music: null, [mode]: { providerId, model } },
      parameters: { image: {}, video: {}, music: {}, [mode]: { count: 10 } },
    }));
    latest = draft;
    return <CreationComposerContext.Provider value={{ draft, update: change => setDraft(change), setMode: () => {}, selectMode: () => {}, exit: () => {} }}>
      <CreationControls prompt='测试生成' onPromptChange={() => {}} files={[]} />
      <CreationModelSelector files={[]} />
    </CreationComposerContext.Provider>;
  }
  const page = render(<I18nextProvider i18n={i18n}><MemoryRouter><SWRConfig value={{ provider: () => new Map(), fallback: { providers: [provider] }, revalidateOnMount: false }}><Harness /></SWRConfig></MemoryRouter></I18nextProvider>);
  fireEvent.click(page.getByRole('button', { name: '生成模型' }));
  const modelPanel = await page.findByTestId('creation-model-panel');
  fireEvent.click(within(modelPanel).getByRole('button', { name: new RegExp(model) }));
  expect(latest.models[mode]).toEqual({ providerId, model });
  fireEvent.click(page.getByRole('button', { name: /^生成参数：/ }));
  const panel = await page.findByTestId('creation-parameter-panel');
  const count = within(within(panel).getByRole('group', { name: '生成数量' }));
  expect(count.getAllByRole('button').map(button => button.textContent)).toEqual(['1', '2', '3', '4']);
  expect(count.getByRole('button', { name: '4', pressed: true })).toBeTruthy();
  const option = { providerId, model, protocol, label: model };
  expect(buildCreationRequest(latest, '测试生成', presetId, [], option).params.count).toBe(4);
  if (mode === 'video') {
    const ratios = within(within(panel).getByRole('group', { name: '宽高比' }));
    fireEvent.click(ratios.getByRole('button', { name: '16:9' }));
    fireEvent.click(within(within(panel).getByRole('group', { name: '分辨率' })).getByRole('button', { name: '1080p' }));
    fireEvent.click(ratios.getByRole('button', { name: '9:16' }));
    expect(within(panel).getByText('1080 × 1920')).toBeTruthy();
    expect(buildCreationRequest(latest, '测试生成', presetId, [], option).params).toMatchObject({ size: '1080x1920', count: 4 });
    expect(buildCreationRequest(latest, '测试生成', presetId, [], option).params.resolution).toBeUndefined();
    fireEvent.click(ratios.getByRole('button', { name: '自动' }));
    expect(buildCreationRequest(latest, '测试生成', presetId, [], option).params.size).toBeUndefined();
  }
});

test('chat menu switches the creative composer to general assistant and keeps its draft', async () => {
  const initial = emptyCreationDraft();
  initial.mode = 'video';
  initial.lastMode = 'video';
  initial.parameters.video = { size: '1080x1920', count: 4 };
  initial.references = [{ asset_id: 'cat', title: '猫', kind: 'image', role: 'reference' }];
  sessionStorage.setItem(creationDraftStorageKey('guid'), JSON.stringify(initial));
  let current!: ReturnType<typeof useGuidCreation>;
  function Harness() {
    const [selection, setSelection] = useState<GuidAgentSelection>({ kind: 'template', templateKey: 'creative-studio.default' });
    current = useGuidCreation({ selection, setSelection, officialTemplates: [], selectedPreset: undefined, selectedTemplate: undefined,
      presets: [], draftPresets: [], isLoading: false, isLoaded: true, loadError: undefined, refreshPresets: async () => {},
    }, '继续编辑我的描述', ['C:/cat.png'], '');
    return <CreationComposerContext.Provider value={current}>
      <output>{selection.kind === 'template' ? selection.templateKey : selection.presetId}</output>
      <ComposerSceneSelector />
      <CreationControls prompt='继续编辑我的描述' onPromptChange={() => {}} files={['C:/cat.png']} />
    </CreationComposerContext.Provider>;
  }
  try {
    const page = render(<I18nextProvider i18n={i18n}><MemoryRouter><SWRConfig value={{ provider: () => new Map(), fallback: { providers: [] }, revalidateOnMount: false }}><Harness /></SWRConfig></MemoryRouter></I18nextProvider>);
    fireEvent.click(page.getByRole('button', { name: '使用场景：视频创作，展开全部场景' }));
    fireEvent.click(await page.findByRole('menuitemradio', { name: /日常对话/ }));
    expect(page.getByText('assistant.general')).toBeTruthy();
    expect(page.queryByRole('button', { name: /^生成参数：/ })).toBeNull();
    expect(current.draft.mode).toBeNull();
    expect(current.draft.references).toEqual(initial.references);
    expect(current.draft.parameters.video).toEqual(initial.parameters.video);
  } finally { sessionStorage.removeItem(creationDraftStorageKey('guid')); }
});
