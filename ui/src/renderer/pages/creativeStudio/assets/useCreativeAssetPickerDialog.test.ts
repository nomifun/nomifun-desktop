/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';

import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import type { CreativeAssetLibraryPort } from './types';
import { toggleCreativeAssetPickerSelection, useCreativeAssetPickerDialog } from './useCreativeAssetPickerDialog';

afterEach(cleanup);

const pickerSource = readFileSync(
  new URL('./components/CreativeAssetPickerModal.tsx', import.meta.url),
  'utf8'
);
const pickerContentSource = readFileSync(
  new URL('./components/CreativeAssetPickerContent.tsx', import.meta.url),
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
    const source = `${pickerSource}\n${pickerContentSource}`;
    expect(source.includes('<CreativeAssetMedia')).toBe(true);
    expect(pickerSource.includes('alignCenter={false}')).toBe(true);
    expect(source.includes("role='listbox'")).toBe(true);
    expect(pickerSource.includes("role='tablist'")).toBe(false);
    expect(source.includes("t('creativeStudio.assets.picker.searchPlaceholder'")).toBe(
      true
    );
    expect(source.includes("t('creativeStudio.assets.picker.addAsset'")).toBe(true);
    expect(source.includes("role='alert'")).toBe(true);
    expect(source.includes('onRetry')).toBe(true);
    expect(source.includes('onConfirm ?? onCancel')).toBe(true);
    expect(pickerCss.includes('grid-auto-rows: max-content')).toBe(true);
    expect(pickerCss.includes('margin-top: auto')).toBe(true);
    expect(pickerCss.includes('@media (prefers-reduced-motion: reduce)')).toBe(true);
  });

  test('refreshes the authoritative asset list whenever the picker opens', async () => {
    let listCalls = 0;
    const client = {
      list: async () => {
        listCalls += 1;
        return { items: [], total: 0 };
      },
    } as unknown as CreativeAssetLibraryPort;
    const hook = renderHook(() => useCreativeAssetPickerDialog({ client }));
    await waitFor(() => expect(listCalls).toBe(1));

    let pending!: Promise<string[] | null>;
    act(() => {
      pending = hook.result.current.pick({ acceptedKinds: ['image'] });
    });
    await waitFor(() => expect(listCalls).toBe(2));

    act(() => hook.unmount());
    expect(await pending).toBeNull();
  });
});
