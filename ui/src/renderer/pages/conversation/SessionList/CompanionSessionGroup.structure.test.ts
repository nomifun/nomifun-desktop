/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

describe('CompanionSessionGroup structure', () => {
  test('uses sidebar overflow controls for long companion rosters', () => {
    const source = readFileSync(join(dirname(fileURLToPath(import.meta.url)), 'CompanionSessionGroup.tsx'), 'utf8');

    expect(source.includes('getVisibleCompanionEntries')).toBe(true);
    expect(source.includes('showAllCompanions')).toBe(true);
    expect(source.includes("t('sessionList.expandDisplay'")).toBe(true);
    expect(source.includes("t('sessionList.collapseDisplay')")).toBe(true);
  });

  test('shows a purpose tip above companion session rows', () => {
    const source = readFileSync(join(dirname(fileURLToPath(import.meta.url)), 'CompanionSessionGroup.tsx'), 'utf8');

    expect(source.includes("import { Info, Robot } from '@icon-park/react';")).toBe(true);
    expect(source.includes("t('sessionList.companionTip')")).toBe(true);
    expect(source.includes('bg-[rgba(var(--primary-6),0.06)]')).toBe(true);
    expect(source.includes('inline-flex h-16px w-16px shrink-0 items-center justify-center')).toBe(true);
  });

  test('nests each robot thread under its bound companion, not a separate bucket', () => {
    const source = readFileSync(join(dirname(fileURLToPath(import.meta.url)), 'CompanionSessionGroup.tsx'), 'utf8');
    // Robot threads come from the shared list-sync snapshot (no extra fetch) and
    // are grouped by the companion they are bound to.
    expect(source.includes('useConversationListSync')).toBe(true);
    expect(source.includes('robotConversations')).toBe(true);
    expect(source.includes('robotsByCompanion')).toBe(true);
    expect(source.includes('companion_id')).toBe(true);
    // Device names share the one `/api/robots` request the robot tab uses.
    expect(source.includes("useSWR('robots.list'")).toBe(true);
    // Clicking a nested row opens that robot's own conversation.
    expect(source.includes('openRobotConversation')).toBe(true);
    // The standalone top-level bucket is gone.
    const listSource = readFileSync(join(dirname(fileURLToPath(import.meta.url)), 'index.tsx'), 'utf8');
    expect(listSource.includes('RobotSessionGroup')).toBe(false);
  });
});
