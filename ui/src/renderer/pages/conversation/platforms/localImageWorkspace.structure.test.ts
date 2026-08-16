/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (relativePath: string): string =>
  readFileSync(new URL(relativePath, import.meta.url), 'utf8');

describe('conversation artifact image workspace wiring', () => {
  test('Nomi mounts LocalImageView.Provider and supplies its conversation workspace', () => {
    const source = readSource('./nomi/NomiChat.tsx');
    expect(source.includes("import LocalImageView from '@renderer/components/media/LocalImageView';")).toBe(true);
    expect(source.includes('const updateLocalImage = LocalImageView.useUpdateLocalImage();')).toBe(true);
    expect(source.includes('updateLocalImage({ root: workspace });')).toBe(true);
    expect(source.includes('LocalImageView.Provider)(')).toBe(true);
  });
});
