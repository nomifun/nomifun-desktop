/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./SkillDetailDrawer.tsx', import.meta.url), 'utf8');

describe('SkillDetailDrawer compact metadata layout', () => {
  test('keeps metadata rows close together without a location divider or extra top padding', () => {
    expect(source.includes("className='mt-12px flex flex-col gap-6px rounded-12px bg-fill-2 p-10px'")).toBe(true);
    expect(source.includes("className='flex min-w-0 items-start gap-8px'")).toBe(true);
    expect(source.includes('pt-10px')).toBe(false);
    expect(source.includes('border-t border-solid border-[var(--color-border-1)]')).toBe(false);
    expect(source.includes("className='flex min-w-0 flex-1 items-center gap-6px'")).toBe(true);
    expect(source.includes("<FolderOpen size={13} fill='currentColor' className='flex-shrink-0 text-t-tertiary' />")).toBe(true);
  });

  test('uses compact padding for the metadata header and instruction preview', () => {
    expect(source.includes('px-20px py-16px')).toBe(true);
    expect(source.includes('px-20px py-14px')).toBe(true);
    expect(source.includes('bg-base p-14px')).toBe(true);
  });
});
