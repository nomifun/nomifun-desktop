/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import AudioWorkbench from './AudioWorkbench';
import type { AudioWorkbenchProps, AudioWorkbenchResult, AudioWorkbenchTaskState } from './types';

const results: readonly AudioWorkbenchResult[] = [
  { id: 'queued', taskId: 'task-queued', status: 'queued', title: '等待中的旁白', text: '稍后开始处理' },
  { id: 'running', taskId: 'task-running', status: 'running', title: '品牌开场', progress: 48, statusLabel: '正在合成' },
  {
    id: 'succeeded',
    taskId: 'task-succeeded',
    status: 'succeeded',
    title: '产品旁白 01',
    text: '欢迎来到 NomiFun 创意工坊。',
    assetId: 'audio-asset-01',
    modelLabel: 'OpenAI · gpt-4o-mini-tts',
    formatLabel: 'MP3',
    durationMs: 12_400,
    sizeBytes: 1_572_864,
    createdAtLabel: '刚刚',
  },
  { id: 'failed', taskId: 'task-failed', status: 'failed', title: '失败的版本', errorMessage: '供应商额度不足' },
  { id: 'canceled', taskId: 'task-canceled', status: 'canceled', title: '已取消的版本', message: '用户取消了任务' },
];

const baseProps: AudioWorkbenchProps = {
  value: {
    text: '欢迎来到 NomiFun 创意工坊。',
    instructions: '自然、温暖，适合产品旁白。',
    voice: 'alloy',
    format: 'mp3',
    speed: 1,
    model: { providerId: 'provider-openai', model: 'gpt-4o-mini-tts' },
  },
  modelSlot: <button data-speech-model-slot='true'>统一模型选择器</button>,
  voiceOptions: [
    { value: 'alloy', label: 'Alloy' },
    { value: 'nova', label: 'Nova' },
  ],
  formatOptions: [
    { value: 'mp3', label: 'MP3' },
    { value: 'wav', label: 'WAV' },
  ],
  references: [
    {
      assetId: 'reference-01',
      name: '品牌主理人参考.wav',
      mimeType: 'audio/wav',
      durationMs: 8_000,
      sizeBytes: 512_000,
    },
  ],
  results,
  task: { state: 'succeeded', message: '已生成 1 个音频结果' },
  playingResultId: 'succeeded',
  onValueChange: () => undefined,
  onChooseReferences: () => undefined,
  onRemoveReference: () => undefined,
  onGenerate: () => undefined,
  onCancel: () => undefined,
  onRetry: () => undefined,
  onPlaybackChange: () => undefined,
  onDownloadResult: () => undefined,
  onInsertResult: () => undefined,
  onRetryResult: () => undefined,
};

const renderWorkbench = (overrides: Partial<AudioWorkbenchProps> = {}) =>
  renderToStaticMarkup(<AudioWorkbench {...baseProps} {...overrides} />);

describe('AudioWorkbench presentation', () => {
  test('renders the controlled composer, injected model slot and real references', () => {
    const html = renderWorkbench();

    expect(html.includes('data-audio-workbench="true"')).toBe(true);
    expect(html.includes('data-audio-workbench-composer="true"')).toBe(true);
    expect(html.includes('data-audio-model-slot="true"')).toBe(true);
    expect(html.includes('data-speech-model-slot="true"')).toBe(true);
    expect(html.includes('朗读文本')).toBe(true);
    expect(html.includes('模型与声音')).toBe(true);
    expect(html.includes('声音指令')).toBe(true);
    expect(html.includes('品牌主理人参考.wav')).toBe(true);
    expect(html.includes('audio/wav')).toBe(true);
    expect(html.includes('0:08')).toBe(true);
    expect(html.includes('当前模型协议未声明语速参数')).toBe(true);
    expect(html.includes('当前模型协议未声明声音指令能力')).toBe(true);
    expect(html.includes('当前模型未声明参考音频能力')).toBe(true);
  });

  test('renders every task/result state and succeeded-result actions without audio placeholders', () => {
    const html = renderWorkbench();

    for (const state of ['queued', 'running', 'succeeded', 'failed', 'canceled']) {
      expect(html.includes(`data-audio-result-state="${state}"`)).toBe(true);
    }
    expect(html.includes('正在合成')).toBe(true);
    expect(html.includes('供应商额度不足')).toBe(true);
    expect(html.includes('用户取消了任务')).toBe(true);
    expect(html.includes('暂停')).toBe(true);
    expect(html.includes('aria-label="下载 产品旁白 01"')).toBe(true);
    expect(html.includes('插入画布')).toBe(true);
    expect(html.includes('<audio')).toBe(false);
    expect(html.includes('blob:')).toBe(false);
  });

  test('surfaces queued/running/succeeded/failed/canceled task summaries', () => {
    const scenarios: Array<[AudioWorkbenchTaskState, string]> = [
      ['queued', '排队中'],
      ['running', '生成中'],
      ['succeeded', '生成完成'],
      ['failed', '生成失败'],
      ['canceled', '已取消'],
    ];

    for (const [state, label] of scenarios) {
      const html = renderWorkbench({
        task: {
          state,
          progress: state === 'queued' || state === 'running' ? 35 : undefined,
          errorMessage: state === 'failed' ? '模型调用失败' : undefined,
        },
      });
      expect(html.includes(`data-audio-task-state="${state}"`)).toBe(true);
      expect(html.includes(label)).toBe(true);
    }
  });

  test('renders an honest empty result state when no assets were supplied', () => {
    const html = renderWorkbench({ results: [], references: [], task: { state: 'idle' } });

    expect(html.includes('data-audio-results="empty"')).toBe(true);
    expect(html.includes('还没有音频结果')).toBe(true);
    expect(html.includes('没有参考音频')).toBe(true);
  });
});
