/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';

import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import CreativeCanvasImageComposer, {
  type CreativeCanvasImageComposerProps,
} from './CreativeCanvasImageComposer';
import type { CreativeCanvasReferencePromptChange } from './CreativeCanvasReferencePromptInput';

const noop = () => undefined;

afterEach(() => cleanup());

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
  aspectRatioOptions: [
    {
      value: '1:1',
      label: '1:1',
      width: 1024,
      height: 1024,
      requestSize: '1024x1024',
    },
    {
      value: '16:9',
      label: '16:9',
      width: 1920,
      height: 1080,
      requestSize: '1920x1080',
    },
    {
      value: 'auto',
      label: '自动',
      width: null,
      height: null,
    },
  ],
  maxCount: 10,
  task: { state: 'idle', pendingCount: 0 },
  onOpenPromptLibrary: noop,
  onModelChange: noop,
  onInterfaceModeChange: noop,
  onQualityChange: noop,
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

  test('opens stable in-flow quality and aspect-ratio selectors', async () => {
    const qualityChanges: string[] = [];
    const aspectRatioChanges: string[] = [];
    const { getByRole } = render(
      withCanvasTestI18n(
        <CreativeCanvasImageComposer
          {...props({
            onQualityChange: (quality) => qualityChanges.push(quality),
            onAspectRatioChange: (option) => aspectRatioChanges.push(option.value),
          })}
        />
      )
    );

    const settingsButton = getByRole('button', { name: '图片生成设置' });
    fireEvent.click(settingsButton);

    await waitFor(() => {
      expect(document.querySelectorAll('select').length).toBe(1);
    });
    const [qualitySelect] = Array.from(
      document.querySelectorAll<HTMLSelectElement>('select')
    );

    expect(qualitySelect.closest('[data-canvas-image-composer]')).not.toBeNull();
    fireEvent.change(qualitySelect, { target: { value: 'high' } });
    fireEvent.click(getByRole('button', { name: '宽高比' }));
    const sizeListbox = getByRole('listbox', { name: '宽高比' });
    const sizeOption = getByRole('option', {
      name: /16:9.*1920 × 1080/,
    });
    expect(within(sizeListbox).getByRole('option', { name: '自动' })).not.toBeNull();
    expect(sizeOption.lastElementChild?.textContent).toBe('1920 × 1080');
    fireEvent.click(sizeOption);
    expect(qualityChanges).toEqual(['high']);
    expect(aspectRatioChanges).toEqual(['16:9']);
    expect(settingsButton.getAttribute('aria-expanded')).toBe('true');

    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => {
      expect(document.querySelectorAll('select').length).toBe(0);
    });
    expect(settingsButton.getAttribute('aria-expanded')).toBe('false');

    fireEvent.click(settingsButton);
    await waitFor(() => {
      expect(document.querySelectorAll('select').length).toBe(1);
    });
    fireEvent.pointerDown(document.body);
    await waitFor(() => {
      expect(document.querySelectorAll('select').length).toBe(0);
    });
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

  test('shows connected references, disconnects an edge, and inserts a stable @ mention', () => {
    const disconnected: string[] = [];
    const promptChanges: Array<{ value: string; mentions: unknown[] }> = [];
    const { getByRole } = render(
      withCanvasTestI18n(
        <CreativeCanvasImageComposer
          {...props({
            references: [
              {
                nodeId: 'person-node',
                assetId: 'asset-person',
                connectionId: 'edge-person',
                base: false,
                label: '人物图',
                thumbnailUrl: 'https://example.test/person.png',
                ordinal: 1,
              },
              {
                nodeId: 'clothes-node',
                assetId: 'asset-clothes',
                connectionId: 'edge-clothes',
                base: false,
                label: '服装图',
                thumbnailUrl: 'https://example.test/clothes.png',
                ordinal: 2,
              },
            ],
            referenceCapacityLabel: '2/3',
            onReferenceDisconnect: (connectionId) => disconnected.push(connectionId),
            onPromptChange: (change) => promptChanges.push(change),
          })}
        />
      )
    );

    expect(getByRole('list').textContent?.includes('人物图')).toBe(true);
    expect(getByRole('list').textContent?.includes('服装图')).toBe(true);
    fireEvent.click(getByRole('button', { name: '断开参考图 服装图' }));
    expect(disconnected).toEqual(['edge-clothes']);

    fireEvent.click(getByRole('button', { name: '引用已连接素材' }));
    fireEvent.click(getByRole('option', { name: /@图片1.*人物图/ }));
    expect(promptChanges.at(-1)).toMatchObject({
      value: '@图片1 ',
      mentions: [{ sourceNodeId: 'person-node', fallbackLabel: '图片1' }],
    });
  });

  test('preserves authored whitespace so mention offsets remain valid on submit', () => {
    const submissions: Array<{ prompt: string; start: number }> = [];
    const migrations: CreativeCanvasReferencePromptChange[] = [];
    const { getByRole } = render(
      withCanvasTestI18n(
        <CreativeCanvasImageComposer
          {...props({
            initialPrompt: '  @人物图',
            initialMentions: [
              {
                id: 'mention-person',
                sourceNodeId: 'person-node',
                fallbackLabel: '人物图',
                start: 2,
                end: 6,
              },
            ],
            references: [
              {
                nodeId: 'person-node',
                assetId: 'asset-person',
                connectionId: 'edge-person',
                base: false,
                label: '人物图',
                ordinal: 1,
              },
            ],
            onPromptChange: (change) => migrations.push(change),
            onGenerate: (prompt, mentions) =>
              submissions.push({ prompt, start: mentions[0]?.start ?? -1 }),
          })}
        />
      )
    );

    expect(migrations).toEqual([
      {
        value: '  @图片1',
        mentions: [
          {
            id: 'mention-person',
            sourceNodeId: 'person-node',
            fallbackLabel: '图片1',
            start: 2,
            end: 6,
          },
        ],
      },
    ]);
    fireEvent.click(getByRole('button', { name: '生成图片' }));
    expect(submissions).toEqual([{ prompt: '  @图片1', start: 2 }]);
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

  test('uses the shared compact shell while keeping image-specific controls', () => {
    const css = readFileSync(
      new URL('./CreativeCanvasImageComposer.module.css', import.meta.url),
      'utf8'
    );
    const shellCss = readFileSync(
      new URL('./CreativeCanvasComposerShell.module.css', import.meta.url),
      'utf8'
    );
    const promptCss = readFileSync(
      new URL('./CreativeCanvasReferencePromptInput.module.css', import.meta.url),
      'utf8'
    );
    expect(css.includes('--color-bg-1: #faf9f7')).toBe(false);
    expect(css.includes('--color-bg-popup: #faf9f7')).toBe(false);
    expect(css.includes('--color-secondary: #f1efea')).toBe(false);
    expect(shellCss.includes(":global([data-theme='light']) .positioner")).toBe(true);
    expect(shellCss.includes(":global([data-theme='dark']) .positioner")).toBe(true);
    expect(shellCss.includes('background: color-mix(in srgb, var(--color-bg-2)')).toBe(true);
    expect(shellCss.includes('background: rgb(var(--primary-6))')).toBe(true);
    expect(shellCss.includes('@media (prefers-color-scheme: dark)')).toBe(false);
    expect(shellCss.includes('width: 580px')).toBe(true);
    expect(promptCss.includes('min-height: 104px')).toBe(true);
    expect(css.includes('height: 160px')).toBe(false);
    expect(shellCss.includes('flex: 0 1 156px')).toBe(true);
    expect(shellCss.includes('min-width: 124px')).toBe(true);
    expect(shellCss.includes('min-width: 48px')).toBe(true);
    expect(shellCss.includes('.footer :global(.i-icon)')).toBe(true);
    expect(shellCss.includes(".positioner[data-placement='above']")).toBe(true);
    expect(shellCss.includes('--creative-canvas-composer-offset-x')).toBe(true);
    expect(shellCss.includes(".positioner[data-overlay='true']")).toBe(true);
    expect(css.includes('.settingsPopover')).toBe(true);
    expect(css.includes('.settingsSelect select')).toBe(true);
    expect(
      /\.sizeMenuOption\s*\{[\s\S]*?justify-content:\s*space-between;/.test(css)
    ).toBe(true);
    expect(css.includes('appearance: none')).toBe(true);
    expect(css.includes('pointer-events: none')).toBe(true);
  });

});
