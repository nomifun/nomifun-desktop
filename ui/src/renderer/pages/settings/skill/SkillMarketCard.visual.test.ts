/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./SkillMarketCard.tsx', import.meta.url), 'utf8');
const panelSource = readFileSync(new URL('../MarketSettingsPanel.tsx', import.meta.url), 'utf8');

describe('SkillMarketCard visual hierarchy', () => {
  test('removes ranking avatars from every shared market card', () => {
    expect(source.includes('item.rank')).toBe(false);
    expect(source.includes('getAvatarColorClass')).toBe(false);
    expect(source.includes("className='min-w-0 pr-76px'")).toBe(true);
  });

  test('uses a quiet, high-contrast secondary add action', () => {
    expect(source.includes("type='secondary'")).toBe(true);
    expect(source.includes('!bg-[var(--color-fill-2)]')).toBe(true);
    expect(source.includes('!text-[var(--color-text-1)]')).toBe(true);
    expect(source.includes('!border-[var(--color-border-2)]')).toBe(true);
    expect(source.includes('hover:!bg-[var(--color-fill-3)]')).toBe(true);
    expect(source.includes("type='primary'")).toBe(false);
  });

  test('keeps the full install command copyable beside its truncated preview', () => {
    expect(source.includes("import CopyIconButton from '@/renderer/components/base/CopyIconButton'")).toBe(true);
    expect(source.includes("className='min-w-0 flex-1 truncate text-11px")).toBe(true);
    expect(source.includes('text={item.install_command}')).toBe(true);
    expect(source.includes("t('settings.skillsMarket.copyCommand'")).toBe(true);
    expect(source.includes("className='size-22px shrink-0 -mr-4px'")).toBe(true);
  });

  test('locks each add action while its asynchronous request is pending', () => {
    expect(panelSource.includes('const pendingAddIdsRef = useRef<Set<string>>(new Set())')).toBe(true);
    expect(panelSource.includes('if (pendingAddIdsRef.current.has(item.id)) return')).toBe(true);
    expect(panelSource.includes('await onAdd(item)')).toBe(true);
    expect(panelSource.includes('finished.delete(item.id)')).toBe(true);
    expect(panelSource.includes("console.error('Market add callback failed:'")).toBe(true);
    expect(source.includes('loading={adding}')).toBe(true);
    expect(source.includes('disabled={adding}')).toBe(true);
  });
});
