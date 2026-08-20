/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const root = readFileSync(new URL('./VideoWorkbench.tsx', import.meta.url), 'utf8');
const composer = readFileSync(new URL('./VideoWorkbenchComposer.tsx', import.meta.url), 'utf8');
const results = readFileSync(new URL('./VideoWorkbenchResults.tsx', import.meta.url), 'utf8');
const types = readFileSync(new URL('./types.ts', import.meta.url), 'utf8');

describe('VideoWorkbench controlled boundary', () => {
  test('owns only presentation and switches controlled layouts', () => {
    expect(root.includes("props.layout === 'side'")).toBe(true);
    expect(root.includes("props.layout === 'bottom'")).toBe(true);
    expect(types.includes('onLayoutChange: (layout: VideoWorkbenchLayout) => void')).toBe(true);
    expect(types.includes('onPromptChange: (value: string) => void')).toBe(true);
    expect(types.includes('modelSlot: ReactNode')).toBe(true);
  });

  test('contains no API, persistence, model-name heuristics or retired-workshop dependency', () => {
    const combined = `${root}\n${composer}\n${results}`;
    expect(combined.includes('ipcBridge')).toBe(false);
    expect(combined.includes('fetch(')).toBe(false);
    expect(combined.includes('localStorage')).toBe(false);
    expect(combined.includes('pages/workshop')).toBe(false);
    expect(combined.includes('.includes(model')).toBe(false);
  });

  test('requires real media for successful results and never ships sample URLs', () => {
    expect(types.includes("status: 'succeeded'")).toBe(true);
    expect(types.includes('assetId: string')).toBe(true);
    expect(types.includes('videoUrl: string')).toBe(true);
    expect(results.includes('<video')).toBe(true);
    expect(results.includes('data:video')).toBe(false);
    expect(results.includes('placehold')).toBe(false);
  });

  test('exposes the requested result states and task selection/deletion', () => {
    for (const status of ['queued', 'running', 'succeeded', 'failed', 'canceled']) {
      expect(types.includes(`status: '${status}'`)).toBe(true);
    }
    expect(results.includes("data-video-result-state='empty'")).toBe(true);
    expect(results.includes('data-video-result-state={task.status}')).toBe(true);
    expect(results.includes('toggleVideoTaskSelection')).toBe(true);
    expect(results.includes('toggleAllVideoTasks')).toBe(true);
    expect(results.includes('onDeleteTasks(visibleSelectedIds)')).toBe(true);
    expect(results.includes('onDeleteTasks([task.id])')).toBe(true);
    expect(results.includes("if (task.status === 'queued') return '排队中'")).toBe(true);
    expect(results.includes("if (task.status === 'canceled') return <CanceledVisual task={task} />")).toBe(true);
  });

  test('preserves exact model identity separately from display labels', () => {
    expect(types.includes('model: VideoWorkbenchModelIdentity')).toBe(true);
    expect(types.includes('providerId: string')).toBe(true);
    expect(types.includes('model: string')).toBe(true);
    expect(results.includes('data-provider-id={task.model.providerId}')).toBe(true);
    expect(results.includes('data-model={task.model.model}')).toBe(true);
  });

  test('keeps references, task parameters and advanced parameters behind callbacks/slots', () => {
    expect(types.includes('onAddReferences: () => void')).toBe(true);
    expect(types.includes('onRemoveReference: (referenceId: string) => void')).toBe(true);
    expect(types.includes('onResolutionChange: (value: string) => void')).toBe(true);
    expect(types.includes('onSizeChange: (value: string) => void')).toBe(true);
    expect(types.includes('onDurationChange: (value: string) => void')).toBe(true);
    expect(types.includes('onTaskCountChange: (value: number) => void')).toBe(true);
    expect(types.includes('onOpenParameters: () => void')).toBe(true);
  });
});
