/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import zh from '../../../../services/i18n/locales/zh-CN/creativeStudio.json';
import type { CreativeCanvasReferencePromptChange } from './CreativeCanvasReferencePromptInput';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';

import type { CreativeModelOption } from '../../models';
import CreativeCanvasVideoComposer, {
  dispatchCanvasVideoComposerSubmission,
  type CreativeCanvasVideoComposerProps,
} from './CreativeCanvasVideoComposer';

const PROVIDER_ID =
  '019b0000-0000-7000-8000-000000000009' as CreativeModelOption['providerId'];
const noop = () => undefined;
afterEach(cleanup);
const i18n = createInstance();
await i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { creativeStudio: zh } } } });
const wrap = (content: React.ReactNode) => <I18nextProvider i18n={i18n}>{content}</I18nextProvider>;
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
  test('inserts a stable reference from @, preserves whitespace and blocks disconnected bindings', () => {
    const changes: CreativeCanvasReferencePromptChange[] = [];
    const generated: CreativeCanvasReferencePromptChange[] = [];
    const reference = {
      nodeId: 'cat-node', assetId: 'cat-asset', connectionId: 'cat-edge',
      base: false, label: '猫咪', ordinal: 1,
    };
    const componentProps = props({
      mode: 'i2v', references: [reference],
      onPromptChange: (change) => changes.push(change),
      onGenerate: (value, mentions) => generated.push({ value, mentions: [...mentions] }),
    });
    const view = render(wrap(<CreativeCanvasVideoComposer {...componentProps} />));
    const input = view.getByRole('combobox', { name: '视频创作提示词' }) as HTMLTextAreaElement;
    fireEvent.change(input, { target: { value: '  让 @', selectionStart: 5, selectionEnd: 5 } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(generated).toHaveLength(0);
    const draft = changes.at(-1)!;
    expect(draft.value).toBe('  让 @图片1 ');
    expect(draft.mentions[0]).toMatchObject({ sourceNodeId: 'cat-node', start: 4, end: 8 });
    fireEvent.keyDown(input, { key: 'Enter', isComposing: true });
    expect(generated).toHaveLength(0);
    fireEvent.keyDown(input, { key: 'Enter', shiftKey: true });
    expect(generated).toHaveLength(0);
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(generated).toEqual([draft]);

    view.rerender(wrap(<CreativeCanvasVideoComposer {...componentProps}
      references={[]} initialPrompt={draft.value} initialMentions={draft.mentions} />));
    expect((view.getByRole('button', { name: '生成视频' }) as HTMLButtonElement).disabled).toBe(true);
    expect(input.getAttribute('aria-invalid')).toBe('true');
    expect(view.getByText(/已断开的素材引用/)).toBeTruthy();
    fireEvent.keyDown(input, { key: 'Enter' });
    fireEvent.click(view.getByRole('button', { name: '生成视频' }));
    expect(generated).toHaveLength(1);
  });

  test('shares locate, disconnect and batch actions and preserves canonical retries', () => {
    const disconnected: string[][] = [];
    const activated: string[] = [];
    let retries = 0;
    const reference = {
      nodeId: 'source', assetId: 'asset', connectionId: 'edge', base: false, label: '客厅', ordinal: 1,
    };
    const componentProps = props({
      mode: 'i2v', references: [reference],
      onReferenceActivate: (id) => activated.push(id),
      onReferenceDisconnect: (id) => disconnected.push([id]),
      onReferencesDisconnect: (ids) => disconnected.push([...ids]),
    });
    const view = render(wrap(<CreativeCanvasVideoComposer {...componentProps} />));
    fireEvent.click(view.getByRole('button', { name: '定位参考 客厅' }));
    fireEvent.click(view.getByRole('button', { name: '断开参考 客厅' }));
    fireEvent.click(view.getByRole('button', { name: '批量管理' }));
    fireEvent.click(view.getByRole('button', { name: '全选连接' }));
    fireEvent.click(view.getByRole('button', { name: '断开所选' }));
    expect(activated).toEqual(['source']);
    expect(disconnected).toEqual([['edge'], ['edge']]);
    view.rerender(wrap(<CreativeCanvasVideoComposer {...componentProps}
      generateBlocked retrySubmission task={{ state: 'queued', pendingCount: 1 }}
      onRetrySubmission={() => { retries += 1; }} />));
    fireEvent.click(view.getByRole('button', { name: '生成视频' }));
    expect(retries).toBe(1);
  });

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
          references: [{
            assetId: 'reference',
            nodeId: 'source', connectionId: 'edge', base: false, ordinal: 1,
            label: '晨雾参考图.png',
            thumbnailUrl: 'http://127.0.0.1:8788/assets/reference.png',
          }],
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
          references: [{ assetId: 'reference', nodeId: 'source', connectionId: 'edge', base: false, ordinal: 1, label: '参考图', originalUrl: '/reference-original.png' }],
        })}
      />
    );

    expect(html.includes('src="/reference-original.png"')).toBe(true);
    expect(html.match(/<img\b/g)?.length).toBe(1);
    expect(html.includes('data-creative-media-preview="image"')).toBe(true);
  });

  test('previews every linked image in keyframe order', () => {
    const html = renderToStaticMarkup(<CreativeCanvasVideoComposer {...props({
      mode: 'i2v', initialPrompt: 'transition',
      modelOptions: [{ ...model, protocol: 'agnes.video_jobs' }],
      references: ['first', 'middle', 'last'].map((id, index) => ({
        assetId: id, nodeId: id, connectionId: id, base: false, ordinal: index + 1,
        label: id, originalUrl: `/${id}.png`,
      })),
    })} />);
    expect(html.match(/<img\b/g)?.length).toBe(3);
    expect(html.indexOf('/first.png')).toBeLessThan(html.indexOf('/middle.png'));
    expect(html.indexOf('/middle.png')).toBeLessThan(html.indexOf('/last.png'));
    expect(html.includes('aria-label="生成视频" disabled')).toBe(false);
    expect(html.includes('关键帧 · 按连线顺序')).toBe(true);
  });

  test('keeps single-image model limits visible without hiding the references', () => {
    for (const protocol of ['openai.videos', 'siliconflow.video_jobs']) {
      const html = renderToStaticMarkup(<CreativeCanvasVideoComposer {...props({
        mode: 'i2v', initialPrompt: 'transition', modelOptions: [{ ...model, protocol }],
        references: ['first', 'last'].map((assetId, index) => ({
          assetId, nodeId: assetId, connectionId: assetId, base: false, ordinal: index + 1,
          label: assetId, originalUrl: `/${assetId}.png`,
        })),
      })} />);
      expect(html.includes('所选模型仅支持一张参考图')).toBe(true);
      expect(html.includes('aria-label="生成视频" disabled')).toBe(true);
      expect(html.match(/<img\b/g)?.length).toBe(2);
    }
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
    expect(html.includes('aria-label="视频创作提示词"')).toBe(true);
    expect(/<textarea[^>]*disabled/.test(html)).toBe(true);
    expect(html.includes('aria-label="生成视频" disabled')).toBe(true);
  });

  test('preserves authored offsets for generation and canonical retry callbacks', () => {
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
    expect(generated).toEqual(['  慢慢拉远  ']);

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
    expect(generated).toEqual(['  慢慢拉远  ']);
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
    const referenceCss = readFileSync(
      new URL('./CreativeCanvasReferenceList.module.css', import.meta.url), 'utf8'
    );
    expect(referenceCss.includes('.referencePreview')).toBe(true);
  });
});
