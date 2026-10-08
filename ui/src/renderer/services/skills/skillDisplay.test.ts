/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { SkillInfo } from '@/common/types/skill';
import { describe, expect, test } from 'bun:test';
import { resolveMarketSkillDisplay, resolveSkillDisplay } from './skillDisplay';
import { recordInstalledMarketItem } from './skillMarketProvenance';
import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';

const skill: SkillInfo = {
  name: 'planning-with-files',
  description:
    'Maintain durable task plans, findings, and progress files for long-running work.',
  location: '/tmp/builtin-skills/planning-with-files/SKILL.md',
  relative_location: 'planning-with-files/SKILL.md',
  is_custom: false,
  source: 'builtin',
  name_i18n: {
    'zh-CN': '文件化规划',
  },
  description_i18n: {
    'zh-CN': '为长期任务维护持久的计划、发现与进度文件。',
  },
};

describe('skill display localization', () => {
  test('uses localized built-in skill descriptions for the active locale', () => {
    expect(resolveSkillDisplay(skill, 'zh-CN').description).toBe(skill.description_i18n?.['zh-CN']);
  });

  test('falls back to the canonical skill description when locale metadata is missing', () => {
    expect(resolveSkillDisplay(skill, 'en-US').description).toBe(skill.description);
  });

  test('uses the same resolver for localized names and language-family locale variants', () => {
    expect(resolveSkillDisplay(skill, 'zh').name).toBe('文件化规划');
    expect(resolveSkillDisplay(skill, 'ZH-hans').description).toBe(skill.description_i18n?.['zh-CN']);
  });

  test('supports lightweight auto-injected skill records without SkillInfo-only fields', () => {
    expect(
      resolveSkillDisplay(
        {
          name: 'cron',
          description: 'Scheduled task management.',
          description_i18n: { 'zh-CN': '定时任务管理。' },
        },
        'zh-CN'
      )
    ).toEqual({
      name: 'cron',
      description: '定时任务管理。',
    });
  });
});

describe('shared skill presentation identity', () => {
  const item: ISkillMarketItem = {
    id: 'skillhub:owner/dev-expert', source: 'skillhub', rank: 1,
    name: '编程专家.Skill', description: 'GitHub coding helper',
    url: 'https://skillhub.cn/skills/owner/dev-expert',
    install_command: 'npx skills add @owner/dev-expert',
  };
  const installed = { name: 'dev-expert', description: 'Canonical instructions summary', source: 'custom', location: '/skills/dev-expert/SKILL.md' };
  const state = recordInstalledMarketItem({}, item, [installed.name]);

  test.each(['zh-CN', 'en-US'])('uses the exact market title and description without guessing translations (%s)', (locale) => {
    expect(resolveSkillDisplay(installed, locale, state)).toEqual(resolveMarketSkillDisplay(item));
    expect(installed.name).toBe('dev-expert');
    expect(installed.description).toBe('Canonical instructions summary');
  });

  test('does not overwrite builtin, extension, missing or unrelated catalog entries', () => {
    for (const entry of [
      { ...installed, source: 'builtin' },
      { ...installed, source: 'extension' },
      { ...installed, source: 'claude' },
      { ...installed, location: '' },
      { ...installed, name: 'different-owner/dev-expert' },
    ]) {
      expect(resolveSkillDisplay(entry, 'zh-CN', state)).toEqual({ name: entry.name, description: entry.description });
    }
  });

  test('fails closed on ambiguous provenance instead of choosing an arbitrary market owner', () => {
    const ambiguous = { ...state, 'skillhub:another/dev-expert': { ...state[item.id], presentation: { ...state[item.id].presentation!, name: 'Another title' } } };
    expect(resolveSkillDisplay(installed, 'zh-CN', ambiguous).name).toBe(installed.name);
  });

  test('preserves an empty market description instead of reviving divergent canonical copy', () => {
    const empty = recordInstalledMarketItem({}, { ...item, description: '' }, [installed.name]);
    expect(resolveSkillDisplay(installed, 'zh-CN', empty)).toEqual({ name: item.name, description: '' });
  });
});
