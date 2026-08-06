/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { ICompanionSkill } from '@/common/adapter/ipcBridge';
import { buildSkillEntries, countDrafts, filterSkillEntries, isSkillGranted } from './unify';

const generated = (
  id: string,
  name: string,
  status: ICompanionSkill['status'],
  updatedAt: number
): ICompanionSkill =>
  ({
    companion_skill_id: id,
    skill_name: name,
    status,
    source: 'mined',
    description: `${name} desc`,
    updated_at: updatedAt,
    created_at: updatedAt,
  }) as unknown as ICompanionSkill;

const catalog = [
  { name: 'cron', description: 'cron desc', location: '/skills/cron', source: 'builtin' },
  { name: 'mermaid', description: 'mermaid desc', location: '/skills/mermaid', source: 'custom' },
];

describe('unified companion skill list', () => {
  test('effective grant is (auto ∪ enabled) \\ disabled_auto', () => {
    const auto = new Set(['cron']);
    expect(isSkillGranted({ enabled: [], disabled_auto: [] }, auto, 'cron')).toBe(true);
    expect(isSkillGranted({ enabled: [], disabled_auto: ['cron'] }, auto, 'cron')).toBe(false);
    expect(isSkillGranted({ enabled: ['mermaid'], disabled_auto: [] }, auto, 'mermaid')).toBe(true);
    expect(isSkillGranted({ enabled: [], disabled_auto: [] }, auto, 'mermaid')).toBe(false);
  });

  test('drafts sort first, then active, then granted capabilities, then archived', () => {
    const entries = buildSkillEntries({
      generated: [
        generated('s1', 'archived-one', 'archived', 30),
        generated('s2', 'active-one', 'active', 20),
        generated('s3', 'draft-one', 'draft', 10),
      ],
      catalog,
      autoNames: new Set(['cron']),
      config: { enabled: ['mermaid'], disabled_auto: [] },
      missingDescription: 'missing',
    });
    expect(entries.map((entry) => entry.name)).toEqual([
      'draft-one',
      'active-one',
      'cron',
      'mermaid',
      'archived-one',
    ]);
    expect(countDrafts(entries)).toBe(1);
  });

  test('only granted catalog names appear, and an uninstalled grant is marked', () => {
    const entries = buildSkillEntries({
      generated: [],
      catalog,
      autoNames: new Set(['cron']),
      config: { enabled: ['ghost'], disabled_auto: ['cron'] },
      missingDescription: 'missing',
    });
    expect(entries.map((entry) => entry.name)).toEqual(['ghost']);
    expect(entries[0]).toMatchObject({ kind: 'catalog', installed: false, description: 'missing' });
  });

  test('localized display names replace the raw catalog name', () => {
    const entries = buildSkillEntries({
      generated: [],
      catalog: [{ ...catalog[0], name_i18n: { 'zh-CN': '定时任务' } }],
      autoNames: new Set(['cron']),
      config: { enabled: [], disabled_auto: [] },
      missingDescription: 'missing',
      display: (skill) => ({ name: skill.name_i18n?.['zh-CN'] ?? skill.name, description: skill.description }),
    });
    expect(entries[0].name).toBe('定时任务');
  });

  test('the source filter splits the merged list', () => {
    const entries = buildSkillEntries({
      generated: [generated('s1', 'mined-one', 'active', 1)],
      catalog,
      autoNames: new Set(['cron']),
      config: { enabled: [], disabled_auto: [] },
      missingDescription: 'missing',
    });
    expect(filterSkillEntries(entries, 'generated').map((entry) => entry.name)).toEqual(['mined-one']);
    expect(filterSkillEntries(entries, 'catalog').map((entry) => entry.name)).toEqual(['cron']);
    expect(filterSkillEntries(entries, 'all')).toHaveLength(2);
  });
});
