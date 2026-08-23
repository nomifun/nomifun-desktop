/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const image = readFileSync(new URL('./ImageWorkbenchProductRoute.tsx', import.meta.url), 'utf8');
const video = readFileSync(new URL('./VideoWorkbenchProductRoute.tsx', import.meta.url), 'utf8');
const ownership = readFileSync(new URL('./ownership.ts', import.meta.url), 'utf8');
const shared = readFileSync(new URL('./shared.tsx', import.meta.url), 'utf8');
const wiring = readFileSync(new URL('./WIRING.md', import.meta.url), 'utf8');
const css = readFileSync(new URL('./StandaloneWorkbenchProduct.module.css', import.meta.url), 'utf8');

describe('standalone workbench product wiring', () => {
  test('exports prop-free product routes composed from the source-parity views', () => {
    expect(image.includes('const ImageWorkbenchProductRoute: React.FC = () =>')).toBe(true);
    expect(image.includes('<ImageWorkbench {...props} modelSlot={modelSlot} />')).toBe(true);
    expect(video.includes('const VideoWorkbenchProductRoute: React.FC = () =>')).toBe(true);
    expect(video.includes('<VideoWorkbench {...props} />')).toBe(true);
  });

  test('fails closed without scope and wires owner-scoped history recovery', () => {
    expect(ownership.includes("if (values.length === 0) return { state: 'missing'")).toBe(true);
    expect(image.includes("owner: standaloneWorkbenchOwner(projectId, 'image')")).toBe(true);
    expect(video.includes("owner: standaloneWorkbenchOwner(projectId, 'video')")).toBe(true);
    expect(image.includes('useStandaloneWorkbenchHistory')).toBe(true);
    expect(video.includes('standaloneHistoryResumeRequests')).toBe(true);
    expect(image.includes('useStandalonePersistence')).toBe(false);
    expect(video.includes('ensureStandaloneWorkbenchNode')).toBe(false);
    expect(shared.includes('navigate(CREATIVE_STUDIO_PROJECTS_PATH)')).toBe(true);
    expect(shared.includes("navigate('/workshop/projects')")).toBe(false);
    expect(wiring.includes('never borrows or creates a recent project implicitly')).toBe(true);
  });

  test('documents the durable owner foundation and coordinated route blocker', () => {
    expect(video.includes('STANDALONE_VIDEO_MAX_CONCURRENT_TASKS')).toBe(true);
    expect(
      wiring.includes('standalone_workbench { projectId, workbenchKind }')
    ).toBe(true);
    expect(wiring.includes('{ assetId, kind, role }')).toBe(true);
    expect(wiring.includes('inputs: null')).toBe(true);
    expect(wiring.includes('owner-scoped list/recovery')).toBe(true);
  });

  test('does not disguise asset deletion or hidden React ids as task-history deletion', () => {
    expect(image.includes('creativeAssetClient.remove')).toBe(false);
    expect(video.includes('creativeAssetClient.remove')).toBe(false);
    expect(image.includes('hiddenResultIds')).toBe(false);
    expect(video.includes('hiddenTaskIds')).toBe(false);
  });

  test('retires only terminal history through the exact backend command', () => {
    expect(image.includes('creativeTaskHistoryClient.retireStandalone')).toBe(true);
    expect(video.includes('creativeTaskHistoryClient.retireStandalone')).toBe(true);
    expect(image.includes('StandaloneHistoryRetireDialog')).toBe(true);
    expect(video.includes('StandaloneHistoryRetireDialog')).toBe(true);
    expect(wiring.includes('POST /api/creative-studio/tasks/retire')).toBe(true);
    expect(wiring.includes('Retirement never deletes media')).toBe(true);
  });

  test('keeps the focused creation palette independent from the app theme', () => {
    expect(css.includes('--color-bg-1: #f4f2ed')).toBe(true);
    expect(css.includes('--color-text-1: #292524')).toBe(true);
    expect(css.includes('--primary-6: 87, 83, 78')).toBe(true);
    expect(css.includes("[data-theme='dark']")).toBe(false);
  });

  test('keeps recovery retryable and fences stale task-load hydration', () => {
    expect(image.includes('重试任务同步')).toBe(true);
    expect(video.includes('重试任务同步')).toBe(true);
    expect(image.includes('loadGenerationRef.current !== generation')).toBe(true);
    expect(video.includes('loadGenerationRef.current !== generation')).toBe(true);
    expect(image.includes('creativeTaskClient.cancel(creativeTaskReference(task))')).toBe(true);
    expect(video.includes('creativeTaskClient.cancel(creativeTaskReference(task))')).toBe(true);
  });

  test('clears exact standalone selections when their catalog model disappears', () => {
    expect(image.includes('exactWorkbenchModelOptions(catalog, modelTask).some')).toBe(true);
    expect(video.includes("exactWorkbenchModelOptions(catalog, 'video_generation').some")).toBe(true);
    expect(video.includes('if (!stillAvailable) setModel(null)')).toBe(true);
  });

  test('gives the image workbench explicit catalog states and exact model-management routes', () => {
    expect(image.includes('<CreativeModelSelect')).toBe(true);
    expect(image.includes("filter={{ capability: 'task', task: modelTask }}")).toBe(true);
    expect(image.includes('IMAGE_GENERATION_MODEL_COPY')).toBe(true);
    expect(image.includes('IMAGE_EDIT_MODEL_COPY')).toBe(true);
    expect(image.includes("modelTask === 'image_edit' ? 'image-edit' : 'image'")).toBe(true);
    expect(image.includes('<ImageWorkbench {...props} modelSlot={modelSlot} />')).toBe(true);
  });
});
