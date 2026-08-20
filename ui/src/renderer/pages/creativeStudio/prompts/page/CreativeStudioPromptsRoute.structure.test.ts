/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const routeSource = readFileSync(
  new URL('./CreativeStudioPromptsRoute.tsx', import.meta.url),
  'utf8'
);
const selectionSource = readFileSync(
  new URL('./standaloneSelection.ts', import.meta.url),
  'utf8'
);

describe('Creative Studio prompt-library route wiring', () => {
  test('composes the existing library with real NomiFun presets and text assets', () => {
    expect(routeSource.includes('<PromptLibraryPage')).toBe(true);
    expect(routeSource.includes('createNomiPromptLibraryPort({ locale, assets: assetPort })')).toBe(true);
    expect(routeSource.includes('assetPort = creativeAssetClient')).toBe(true);
    expect(routeSource.includes('onSelect={selectPrompt}')).toBe(true);
  });

  test('opens details and copies without pretending the standalone route inserts into a canvas', () => {
    expect(routeSource.includes('<PromptLibraryDetails')).toBe(true);
    expect(routeSource.includes('copyStandalonePrompt(selected, writeClipboardText)')).toBe(true);
    expect(routeSource.includes('独立提示词库不会修改任何画布')).toBe(false);
    expect(routeSource.includes('onInsert=')).toBe(false);
    expect(selectionSource.includes('await writeText(item.prompt)')).toBe(true);
    expect(selectionSource.includes('toPromptLibrarySelection(item)')).toBe(true);
  });

  test('does not create fake data, persistence, or a duplicate transport', () => {
    for (const forbidden of ['localStorage', 'sessionStorage', 'fetch(', 'axios', 'PRESET_ITEMS']) {
      expect(routeSource.includes(forbidden)).toBe(false);
      expect(selectionSource.includes(forbidden)).toBe(false);
    }
  });
});
