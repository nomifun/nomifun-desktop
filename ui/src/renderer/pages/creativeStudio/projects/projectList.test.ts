/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  formatProjectTimestamp,
  mergeProjects,
  projectErrorMessage,
  pruneProjectSelection,
} from './projectList';
import { CREATIVE_STUDIO_PROJECT_FIXTURES, createCreativeStudioProjectFixture } from './testing';

describe('Creative Studio project list model', () => {
  test('merges server updates by identity without mutating the source list', () => {
    const current = CREATIVE_STUDIO_PROJECT_FIXTURES.slice(0, 2);
    const renamed = { ...current[0], title: '更新后的品牌短片' };
    const added = createCreativeStudioProjectFixture({ id: 'new-project', title: '新画布' });
    const merged = mergeProjects(current, [renamed, added]);

    expect(merged.map(({ id }) => id)).toEqual([current[0].id, current[1].id, 'new-project']);
    expect(merged[0].title).toBe('更新后的品牌短片');
    expect(current[0].title).toBe('品牌短片概念');
  });

  test('prunes stale selection and keeps date/error fallbacks deterministic', () => {
    const selected = new Set([CREATIVE_STUDIO_PROJECT_FIXTURES[0].id, 'deleted-project']);
    expect([...pruneProjectSelection(selected, CREATIVE_STUDIO_PROJECT_FIXTURES)]).toEqual([
      CREATIVE_STUDIO_PROJECT_FIXTURES[0].id,
    ]);
    expect(formatProjectTimestamp(Number.NaN, 'zh-CN')).toBe('—');
    expect(projectErrorMessage(new Error('storage offline'))).toBe('storage offline');
    expect(projectErrorMessage(null)).toBe('Unknown error');
  });
});
