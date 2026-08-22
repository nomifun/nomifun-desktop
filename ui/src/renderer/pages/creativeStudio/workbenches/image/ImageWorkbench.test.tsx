/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import ImageWorkbench from './ImageWorkbench';
import {
  imageWorkbenchModelKey,
  nextImageWorkbenchSelection,
  parseImageWorkbenchModelKey,
  type ImageWorkbenchProps,
  type ImageWorkbenchResult,
} from './types';

const noop = () => undefined;

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
  onDimensionsChange: noop,
  onAspectRatioChange: noop,
  onCountChange: noop,
  onGenerate: noop,
  onResultSelectionChange: noop,
  onDeleteResult: noop,
  onDeleteSelected: noop,
  ...overrides,
});

const renderWorkbench = (overrides: Partial<ImageWorkbenchProps> = {}) =>
  renderToStaticMarkup(<ImageWorkbench {...baseProps(overrides)} />);

const resultBase = {
  id: 'result-1',
  taskId: 'task-1',
  prompt: '一座被晨雾包围的未来城市',
  model: { providerId: 'provider-a', model: 'image-model' },
  modelLabel: 'Provider A · Image Model',
  createdAtLabel: '刚刚',
};

describe('ImageWorkbench visual states', () => {
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
    expect(html.includes('质量')).toBe(true);
    expect(html.includes('数量')).toBe(true);
    expect(html.includes('2 个生成中')).toBe(true);
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
        errorMessage: '模型暂时不可用',
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
    expect(html.includes('模型暂时不可用')).toBe(true);
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
    expect(html.includes('data:image')).toBe(false);
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

describe('ImageWorkbench controlled contract', () => {
  test('preserves provider plus model identity and never resolves by model name alone', () => {
    const options = [
      { providerId: 'provider-a', model: 'shared-model', label: 'A' },
      { providerId: 'provider-b', model: 'shared-model', label: 'B' },
    ];
    const key = imageWorkbenchModelKey(options[1]);

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
      'onDimensionsChange',
      'onAspectRatioChange',
      'onCountChange',
      'onResultSelectionChange',
      'onDeleteSelected',
    ]) {
      expect(typesSource.includes(callback)).toBe(true);
    }
    expect(componentSource.includes('httpRequest')).toBe(false);
    expect(componentSource.includes('useModelsForTask')).toBe(false);
    expect(css.includes('.bottomComposerDock {\n  position: absolute;')).toBe(true);
    expect(css.includes('position: fixed')).toBe(false);
    expect(css.includes('@media (max-width: 640px)')).toBe(true);
  });
});
