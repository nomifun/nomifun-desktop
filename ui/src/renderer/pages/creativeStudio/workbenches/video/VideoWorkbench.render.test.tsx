/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import VideoWorkbenchResults from './VideoWorkbenchResults';
import type { VideoWorkbenchTask } from './types';

const testI18n = createInstance();

await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
  interpolation: { escapeValue: false },
});

const base = {
  prompt: '镜头缓慢推进',
  createdAtLabel: '08/20 14:30',
  model: { providerId: 'provider-a', model: 'video-model' },
  modelLabel: 'Provider A · Video Model',
  resolutionLabel: '1080P',
  sizeLabel: '16:9',
  durationLabel: '6s',
  taskCount: 1,
};

const tasks: VideoWorkbenchTask[] = [
  { ...base, id: 'queued', taskId: 'task-queued', status: 'queued', deletable: false },
  { ...base, id: 'running', taskId: 'task-running', status: 'running', deletable: false },
  {
    ...base,
    id: 'succeeded',
    taskId: 'task-succeeded',
    status: 'succeeded',
    assetId: 'asset-succeeded',
    videoUrl: 'https://media.invalid/video.mp4',
    deletable: true,
  },
  {
    ...base,
    id: 'failed',
    taskId: 'task-failed',
    status: 'failed',
    error: '模型暂时不可用',
    deletable: true,
  },
  {
    ...base,
    id: 'canceled',
    taskId: 'task-canceled',
    status: 'canceled',
    message: '用户已取消任务',
    deletable: true,
  },
];

describe('VideoWorkbench result rendering', () => {
  test('renders every backend state without inventing progress or media', () => {
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <VideoWorkbenchResults
          tasks={tasks}
          selectedTaskIds={[]}
          onSelectedTaskIdsChange={() => undefined}
          onDeleteTasks={() => undefined}
        />
      </I18nextProvider>
    );

    for (const status of ['queued', 'running', 'succeeded', 'failed', 'canceled']) {
      expect(html.includes(`data-video-result-state="${status}"`)).toBe(true);
    }
    expect(html.includes('排队中')).toBe(true);
    expect(html.includes('等待模型开始处理')).toBe(true);
    expect(html.includes('已取消')).toBe(true);
    expect(html.includes('用户已取消任务')).toBe(true);
    expect(html.includes('data-provider-id="provider-a"')).toBe(true);
    expect(html.includes('data-model="video-model"')).toBe(true);
    expect(html.match(/<video/g)?.length).toBe(1);
    expect(html.includes('https://media.invalid/video.mp4')).toBe(true);
    expect(html.includes('data:video')).toBe(false);
    expect(html.includes('当前创作进度')).toBe(false);
    expect(html.includes('选择任务 queued')).toBe(false);
    expect(html.includes('选择任务 running')).toBe(false);
    expect(html.includes('选择任务 succeeded')).toBe(true);
    expect(html.includes('从历史移除 failed')).toBe(true);
  });
});
