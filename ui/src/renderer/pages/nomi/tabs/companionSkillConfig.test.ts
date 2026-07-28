/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { toggleCompanionSkill } from './companionSkillConfig';

describe('desktop companion global Skill assignment', () => {
  test('disabling an auto Skill records only disabled_auto', () => {
    expect(toggleCompanionSkill({ enabled: [], disabled_auto: [] }, new Set(['cron']), 'cron', false)).toEqual({
      enabled: [],
      disabled_auto: ['cron'],
    });
  });

  test('enabling an opt-in Skill records only enabled', () => {
    expect(toggleCompanionSkill({ enabled: [], disabled_auto: [] }, new Set(), 'mermaid', true)).toEqual({
      enabled: ['mermaid'],
      disabled_auto: [],
    });
  });

  test('enabling an auto Skill removes its opt-out and returns sorted unique arrays', () => {
    expect(
      toggleCompanionSkill(
        { enabled: ['pdf', 'mermaid', 'pdf'], disabled_auto: ['todo', 'cron', 'cron'] },
        new Set(['cron']),
        'cron',
        true
      )
    ).toEqual({ enabled: ['mermaid', 'pdf'], disabled_auto: ['todo'] });
  });

  test('a missing Skill already in disabled_auto remains identifiable as auto', () => {
    expect(
      toggleCompanionSkill({ enabled: [], disabled_auto: ['removed-auto'] }, new Set(), 'removed-auto', true)
    ).toEqual({ enabled: [], disabled_auto: [] });
  });
});
