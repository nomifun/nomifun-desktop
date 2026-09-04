/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';

import type { CreativeModelOption } from '../../models';
import CreativeCanvasVideoComposer, {
  dispatchCanvasVideoComposerSubmission,
  isCanvasVideoComposerSubmitKey,
  type CreativeCanvasVideoComposerProps,
} from './CreativeCanvasVideoComposer';

const PROVIDER_ID =
  '019b0000-0000-7000-8000-000000000009' as CreativeModelOption['providerId'];
const noop = () => undefined;
const model: CreativeModelOption = {
  providerId: PROVIDER_ID,
  model: 'video-v1',
  providerName: 'Provider A',
  platform: 'custom',
  task: 'video_generation',
  traits: [],
  protocol: 'test.video_generation',
};

const props = (
  overrides: Partial<CreativeCanvasVideoComposerProps> = {}
): CreativeCanvasVideoComposerProps => ({
  nodeId: '019b0000-0000-7000-8000-000000000001',
  mode: 't2v',
  initialPrompt: '',
  settings: {
    model: { providerId: PROVIDER_ID, model: 'video-v1' },
    resolution: '1080p',
    aspectRatio: '16:9',
    seconds: 5,
  },
  modelOptions: [model],
  task: { state: 'idle', pendingCount: 0 },
  onOpenPromptLibrary: noop,
  onModelChange: noop,
  onResolutionChange: noop,
  onAspectRatioChange: noop,
  onSecondsChange: noop,
  onGenerate: noop,
  ...overrides,
});

