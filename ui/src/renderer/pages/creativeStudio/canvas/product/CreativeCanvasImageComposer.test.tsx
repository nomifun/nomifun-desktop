/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';

import CreativeCanvasImageComposer, {
  type CreativeCanvasImageComposerProps,
} from './CreativeCanvasImageComposer';

const noop = () => undefined;

const props = (
  overrides: Partial<CreativeCanvasImageComposerProps> = {}
): CreativeCanvasImageComposerProps => ({
  nodeId: '019b0000-0000-7000-8000-000000000001',
  hasImageContent: true,
  initialPrompt: '',
  settings: {
    model: { providerId: 'provider-a', model: 'edit-v1' },
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
      model: 'edit-v1',
      label: 'edit-v1',
      providerLabel: 'Provider A',
    },
  ],
  task: { state: 'idle', pendingCount: 0 },
  onOpenPromptLibrary: noop,
  onModelChange: noop,
  onInterfaceModeChange: noop,
  onQualityChange: noop,
  onDimensionsChange: noop,
  onAspectRatioChange: noop,
  onCountChange: noop,
  onGenerate: noop,
  ...overrides,
});

describe('CreativeCanvasImageComposer', () => {
  test('renders the focused reference-style node composer', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasImageComposer {...props({ initialPrompt: '改成清晨' })} />
    );
    expect(html.includes('data-canvas-image-composer="true"')).toBe(true);
    expect(html.includes('图片创作提示词')).toBe(true);
    expect(html.includes('请输入你想要把这张图修改成什么')).toBe(true);
    expect(html.includes('打开提示词库')).toBe(true);
    expect(html.includes('图片编辑模型')).toBe(true);
    expect(html.includes('arco-select-size-mini')).toBe(true);
    expect(html.includes('图片生成设置')).toBe(true);
    expect(html.includes('生成图片')).toBe(true);
    expect(html.includes('自动 · 1:1 · 1 张')).toBe(true);
  });

  test('keeps an uncertain submission retryable without inventing another key', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasImageComposer
        {...props({
          initialPrompt: '',
          task: { state: 'queued', pendingCount: 1 },
          retrySubmission: true,
          onRetrySubmission: noop,
          error: '任务提交结果尚未确认',
        })}
      />
    );
    expect(html.includes('任务提交结果尚未确认')).toBe(true);
    expect(html.includes('aria-label="生成图片"')).toBe(true);
    expect(html.includes('aria-label="生成图片" disabled')).toBe(false);
  });

  test('projects an empty image node as text-to-image rather than image editing', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasImageComposer
        {...props({
          hasImageContent: false,
          settings: {
            ...props().settings,
            model: { providerId: 'provider-a', model: 'generate-v1' },
          },
          modelOptions: [
            {
              providerId: 'provider-a',
              model: 'generate-v1',
              label: 'generate-v1',
              providerLabel: 'Provider A',
            },
          ],
        })}
      />
    );
    expect(html.includes('描述要生成的图片内容')).toBe(true);
    expect(html.includes('aria-label="图片生成模型"')).toBe(true);
    expect(html.includes('请输入你想要把这张图修改成什么')).toBe(false);
  });

  test('inherits the active application theme for its creation surface', () => {
    const css = readFileSync(
      new URL('./CreativeCanvasImageComposer.module.css', import.meta.url),
      'utf8'
    );
    expect(css.includes('--color-bg-1: #faf9f7')).toBe(false);
    expect(css.includes('--color-bg-popup: #faf9f7')).toBe(false);
    expect(css.includes('--color-secondary: #f1efea')).toBe(false);
    expect(css.includes(":global([data-theme='light']) .positioner")).toBe(true);
    expect(css.includes(":global([data-theme='dark']) .positioner")).toBe(true);
    expect(css.includes('background: color-mix(in srgb, var(--color-bg-2)')).toBe(true);
    expect(css.includes('background: rgb(var(--primary-6))')).toBe(true);
    expect(css.includes('@media (prefers-color-scheme: dark)')).toBe(false);
    expect(css.includes('width: 580px')).toBe(true);
    expect(css.includes('height: 104px')).toBe(true);
    expect(css.includes('height: 160px')).toBe(false);
    expect(css.includes('flex: 0 1 156px')).toBe(true);
    expect(css.includes('min-width: 124px')).toBe(true);
    expect(css.includes('min-width: 48px')).toBe(true);
    expect(css.includes('font-size: 11px')).toBe(true);
    expect(css.includes('width: 14px')).toBe(true);
    expect(css.includes('line-height: 28px')).toBe(true);
    expect(css.includes('.arco-select-popup .arco-select-option')).toBe(true);
    expect(css.includes('min-height: 28px')).toBe(true);
    expect(css.includes('padding: 0 10px')).toBe(true);
    expect(css.includes('.controls > *,\n.submitButton')).toBe(true);
    expect(css.includes('.footer :global(.i-icon)')).toBe(true);
    expect(css.includes('place-items: center')).toBe(true);
    expect(css.includes(".positioner[data-placement='above']")).toBe(true);
    expect(css.includes('--creative-canvas-image-composer-offset-x')).toBe(true);
    expect(css.includes(".positioner[data-overlay='true']")).toBe(true);
    expect(css.includes('.settingsSummary')).toBe(true);
    expect(css.includes('.settingsButton span')).toBe(false);
  });

});
