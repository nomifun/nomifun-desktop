/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./AudioWorkbench.tsx', import.meta.url), 'utf8');
const types = readFileSync(new URL('./types.ts', import.meta.url), 'utf8');
const css = readFileSync(new URL('./AudioWorkbench.module.css', import.meta.url), 'utf8');

describe('AudioWorkbench implementation boundary', () => {
  test('stays fully controlled and delegates model selection through a product slot', () => {
    expect(source.includes('useState')).toBe(false);
    expect(source.includes('useEffect')).toBe(false);
    expect(source.includes('data-audio-model-slot')).toBe(true);
    expect(source.includes('onValueChange({ ...value, ...patch })')).toBe(true);
    expect(types.includes('modelSlot: ReactNode')).toBe(true);
    expect(types.includes('providerId: string')).toBe(true);
    expect(types.includes("taskState: AudioWorkbenchTaskState")).toBe(true);
  });

  test('exposes the complete lifecycle and required outward result callbacks', () => {
    for (const state of ['queued', 'running', 'succeeded', 'failed', 'canceled']) {
      expect(types.includes(`'${state}'`)).toBe(true);
    }
    expect(types.includes('onPlaybackChange(result: AudioWorkbenchSucceededResult')).toBe(true);
    expect(types.includes('onDownloadResult(result: AudioWorkbenchSucceededResult)')).toBe(true);
    expect(types.includes('onInsertResult(result: AudioWorkbenchSucceededResult)')).toBe(true);
    expect(types.includes('onChooseReferences?(): void')).toBe(true);
    expect(types.includes('onRemoveReference(referenceAssetId: string)')).toBe(true);
  });

  test('does not implement transport, create object URLs, or fabricate playable audio', () => {
    for (const forbidden of [
      'fetch(',
      'axios',
      'synthesizeSpeech',
      'URL.createObjectURL',
      'new Audio',
      'new Blob',
      '<audio',
    ]) {
      expect(source.includes(forbidden)).toBe(false);
    }
    expect(source.includes("from '@icon-park/react'")).toBe(true);
    expect(source.includes("from '@arco-design/web-react'")).toBe(true);
  });

  test('keeps the desktop split workspace and compact fallback responsive', () => {
    expect(css.includes('grid-template-columns: minmax(340px, 390px) minmax(0, 1fr)')).toBe(true);
    expect(css.includes('@media (max-width: 960px)')).toBe(true);
    expect(css.includes('@media (max-width: 720px)')).toBe(true);
    expect(css.includes('grid-template-columns: minmax(0, 1fr)')).toBe(true);
    expect(css.includes('linear-gradient')).toBe(false);
    expect(css.includes('radial-gradient')).toBe(false);
  });
});
