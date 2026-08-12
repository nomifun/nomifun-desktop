/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./ModelAdvancedEditor.tsx', import.meta.url), 'utf8');

describe('existing-model advanced editor modal shell', () => {
  test('uses the responsive modal shell instead of a fixed-width popover', () => {
    expect(source.includes("import NomiModal from '@/renderer/components/base/NomiModal'")).toBe(true);
    expect(source.includes('<NomiModal')).toBe(true);
    expect(source.includes("maxWidth: '94vw'")).toBe(true);
    expect(source.includes("maxHeight: 'calc(92vh - 160px)',")).toBe(true);
    expect(source.includes('unmountOnExit')).toBe(true);
    expect(source.includes('maskClosable={!saving}')).toBe(true);
    expect(source.includes('escToExit={!saving}')).toBe(true);
    expect(source.includes('<Popover')).toBe(false);
    expect(source.includes('w-680px')).toBe(false);
    expect(source.includes('data-model-capability-popover')).toBe(false);
  });
});
