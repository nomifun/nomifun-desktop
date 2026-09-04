/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import ImageWorkbench from './ImageWorkbench';
import {
  imageWorkbenchAspectRatioChoices,
  imageWorkbenchResolutionLabel,
  imageWorkbenchResolutionOptions,
  imageWorkbenchSizePolicyForModel,
  imageWorkbenchSizeOptionForAspectRatio,
  imageWorkbenchSelectableSizeOptions,
  normalizeImageWorkbenchSettingsSize,
  imageWorkbenchModelKey,
  nextImageWorkbenchSelection,
  parseImageWorkbenchModelKey,
  type ImageWorkbenchProps,
  type ImageWorkbenchResult,
} from './types';

const noop = () => undefined;
const testI18n = createInstance();

await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
  interpolation: { escapeValue: false },
});

const baseProps = (overrides: Partial<ImageWorkbenchProps> = {}): ImageWorkbenchProps => ({
  layout: 'side',
  prompt: '',
  references: [],
  settings: {
    model: { providerId: 'provider-a', model: 'image-model' },
    interfaceMode: 'images',
    quality: 'auto',
    width: 1024,
    height: 1024,
    aspectRatio: '1:1',
    count: 1,
  },
  modelOptions: [
    {
      providerId: 'provider-a',
      model: 'image-model',
      label: 'Image Model',
      providerLabel: 'Provider A',
    },
  ],
  results: [],
  selectedResultIds: [],
  task: { state: 'idle', pendingCount: 0 },
  onLayoutChange: noop,
  onPromptChange: noop,
  onRemoveReference: noop,
  onModelChange: noop,
  onInterfaceModeChange: noop,
  onQualityChange: noop,
  onAspectRatioChange: noop,
  onCountChange: noop,
  onGenerate: noop,
  onResultSelectionChange: noop,
  onDeleteResult: noop,
  onDeleteSelected: noop,
  ...overrides,
});

const renderWorkbench = (overrides: Partial<ImageWorkbenchProps> = {}) =>
  renderToStaticMarkup(
    <I18nextProvider i18n={testI18n}>
      <ImageWorkbench {...baseProps(overrides)} />
    </I18nextProvider>
  );

const resultBase = {
  id: 'result-1',
  taskId: 'task-1',
  prompt: '一座被晨雾包围的未来城市',
  model: { providerId: 'provider-a', model: 'image-model' },
  modelLabel: 'Provider A · Image Model',
  createdAtLabel: '刚刚',
};

