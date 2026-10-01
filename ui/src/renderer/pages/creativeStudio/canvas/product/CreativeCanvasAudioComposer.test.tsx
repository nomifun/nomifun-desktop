/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';

import type { CreativeModelOption } from '../../models';
import CreativeCanvasAudioComposer, {
  dispatchCanvasAudioComposerSubmission,
  isCanvasAudioComposerSubmitKey,
  type CreativeCanvasAudioComposerProps,
} from './CreativeCanvasAudioComposer';

const PROVIDER_ID =
  '019b0000-0000-7000-8000-000000000019' as CreativeModelOption['providerId'];
const noop = () => undefined;
afterEach(cleanup);
const speechModel: CreativeModelOption = {
  providerId: PROVIDER_ID,
  model: 'tts-v1',
  providerName: 'Provider A',
  platform: 'openai',
  task: 'speech_synthesis',
  traits: [],
  protocol: 'openai.audio_speech',
};

const props = (
  overrides: Partial<CreativeCanvasAudioComposerProps> = {}
): CreativeCanvasAudioComposerProps => ({
  nodeId: '019b0000-0000-7000-8000-000000000001',
  initialPrompt: '',
  settings: {
    model: { providerId: PROVIDER_ID, model: 'tts-v1' },
    voice: 'alloy',
    format: 'mp3',
  },
  modelOptions: [speechModel],
  task: { state: 'idle', pendingCount: 0 },
  voiceSupported: true,
  voiceRequired: false,
  formatSupported: true,
  onPromptChange: noop,
  onModelChange: noop,
  onVoiceChange: noop,
  onFormatChange: noop,
  onOpenPromptLibrary: noop,
  onGenerate: noop,
  ...overrides,
});

