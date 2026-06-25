/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('GuidPage advanced controls', () => {
  test('keeps supported draft controls but hides the multi-agent entry', () => {
    const source = readSource(new URL('./GuidPage.tsx', import.meta.url));

    expect(source.includes('<AutoWorkControl')).toBe(true);
    expect(source.includes('<IdmmControl')).toBe(true);
    expect(source.includes('<KnowledgeControl')).toBe(true);
    expect(source.includes('MultiAgentControl')).toBe(false);
  });

  test('does not persist multi-agent draft config from the session creation page', () => {
    const source = readSource(new URL('./hooks/useGuidAdvancedConfig.ts', import.meta.url));

    expect(source.includes('multi_agent')).toBe(false);
    expect(source.includes('multiAgent')).toBe(false);
    expect(source.includes('setMultiAgent')).toBe(false);
  });
});