describe('ImageWorkbench visual states', () => {
  test('renders deleted history outputs as an explicit placeholder without media requests', () => {
    const html = renderWorkbench({ results: [{ ...resultBase, status: 'succeeded', hasDeletedInputs: true,
      outputs: [{ assetId: 'deleted-output', imageUrl: '/deleted-original', alt: 'Old result', availability: 'deleted' }],
    }] });
    expect(html.includes('素材已删除')).toBe(true);
    expect(html.includes('引用素材已删除')).toBe(true);
    expect(html.includes('<img')).toBe(false);
    expect(html.includes('/deleted-original')).toBe(false);
  });
  test('renders the side composer and a real empty result state', () => {
    const html = renderWorkbench();

    expect(html.includes('data-image-workbench="true"')).toBe(true);
    expect(html.includes('data-workbench-layout="side"')).toBe(true);
    expect(html.includes('data-image-workbench-composer="side"')).toBe(true);
    expect(html.includes('data-image-result-state="empty"')).toBe(true);
    expect(html.includes('生图工作台')).toBe(true);
    expect(html.includes('还没有生成图片')).toBe(true);
    expect(html.includes('<img')).toBe(false);
  });

  test('keeps workbench headings compact and reference content inside the sidebar', () => {
    const css = readFileSync(new URL('./ImageWorkbench.module.css', import.meta.url), 'utf8');
    const composerSource = readFileSync(new URL('./ImageWorkbenchComposer.tsx', import.meta.url), 'utf8');
    const resultsSource = readFileSync(new URL('./ImageWorkbenchResults.tsx', import.meta.url), 'utf8');

    expect(composerSource.includes('className={styles.composerHeading}')).toBe(true);
    expect(composerSource.includes('<Pic size={20} />')).toBe(true);
    expect(composerSource.includes("'creativeStudio.image.header.settings'")).toBe(true);
    expect(/\.composerHeader\s*\{[\s\S]*?align-items:\s*center;/.test(css)).toBe(true);
    expect(/\.composerHeader h1\s*\{[\s\S]*?font-size:\s*14px;[\s\S]*?line-height:\s*18px;/.test(css)).toBe(true);
    expect(/\.resultsTitle h2\s*\{[\s\S]*?font-size:\s*14px;[\s\S]*?line-height:\s*20px;/.test(css)).toBe(true);
    expect(/\.layoutSwitch :global\(\.arco-btn\)\s*\{[\s\S]*?height:\s*28px;[\s\S]*?font-size:\s*12px;/.test(css)).toBe(true);
    expect(/\.layoutSwitch :global\(\.arco-btn\) > :global\(\.i-icon\)[\s\S]*?line-height:\s*0;/.test(css)).toBe(true);
    expect(/\.resultsHeader\s*\{[\s\S]*?min-height:\s*64px;[\s\S]*?padding:\s*12px 16px;/.test(css)).toBe(true);
    expect(resultsSource.includes("<History size={15} />")).toBe(true);
    expect(resultsSource.includes("<Tag size='small' bordered={false}>")).toBe(true);
    expect(/\.composerScroll\s*\{[\s\S]*?padding:\s*12px 16px 16px;/.test(css)).toBe(true);
    expect(/\.sectionHeader\s*\{[\s\S]*?box-sizing:\s*border-box;[\s\S]*?min-height:\s*34px;[\s\S]*?padding:\s*5px 10px;/.test(css)).toBe(true);
    expect(/\.referenceStrip\s*\{[\s\S]*?box-sizing:\s*border-box;[\s\S]*?width:\s*100%;[\s\S]*?max-width:\s*100%;/.test(css)).toBe(true);
    expect(composerSource.includes('SettingTwo')).toBe(false);
  });

  test('keeps side controls dense and both workbench panes inside their viewport', () => {
    const css = readFileSync(new URL('./ImageWorkbench.module.css', import.meta.url), 'utf8');
    const pickerCss = readFileSync(new URL('./ImageSizePicker.module.css', import.meta.url), 'utf8');

    expect(/\.sideLayout,\s*\.bottomLayout\s*\{[\s\S]*?box-sizing:\s*border-box;[\s\S]*?width:\s*100%;[\s\S]*?height:\s*100%;/.test(css)).toBe(true);
    expect(/\.sideLayout\s*\{[\s\S]*?grid-template-columns:\s*minmax\(330px, 380px\) minmax\(0, 1fr\);/.test(css)).toBe(true);
    expect(pickerCss.includes('grid-template-columns: repeat(3, minmax(0, 1fr))')).toBe(true);
    expect(pickerCss.includes('min-height: 30px')).toBe(true);
    expect(/\.aspectShape\s*\{[\s\S]*?width:\s*18px;[\s\S]*?max-height:\s*16px;/.test(css)).toBe(true);
    expect(css.includes('.compactResolutionField')).toBe(true);
    expect(/\.optionPill\s*\{[\s\S]*?height:\s*28px;[\s\S]*?font-size:\s*10px;/.test(css)).toBe(true);
    expect(css.includes('.dimensionGrid')).toBe(false);
    expect(css.includes('.sizeOptionIdentity')).toBe(true);
  });

  test('keeps the side-by-side workspace through compact desktop widths and adapts the result list', () => {
    const css = readFileSync(new URL('./ImageWorkbench.module.css', import.meta.url), 'utf8');

    expect(
      /@media \(max-width:\s*820px\)\s*\{[\s\S]*?\.sideLayout\s*\{[\s\S]*?flex-direction:\s*column;/.test(
        css
      )
    ).toBe(true);
    expect(css.includes('@media (max-width: 900px)')).toBe(false);
    expect(
      /@media \(max-width:\s*1120px\)\s*\{[\s\S]*?\.resultGrid\s*\{[\s\S]*?grid-template-columns:\s*minmax\(0, 1fr\);/.test(
        css
      )
    ).toBe(true);
    expect(/\.resultsPanel\s*\{[\s\S]*?overflow:\s*hidden;/.test(css)).toBe(true);
    expect(/\.resultGrid\s*\{[\s\S]*?overflow-y:\s*auto;/.test(css)).toBe(true);
  });

  test('renders the floating bottom composer with exact model and parameter controls', () => {
    const html = renderWorkbench({
      layout: 'bottom',
      prompt: '产品摄影，柔和侧光',
      task: { state: 'running', pendingCount: 2, message: '正在生成' },
    });

    expect(html.includes('data-workbench-layout="bottom"')).toBe(true);
    expect(html.includes('data-image-workbench-composer="bottom"')).toBe(true);
    expect(html.includes('Images')).toBe(true);
    expect(html.includes('Responses')).toBe(true);
    expect(html.includes('宽高比')).toBe(true);
    expect(html.includes('分辨率')).toBe(true);
    expect(html.includes('质量')).toBe(true);
    expect(html.includes('数量')).toBe(true);
    expect(html.includes('2 个生成中')).toBe(true);

    const css = readFileSync(new URL('./ImageWorkbench.module.css', import.meta.url), 'utf8');
    const composerSource = readFileSync(
      new URL('./ImageWorkbenchComposer.tsx', import.meta.url),
      'utf8'
    );
    expect(composerSource.includes('className={styles.bottomComposerBody}')).toBe(true);
    expect(composerSource.includes('className={styles.bottomActionRow}')).toBe(true);
    expect(
      /\.bottomComposerBody\s*\{[\s\S]*?grid-template-columns:\s*minmax\(330px, 0\.86fr\) minmax\(0, 1\.14fr\);/.test(
        css
      )
    ).toBe(true);
    expect(
      /\.compactSettings\s*\{[\s\S]*?grid-template-columns:\s*repeat\(12, minmax\(0, 1fr\)\);/.test(
        css
      )
    ).toBe(true);
  });

  test('renders an honest disabled state when no image model is configured', () => {
    const html = renderWorkbench({
      settings: { ...baseProps().settings, model: null },
      modelOptions: [],
    });

    expect(html.includes('没有可用生图模型')).toBe(true);
    expect(html.includes('aria-label="生图模型"')).toBe(true);
    expect(html.includes('arco-select-view-disabled')).toBe(true);
  });

  test('renders a catalog-owned model selector instead of the fallback field', () => {
    const html = renderWorkbench({
      modelSlot: <div data-model-selector-state='no-compatible-model'>配置生图模型</div>,
    });

    expect(html.includes('data-model-selector-state="no-compatible-model"')).toBe(true);
    expect(html.includes('配置生图模型')).toBe(true);
    expect(html.includes('aria-label="生图模型"')).toBe(false);
  });

  test('renders queued, running, failed and canceled cards without invented media', () => {
    const results: ImageWorkbenchResult[] = [
      { ...resultBase, status: 'queued' },
      { ...resultBase, id: 'result-2', taskId: 'task-2', status: 'running', progress: 46 },
      {
        ...resultBase,
        id: 'result-3',
        taskId: 'task-3',
        status: 'failed',
        errorMessage: 'provider returned 400 Bad Request: {"error":{"message":"size 不支持，当前模型支持的 size..."}}',
      },
      {
        ...resultBase,
        id: 'result-4',
        taskId: 'task-4',
        status: 'canceled',
        message: '用户已取消任务',
      },
    ];
    const html = renderWorkbench({
      results,
      task: { state: 'running', pendingCount: 1 },
      onRetryResult: noop,
    });

    expect(html.includes('data-image-result-state="queued"')).toBe(true);
    expect(html.includes('data-image-result-state="running"')).toBe(true);
    expect(html.includes('data-image-result-state="failed"')).toBe(true);
    expect(html.includes('data-image-result-state="canceled"')).toBe(true);
    expect(html.includes('排队中')).toBe(true);
    expect(html.includes('生成中')).toBe(true);
    expect(html.includes('生成失败')).toBe(true);
    expect(html.includes('已取消')).toBe(true);
    expect(html.includes('provider returned 400 Bad Request')).toBe(true);
    expect(html.includes('复制完整报错信息')).toBe(true);
    const css = readFileSync(new URL('./ImageWorkbench.module.css', import.meta.url), 'utf8');
    expect(/\.failureMessage\s*\{[\s\S]*?-webkit-line-clamp:\s*2;/.test(css)).toBe(true);
    expect(html.includes('用户已取消任务')).toBe(true);
    expect(html.includes('data-provider-id="provider-a"')).toBe(true);
    expect(html.includes('data-model="image-model"')).toBe(true);
    expect(html.includes('<img')).toBe(false);
  });

  test('renders only caller-supplied media for references and successful results', () => {
    const html = renderWorkbench({
      references: [
        { id: 'reference-1', name: '真实参考图', previewUrl: 'https://media.invalid/reference.png' },
      ],
      results: [
        {
          ...resultBase,
          status: 'succeeded',
          outputs: [
            {
              assetId: 'asset-result-1',
              imageUrl: 'https://media.invalid/result.png',
              alt: '生成的未来城市',
              width: 1536,
              height: 1024,
              sizeLabel: '2.4 MB',
            },
          ],
        },
      ],
      task: { state: 'succeeded', pendingCount: 0 },
    });

    expect(html.includes('https://media.invalid/reference.png')).toBe(true);
    expect(html.includes('https://media.invalid/result.png')).toBe(true);
    expect(html.includes('生成的未来城市')).toBe(true);
    expect(html.includes('1536 × 1024 · 2.4 MB')).toBe(true);
    expect(html.includes('复制提示词')).toBe(true);
    expect(html.includes('data:image')).toBe(false);
  });

  test('removes the result-card load action completely', () => {
    const html = renderWorkbench({
      results: [{ ...resultBase, status: 'succeeded', outputs: [] }],
    });
    const resultsSource = readFileSync(new URL('./ImageWorkbenchResults.tsx', import.meta.url), 'utf8');
    expect(html.includes('载入')).toBe(false);
    expect(resultsSource.includes('onLoadResult')).toBe(false);
  });

  test('keeps queued and canceled task summaries distinct from running and failed', () => {
    const queued = renderWorkbench({ task: { state: 'queued', pendingCount: 2 } });
    const canceled = renderWorkbench({ task: { state: 'canceled', pendingCount: 0 } });

    expect(queued.includes('2 个排队中')).toBe(true);
    expect(canceled.includes('最近任务已取消')).toBe(true);
  });

  test('offers history retirement only for terminal task cards', () => {
    const html = renderWorkbench({
      results: [
        { ...resultBase, status: 'queued', deletable: false },
        {
          ...resultBase,
          id: 'terminal-task',
          taskId: 'terminal-task',
          status: 'failed',
          errorMessage: 'failed',
          deletable: true,
        },
      ],
      selectedResultIds: ['terminal-task'],
    });
    expect(html.includes('选择结果 result-1')).toBe(false);
    expect(html.includes('选择结果 terminal-task')).toBe(true);
    expect(html.includes('从历史移除 terminal-task')).toBe(true);
    expect(html.includes('移除 1')).toBe(true);
  });
});

describe('ImageWorkbench model size policies', () => {
  test('maps StepFun display dimensions to its native height-by-width enum', () => {
    const policy = imageWorkbenchSizePolicyForModel({
      platform: 'stepfun',
      protocol: 'stepfun.images',
      model: 'step-image-edit-2',
    });
    expect(policy.allowCustomDimensions).toBe(false);
    expect(policy.maxCount).toBe(1);
    expect(policy.options.find((option) => option.value === '16:9')).toMatchObject({
      width: 1360,
      height: 768,
      requestSize: '768x1360',
    });
    expect(policy.options.find((option) => option.value === '9:16')).toMatchObject({
      width: 768,
      height: 1360,
      requestSize: '1360x768',
    });
  });

  test('keeps custom dimensions for non-StepFun models', () => {
    const policy = imageWorkbenchSizePolicyForModel({
      platform: 'custom',
      protocol: 'custom.images',
      model: 'custom-image',
    });
    expect(policy.allowCustomDimensions).toBe(true);
    expect(policy.maxCount).toBe(10);
  });

  test('does not expose unsupported repeat count for Ark image requests', () => {
    const policy = imageWorkbenchSizePolicyForModel({
      platform: 'ark',
      protocol: 'ark.images',
      model: 'ep-seedream',
    });
    expect(policy.allowCustomDimensions).toBe(true);
    expect(policy.maxCount).toBe(1);
  });

  test('uses the selected size option as the sole UI authority for dimensions', () => {
    const policy = imageWorkbenchSizePolicyForModel({
      platform: 'custom',
      protocol: 'custom.images',
      model: 'custom-image',
    });
    const normalized = normalizeImageWorkbenchSettingsSize(
      {
        ...baseProps().settings,
        aspectRatio: '16:9',
        width: 64,
        height: 1024,
      },
      policy
    );
    const option = policy.options.find((candidate) => candidate.value === '16:9')!;
    expect(normalized).toMatchObject({
      aspectRatio: '16:9',
      width: option.width,
      height: option.height,
    });
    expect(option.requestSize).toBe(`${option.width}x${option.height}`);
  });

  test('keeps automatic sizing without restoring manual width and height inputs', () => {
    const policy = imageWorkbenchSizePolicyForModel(null);
    const selectable = imageWorkbenchSelectableSizeOptions(policy.options);
    expect(selectable.some((option) => option.value === 'auto')).toBe(true);
    expect(
      normalizeImageWorkbenchSettingsSize(
        { ...baseProps().settings, aspectRatio: 'auto' },
        policy
      )
    ).toMatchObject({ aspectRatio: 'auto', width: null, height: null });
  });

  test('splits ratio and resolution while preserving the exact provider option', () => {
    const policy = imageWorkbenchSizePolicyForModel(null);
    const ratios = imageWorkbenchAspectRatioChoices(policy.options);
    expect(ratios.filter((option) => option.value === '16:9')).toHaveLength(1);

    const resolutions = imageWorkbenchResolutionOptions(policy.options, '16:9');
    expect(resolutions.map(imageWorkbenchResolutionLabel)).toEqual(['标准', '2K', '4K']);

    const current = policy.options.find((option) => option.value === '2048x2048')!;
    expect(
      imageWorkbenchSizeOptionForAspectRatio(policy.options, current, '16:9')
    ).toMatchObject({
      value: '2048x1152',
      width: 2048,
      height: 1152,
      requestSize: '2048x1152',
    });
  });
});

describe('ImageWorkbench controlled contract', () => {
  test('preserves provider plus model identity and never resolves by model name alone', () => {
    const options = [
      { providerId: 'provider-a', model: 'shared-model', label: 'A' },
      { providerId: 'provider-b', model: 'shared-model', label: 'B' },
    ];
    const key = imageWorkbenchModelKey(options[1]);

    expect(key).toBe(JSON.stringify(['provider-b', 'shared-model']));
    expect(key.includes('\u0000')).toBe(false);
    expect(key).not.toBe(imageWorkbenchModelKey(options[0]));
    expect(parseImageWorkbenchModelKey(key, options)).toEqual({
      providerId: 'provider-b',
      model: 'shared-model',
    });
    expect(parseImageWorkbenchModelKey('shared-model', options)).toBeNull();
  });

  test('computes immutable selection changes for controlled consumers', () => {
    expect(nextImageWorkbenchSelection(['a'], 'b', true)).toEqual(['a', 'b']);
    expect(nextImageWorkbenchSelection(['a', 'b'], 'a', false)).toEqual(['b']);
    expect(nextImageWorkbenchSelection(['a'], 'a', true)).toEqual(['a']);
  });

  test('keeps callbacks narrow and the bottom overlay scoped to the component', () => {
    const typesSource = readFileSync(new URL('./types.ts', import.meta.url), 'utf8');
    const css = readFileSync(new URL('./ImageWorkbench.module.css', import.meta.url), 'utf8');
    const componentSource = readFileSync(new URL('./ImageWorkbench.tsx', import.meta.url), 'utf8');

    for (const callback of [
      'onLayoutChange',
      'onPromptChange',
      'onChooseReferences',
      'onModelChange',
      'onInterfaceModeChange',
      'onQualityChange',
      'onAspectRatioChange',
      'onCountChange',
      'onResultSelectionChange',
      'onDeleteSelected',
    ]) {
      expect(typesSource.includes(callback)).toBe(true);
    }
    expect(typesSource.includes('onDimensionsChange')).toBe(false);
    expect(componentSource.includes('httpRequest')).toBe(false);
    expect(componentSource.includes('useModelsForTask')).toBe(false);
    expect(css.includes('.bottomComposerDock {\n  position: absolute;')).toBe(true);
    expect(css.includes('position: fixed')).toBe(false);
    expect(css.includes('@media (max-width: 560px)')).toBe(true);
  });
});