describe('CreativeCanvasAudioComposer', () => {
  test.each(['queued', 'running'] as const)('blocks another generation on the same node while its task is %s', (state) => {
    const generated: string[] = [];
    const componentProps = props({
      initialPrompt: '第一段旁白', task: { state, pendingCount: 1 },
      onGenerate: (prompt) => generated.push(prompt),
    });
    const view = render(withCanvasTestI18n(<CreativeCanvasAudioComposer {...componentProps} />));
    const input = view.getByRole('textbox', { name: '朗读文本' }) as HTMLTextAreaElement;
    const button = view.getByRole('button', { name: '生成音频' }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(input.disabled).toBe(false);
    fireEvent.click(button);
    fireEvent.change(input, { target: { value: '第二段旁白' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(generated).toEqual([]);
    view.rerender(withCanvasTestI18n(<CreativeCanvasAudioComposer {...componentProps}
      task={{ state: 'succeeded', pendingCount: 0 }} />));
    expect(button.disabled).toBe(false);
    fireEvent.click(button);
    expect(generated).toEqual(['第二段旁白']);
  });

  test('keeps another node available while this node is running', () => {
    const generated: string[] = [];
    const firstNode = props({
      nodeId: 'first-node', initialPrompt: '第一段旁白',
      task: { state: 'running', pendingCount: 1 },
      onGenerate: (prompt) => generated.push(prompt),
    });
    const secondNode = props({
      nodeId: 'second-node', initialPrompt: '第二段旁白',
      onGenerate: (prompt) => generated.push(prompt),
    });
    const view = render(withCanvasTestI18n(<>
      <CreativeCanvasAudioComposer {...firstNode} />
      <CreativeCanvasAudioComposer {...secondNode} />
    </>));
    const buttons = view.getAllByRole('button', { name: '生成音频' }) as HTMLButtonElement[];
    expect(buttons[0]!.disabled).toBe(true);
    expect(buttons[1]!.disabled).toBe(false);
    fireEvent.click(buttons[1]!);
    expect(generated).toEqual(['第二段旁白']);
  });

  test('renders a focused speech synthesis composer', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasAudioComposer
        {...props({ initialPrompt: '欢迎来到 NomiFun。' })}
      />
    );
    expect(html.includes('data-canvas-audio-composer="true"')).toBe(true);
    expect(html.includes('data-voice-profile="optional"')).toBe(true);
    expect(html.includes('朗读文本')).toBe(true);
    expect(html.includes('输入要朗读的文本')).toBe(true);
    expect(html.includes('打开朗读提示词库')).toBe(true);
    expect(html.includes('语音合成模型')).toBe(true);
    expect(html.includes('语音生成设置')).toBe(true);
    expect(html.includes('MP3 · alloy')).toBe(true);
    expect(html.includes('aria-label="生成音频"')).toBe(true);
  });

  test('does not expose optional settings without an explicit protocol profile', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasAudioComposer
        {...props({
          voiceSupported: false,
          voiceRequired: false,
          formatSupported: false,
        })}
      />
    );
    expect(html.includes('data-voice-profile="unsupported"')).toBe(true);
    expect(html.includes('语音生成设置')).toBe(false);
    expect(html.includes('MP3 · alloy')).toBe(false);
  });

  test('requires a provider voice only when the resolved profile says so', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasAudioComposer
        {...props({
          settings: { ...props().settings, voice: '' },
          voiceRequired: true,
        })}
      />
    );
    expect(html.includes('data-voice-profile="required"')).toBe(true);
    expect(html.includes('MP3 · 待填写音色')).toBe(true);
    expect(html.includes('当前协议要求填写 provider Voice ID。')).toBe(true);
    expect(html.includes('aria-label="生成音频" disabled')).toBe(true);
  });

  test('filters out every model that is not exact speech synthesis', () => {
    const videoModel: CreativeModelOption = {
      ...speechModel,
      model: 'video-v1',
      task: 'video_generation',
      protocol: 'test.video_generation',
    };
    const html = renderToStaticMarkup(
      <CreativeCanvasAudioComposer
        {...props({
          settings: {
            ...props().settings,
            model: { providerId: PROVIDER_ID, model: 'video-v1' },
          },
          modelOptions: [videoModel],
        })}
      />
    );
    expect(html.includes('没有可用的语音合成模型')).toBe(true);
    expect(html.includes('aria-label="生成音频" disabled')).toBe(true);
    expect(html.includes('video-v1 · Provider A')).toBe(false);
  });

  test('enforces the profile text limit without truncating restored content', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasAudioComposer
        {...props({ initialPrompt: '1234', maxTextLength: 3 })}
      />
    );
    expect(html.includes('maxLength="3"')).toBe(true);
    expect(html.includes('朗读文本不能超过 3 个字符。')).toBe(true);
    expect(html.includes('aria-label="生成音频" disabled')).toBe(true);
  });

  test('offers same-key retry and explicit status confirmation together', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasAudioComposer
        {...props({
          initialPrompt: '',
          retrySubmission: true,
          error: '任务提交结果尚未确认',
          onRetrySubmission: noop,
          onConfirmSubmission: noop,
        })}
      />
    );
    expect(html.includes('任务提交结果尚未确认')).toBe(true);
    expect(html.includes('aria-label="同键重试音频任务"')).toBe(true);
    expect(html.includes('同键重试')).toBe(true);
    expect(html.includes('确认任务状态')).toBe(true);
    expect(html.includes('aria-label="同键重试音频任务" disabled')).toBe(
      false
    );
  });

  test('dispatches trimmed generation and idempotent retry callbacks', () => {
    const generated: string[] = [];
    let retries = 0;
    expect(
      dispatchCanvasAudioComposerSubmission({
        disabled: false,
        busy: false,
        prompt: '  今天天气很好。  ',
        hasModel: true,
        requiredVoiceReady: true,
        promptLengthReady: true,
        retrySubmission: false,
        onGenerate: (prompt) => generated.push(prompt),
      })
    ).toBe('generated');
    expect(generated).toEqual(['今天天气很好。']);

    expect(
      dispatchCanvasAudioComposerSubmission({
        disabled: false,
        busy: false,
        prompt: '',
        hasModel: false,
        requiredVoiceReady: false,
        promptLengthReady: false,
        retrySubmission: true,
        onGenerate: (prompt) => generated.push(prompt),
        onRetrySubmission: () => {
          retries += 1;
        },
      })
    ).toBe('retried');
    expect(retries).toBe(1);
    expect(generated).toEqual(['今天天气很好。']);
  });

  test('submits on Enter while preserving Shift+Enter for a newline', () => {
    expect(isCanvasAudioComposerSubmitKey('Enter', false)).toBe(true);
    expect(isCanvasAudioComposerSubmitKey('Enter', true)).toBe(false);
    expect(isCanvasAudioComposerSubmitKey('a', false)).toBe(false);
  });

  test('wires only the supported first audio settings', () => {
    const component = readFileSync(
      new URL('./CreativeCanvasAudioComposer.tsx', import.meta.url),
      'utf8'
    );
    expect(component.includes("['mp3', 'wav']")).toBe(true);
    expect(component.includes('{voiceVisible ? (')).toBe(true);
    expect(component.includes('{formatVisible ? (')).toBe(true);
    expect(component.includes('maxLength={256}')).toBe(true);
    for (const callback of [
      'onPromptChange',
      'onModelChange',
      'onVoiceChange',
      'onFormatChange',
      'onOpenPromptLibrary',
      'onGenerate',
      'onRetrySubmission',
      'onConfirmSubmission',
    ]) {
      expect(component.includes(callback)).toBe(true);
    }
    for (const unsupported of [
      'speed',
      'instructions',
      'referenceAudio',
      'VoiceClone',
      "'aac'",
      "'pcm'",
      'credits',
    ]) {
      expect(component.includes(unsupported)).toBe(false);
    }
  });

  test('uses the shared compact shell and keeps safe audio settings controls', () => {
    const shellCss = readFileSync(
      new URL('./CreativeCanvasComposerShell.module.css', import.meta.url),
      'utf8'
    );
    expect(shellCss.includes(":global([data-theme='light']) .positioner")).toBe(true);
    expect(shellCss.includes(":global([data-theme='dark']) .positioner")).toBe(true);
    expect(shellCss.includes('height: 92px')).toBe(true);
    expect(shellCss.includes('min-width: 48px')).toBe(true);
    expect(shellCss.includes('.retrySubmitButton')).toBe(true);
    expect(shellCss.includes('.popoverSettingsPanel')).toBe(true);
    expect(shellCss.includes('.settingsControl')).toBe(true);
  });
});
