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
const css = readFileSync(new URL('./VideoWorkbench.module.css', import.meta.url), 'utf8');
const productRoute = readFileSync(
  new URL('../product/VideoWorkbenchProductRoute.tsx', import.meta.url),
  'utf8'
);

describe('VideoWorkbench controlled boundary', () => {
  test('owns only presentation and switches controlled layouts', () => {
    expect(root.includes("props.layout === 'side'")).toBe(true);
    expect(root.includes("props.layout === 'bottom'")).toBe(true);
    expect(types.includes('onLayoutChange: (layout: VideoWorkbenchLayout) => void')).toBe(true);
    expect(types.includes('onPromptChange: (value: string) => void')).toBe(true);
    expect(types.includes('modelSlot: ReactNode')).toBe(true);
  });

  test('matches the compact image-workbench header rhythm', () => {
    expect(/\.composerHeader\s*\{[\s\S]*?min-height:\s*64px;[\s\S]*?padding:\s*12px 14px;/.test(css)).toBe(true);
    expect(/\.composerHeader strong\s*\{[\s\S]*?font-size:\s*14px;[\s\S]*?line-height:\s*18px;/.test(css)).toBe(true);
    expect(/\.layoutSwitch button\s*\{[\s\S]*?height:\s*28px;[\s\S]*?font-size:\s*12px;/.test(css)).toBe(true);
    expect(/\.sideComposerBody\s*\{[\s\S]*?padding:\s*12px 16px 16px;/.test(css)).toBe(true);
    expect(/\.resultsHeader\s*\{[\s\S]*?min-height:\s*64px;[\s\S]*?margin-bottom:\s*0;[\s\S]*?padding:\s*12px 16px;/.test(css)).toBe(true);
    expect(css.includes('min-height: 62px')).toBe(false);
    expect(/\.resultsTitle h2\s*\{[\s\S]*?font-size:\s*14px;[\s\S]*?line-height:\s*20px;/.test(css)).toBe(true);
    expect(/\.layoutSwitch button > :global\(\.i-icon\)[\s\S]*?line-height:\s*0;/.test(css)).toBe(true);
    expect(results.includes("<History size={15} />")).toBe(true);
    expect(results.includes("<Tag size='small' bordered={false}>")).toBe(true);
    expect(/\.emptyResults\s*\{[\s\S]*?margin:\s*0 18px 18px;/.test(css)).toBe(true);
  });

  test('uses the shared two-pane bottom-composer rhythm without dropping video controls', () => {
    expect(composer.includes('className={styles.bottomComposerBody}')).toBe(true);
    expect(composer.includes('className={styles.bottomActionRow}')).toBe(true);
    expect(composer.includes('<SettingsGrid {...settings} compact />')).toBe(true);
    expect(
      /\.bottomComposerBody\s*\{[\s\S]*?grid-template-columns:\s*minmax\(330px, 0\.86fr\) minmax\(0, 1\.14fr\);/.test(
        css
      )
    ).toBe(true);
    expect(
      /\.compactSettingsGrid\s*\{[\s\S]*?grid-template-columns:\s*repeat\(12, minmax\(0, 1fr\)\);/.test(
        css
      )
    ).toBe(true);
    expect(composer.indexOf('styles.bottomGenerateButton')).toBeLessThan(
      composer.indexOf('styles.bottomTools')
    );
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
    expect(results.includes('<CreativeVideoPlayer')).toBe(true);
    expect(results.includes('data:video')).toBe(false);
    expect(results.includes('placehold')).toBe(false);
  });

  test('exposes result states while gating deletion behind a real capability', () => {
    for (const status of ['queued', 'running', 'succeeded', 'failed', 'canceled']) {
      expect(types.includes(`status: '${status}'`)).toBe(true);
    }
    expect(results.includes("data-video-result-state='empty'")).toBe(true);
    expect(results.includes('data-video-result-state={task.status}')).toBe(true);
    expect(results.includes('toggleVideoTaskSelection')).toBe(true);
    expect(results.includes('toggleAllVideoTasks')).toBe(true);
    expect(results.includes('onDeleteTasks?.(visibleSelectedIds)')).toBe(true);
    expect(results.includes('onDeleteTasks?.([task.id])')).toBe(true);
    expect(results.includes('deletionEnabled')).toBe(true);
    expect(results.includes('加载更多历史')).toBe(true);
    expect(results.includes('onCancelTask(task.id)')).toBe(true);
    expect(results.includes("'creativeStudio.video.task.queued'")).toBe(true);
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
    expect(productRoute.includes('Message.info({')).toBe(true);
    expect(productRoute.includes('duration: 2600')).toBe(true);
    expect(productRoute.includes("position: 'top'")).toBe(true);
    expect(composer.includes("'creativeStudio.video.settings.aspectRatio'")).toBe(true);
    expect(productRoute.includes('sizeOptions: ASPECTS')).toBe(true);
    expect(productRoute.includes('videoWorkbenchSizeOptionLabel')).toBe(false);
    expect(
      productRoute.includes("onOpenParameters: () =>\n      setError(")
    ).toBe(false);
  });
});
