/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = (fileName: string): string =>
  readFileSync(new URL(fileName, import.meta.url), 'utf8');

describe('per-capability output limit editor wiring', () => {
  test('renders and serializes the preset and unit-aware output limit control', () => {
    const editorSource = source('./ModelDefinitionEditor.tsx');
    const advancedSource = source('./providerModelAdvanced.ts');
    const inputSource = source('./OutputLimitInput.tsx');

    expect(editorSource.includes('<OutputLimitInput')).toBe(true);
    expect(editorSource.includes('value={capability.outputLimit}')).toBe(true);
    expect(editorSource.includes('descriptor?.requires_output_ceiling')).toBe(true);
    expect(advancedSource.includes('output_limit: capability.outputLimit')).toBe(true);
    expect(inputSource.includes('settings.outputLimitProviderDefault')).toBe(true);
    expect(inputSource.includes('OUTPUT_LIMIT_PRESETS')).toBe(true);
    expect(inputSource.includes('OUTPUT_LIMIT_UNIT_MULTIPLIERS')).toBe(true);
    expect(inputSource.includes('settings.outputLimitConverted')).toBe(true);
  });

  test('passes only declared provider output windows through every catalog entry point', () => {
    const hookSource=source('../../../hooks/agent/useModeModeList.ts');
    expect(hookSource.includes('outputLimit: model.output_limit')).toBe(true);
    for(const relative of ['./AddPlatformModal.tsx','./AddModelModal.tsx']) {
      expect(source(relative).includes('outputLimit: model.outputLimit')).toBe(true);
    }
    expect(source('./ModelDefinitionEditor.tsx').includes('outputLimit: profile.outputLimit')).toBe(true);
  });
});
