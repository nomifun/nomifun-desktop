/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import { toggleCreativeAssetPickerSelection } from './useCreativeAssetPickerDialog';

const pickerSource = readFileSync(
  new URL('./components/CreativeAssetPickerModal.tsx', import.meta.url),
  'utf8'
);
const pickerCss = readFileSync(
  new URL('./components/CreativeAssetPickerModal.module.css', import.meta.url),
  'utf8'
);

describe('Creative asset picker dialog', () => {
  test('keeps single and bounded multi-selection deterministic', () => {
    expect(toggleCreativeAssetPickerSelection([], 'asset-a', 1)).toEqual(['asset-a']);
    expect(toggleCreativeAssetPickerSelection(['asset-a'], 'asset-b', 1)).toEqual(['asset-b']);
    expect(toggleCreativeAssetPickerSelection(['asset-a'], 'asset-a', 1)).toEqual([]);
    expect(toggleCreativeAssetPickerSelection(['asset-a'], 'asset-b', 2)).toEqual([
      'asset-a',
      'asset-b',
    ]);
    expect(toggleCreativeAssetPickerSelection(['asset-a', 'asset-b'], 'asset-c', 2)).toEqual([
      'asset-a',
      'asset-b',
    ]);
  });

  test('uses real asset media and exposes loading, error, retry and completion states', () => {
    expect(pickerSource.includes('<CreativeAssetMedia')).toBe(true);
    expect(pickerSource.includes('alignCenter={false}')).toBe(true);
    expect(pickerSource.includes("role='listbox'")).toBe(true);
    expect(pickerSource.includes("role='tablist'")).toBe(true);
    expect(pickerSource.includes("t('creativeStudio.assets.picker.searchPlaceholder'")).toBe(
      true
    );
    expect(pickerSource.includes("t('creativeStudio.assets.picker.addAsset'")).toBe(true);
    expect(pickerSource.includes("role='alert'")).toBe(true);
    expect(pickerSource.includes('onRetry')).toBe(true);
    expect(pickerSource.includes('onConfirm ?? onCancel')).toBe(true);
    expect(pickerCss.includes('@media (max-width: 620px)')).toBe(true);
    expect(pickerCss.includes('@media (prefers-reduced-motion: reduce)')).toBe(true);
  });
});