describe('CreativeCanvasVideoComposer', () => {
  test('renders the focused text-to-video composer', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasVideoComposer
        {...props({ initialPrompt: '海边的清晨，缓慢推进' })}
      />
    );
    expect(html.includes('data-canvas-video-composer="true"')).toBe(true);
    expect(html.includes('data-mode="t2v"')).toBe(true);
    expect(html.includes('文生视频')).toBe(false);
    expect(html.includes('视频创作提示词')).toBe(true);
    expect(html.includes('描述要生成的视频内容、动作与镜头')).toBe(true);
    expect(html.includes('打开视频提示词库')).toBe(true);
    expect(html.includes('视频生成模型')).toBe(true);
    expect(html.includes('视频生成设置')).toBe(true);
    expect(html.includes('1080p · 16:9 · 5 秒')).toBe(true);
    expect(html.includes('aria-label="生成视频"')).toBe(true);
  });

  test('renders exactly one image reference for image-to-video', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasVideoComposer
        {...props({
          mode: 'i2v',
          reference: {
            name: '晨雾参考图.png',
            previewUrl: 'http://127.0.0.1:8788/assets/reference.png',
          },
        })}
      />
    );
    expect(html.includes('data-mode="i2v"')).toBe(true);
    expect(html.includes('图生视频·1张参考图')).toBe(false);
    expect(html.includes('晨雾参考图.png')).toBe(true);
    expect(html.includes('reference.png')).toBe(true);
    expect(html.includes('data-creative-media-preview="image"')).toBe(true);
    expect(html.match(/<img\b/g)?.length).toBe(1);
    expect(html.includes('描述参考图要如何运动、变化与运镜')).toBe(true);
  });

  test('uses the original image when an image-to-video reference has no thumbnail', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasVideoComposer
        {...props({
          mode: 'i2v',
          reference: { name: '参考图', originalUrl: '/reference-original.png' },
        })}
      />
    );

    expect(html.includes('src="/reference-original.png"')).toBe(true);
    expect(html.match(/<img\b/g)?.length).toBe(1);
    expect(html.includes('data-creative-media-preview="image"')).toBe(true);
  });

  test('keeps generation disabled when no exact video model exists', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasVideoComposer
        {...props({
          settings: { ...props().settings, model: null },
          modelOptions: [],
        })}
      />
    );
    expect(html.includes('没有可用的视频生成模型，请先在模型管理中配置。')).toBe(
      true
    );
    expect(html.includes('aria-label="生成视频" disabled')).toBe(true);
  });

  test('makes the unsupported mode explicit and inert', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasVideoComposer
        {...props({ mode: 'unsupported', initialPrompt: '不应提交' })}
      />
    );
    expect(html.includes('data-mode="unsupported"')).toBe(true);
    expect(html.includes('当前节点不支持直接生成视频')).toBe(true);
    expect(html.includes('aria-label="视频创作提示词" disabled')).toBe(true);
    expect(html.includes('aria-label="生成视频" disabled')).toBe(true);
  });

  test('dispatches trimmed generation and canonical retry callbacks', () => {
    const generated: string[] = [];
    let retries = 0;
    expect(
      dispatchCanvasVideoComposerSubmission({
        mode: 't2v',
        disabled: false,
        busy: false,
        prompt: '  慢慢拉远  ',
        hasModel: true,
        retrySubmission: false,
        onGenerate: (prompt) => generated.push(prompt),
      })
    ).toBe('generated');
    expect(generated).toEqual(['慢慢拉远']);

    expect(
      dispatchCanvasVideoComposerSubmission({
        mode: 'i2v',
        disabled: false,
        busy: true,
        prompt: '',
        hasModel: false,
        retrySubmission: true,
        onGenerate: (prompt) => generated.push(prompt),
        onRetrySubmission: () => {
          retries += 1;
        },
      })
    ).toBe('retried');
    expect(retries).toBe(1);
    expect(generated).toEqual(['慢慢拉远']);
  });

  test('offers an authoritative status check for an uncertain submission', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasVideoComposer
        {...props({
          retrySubmission: true,
          error: '任务提交结果尚未确认',
          onRetrySubmission: noop,
          onConfirmSubmission: noop,
        })}
      />
    );
    expect(html.includes('任务提交结果尚未确认')).toBe(true);
    expect(html.includes('确认任务状态')).toBe(true);
    expect(html.includes('确认任务状态</button>')).toBe(true);
  });

  test('submits on Enter while preserving Shift+Enter for a newline', () => {
    expect(isCanvasVideoComposerSubmitKey('Enter', false)).toBe(true);
    expect(isCanvasVideoComposerSubmitKey('Enter', true)).toBe(false);
    expect(isCanvasVideoComposerSubmitKey('a', false)).toBe(false);
  });

  test('wires only the supported controlled video settings', () => {
    const component = readFileSync(
      new URL('./CreativeCanvasVideoComposer.tsx', import.meta.url),
      'utf8'
    );
    for (const callback of [
      'onOpenPromptLibrary',
      'onModelChange',
      'onResolutionChange',
      'onAspectRatioChange',
      'onSecondsChange',
      'onGenerate',
      'onRetrySubmission',
      'onConfirmSubmission',
    ]) {
      expect(component.includes(callback)).toBe(true);
    }
    expect(component.includes("['720p', '1080p']")).toBe(true);
    expect(component.includes("'16:9',\n  '9:16',\n  '1:1'")).toBe(true);
    expect(component.includes('[5, 10]')).toBe(true);
    expect(component.includes('videoWorkbenchSizeOptionLabel')).toBe(false);
    expect(component.includes('credits')).toBe(false);
    expect(component.includes('camera')).toBe(false);
    expect(component.includes("'v2v'")).toBe(false);
  });

  test('uses the shared compact shell and keeps video context styling', () => {
    const css = readFileSync(
      new URL('./CreativeCanvasVideoComposer.module.css', import.meta.url),
      'utf8'
    );
    const shellCss = readFileSync(
      new URL('./CreativeCanvasComposerShell.module.css', import.meta.url),
      'utf8'
    );
    expect(css.includes('--color-bg-1: #faf9f7')).toBe(false);
    expect(css.includes('--color-bg-popup: #faf9f7')).toBe(false);
    expect(css.includes('--color-secondary: #f1efea')).toBe(false);
    expect(shellCss.includes(":global([data-theme='light']) .positioner")).toBe(true);
    expect(shellCss.includes(":global([data-theme='dark']) .positioner")).toBe(true);
    expect(shellCss.includes('background: color-mix(in srgb, var(--color-bg-2)')).toBe(true);
    expect(shellCss.includes('background: rgb(var(--primary-6))')).toBe(true);
    expect(shellCss.includes('height: 92px')).toBe(true);
    expect(shellCss.includes('height: 30px')).toBe(true);
    expect(/\.controls\s*\{[\s\S]*?flex-wrap:\s*nowrap;/.test(shellCss)).toBe(true);
    expect(
      /\.settingsButton\s*\{[\s\S]*?flex:\s*0 1 144px;[\s\S]*?flex-direction:\s*row;[\s\S]*?flex-wrap:\s*nowrap;[\s\S]*?overflow:\s*hidden;/.test(
        shellCss
      )
    ).toBe(true);
    expect(
      /\.settingsSummary\s*\{[\s\S]*?flex:\s*1 1 auto;[\s\S]*?text-overflow:\s*ellipsis;[\s\S]*?white-space:\s*nowrap;/.test(
        shellCss
      )
    ).toBe(true);
    expect(
      /\.settingsButton > button\s*\{[\s\S]*?display:\s*inline-flex;[\s\S]*?flex-direction:\s*row;[\s\S]*?flex-wrap:\s*nowrap;/.test(
        shellCss
      )
    ).toBe(true);
    expect(shellCss.includes(".positioner[data-placement='above']")).toBe(true);
    expect(shellCss.includes('--creative-canvas-composer-offset-x')).toBe(true);
    expect(shellCss.includes(".positioner[data-overlay='true']")).toBe(true);
    expect(css.includes('.contextRow')).toBe(true);
    expect(css.includes('.modePill')).toBe(true);
    expect(css.includes('.referencePreview')).toBe(true);
  });
});
