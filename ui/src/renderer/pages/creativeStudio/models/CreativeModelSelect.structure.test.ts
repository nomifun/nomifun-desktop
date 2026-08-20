/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const component = readFileSync(new URL('./CreativeModelSelect.tsx', import.meta.url), 'utf8');
const adapter = readFileSync(new URL('./useNomiCreativeModelCatalog.ts', import.meta.url), 'utf8');

describe('CreativeModelSelect integration boundary', () => {
  test('the view is controlled and does not own provider fetching', () => {
    expect(component.includes('catalog: CreativeModelCatalogSnapshot')).toBe(true);
    expect(component.includes('value: CreativeModelSelectionRef | null')).toBe(true);
    expect(component.includes('onChange: (selection: CreativeModelOption) => void')).toBe(true);
    expect(component.includes('useProvidersQuery')).toBe(false);
  });

  test('the NomiFun adapter is the only provider-query connection', () => {
    expect(adapter.includes('useProvidersQuery()')).toBe(true);
    expect(adapter.includes('adaptCreativeModelCatalog')).toBe(true);
    expect(adapter.includes('fetch(')).toBe(false);
  });

  test('all required view states and a disabled stale selection are explicit', () => {
    for (const state of [
      'loading',
      'no-provider',
      'no-compatible-model',
      'disabled',
      'error',
      'ready',
    ]) {
      expect(component.includes(`'${state}'`)).toBe(true);
    }
    expect(component.includes('<NomiSelect.Option value={optionKey(value)} disabled>')).toBe(true);
    expect(component.includes("role={status === 'error' ? 'alert' : 'status'}")).toBe(true);
  });
});
