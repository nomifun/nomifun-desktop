/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./CreateCreativeTextAssetModal.tsx', import.meta.url), 'utf8');

describe('CreateCreativeTextAssetModal contract', () => {
  test('keeps every form value and async state controlled by its owner', () => {
    for (const contract of [
      'value: CreativeTextAssetFormValue',
      'submitting?: boolean',
      'error?: string | null',
      'onChange: (value: CreativeTextAssetFormValue) => void',
      'onSubmit: (value: CreativeTextAssetFormValue) => void',
    ]) {
      expect(source.includes(contract)).toBe(true);
    }
    expect(source.includes('useState(')).toBe(false);
  });

  test('includes title content collection tags and library fields without transport calls', () => {
    for (const field of ['value.title', 'value.textContent', 'value.collection', 'value.tags', 'value.inLibrary']) {
      expect(source.includes(field)).toBe(true);
    }
    expect(source.includes('httpRequest(')).toBe(false);
    expect(source.includes('fetch(')).toBe(false);
    expect(source.includes('useCreativeAssets(')).toBe(false);
  });
});
