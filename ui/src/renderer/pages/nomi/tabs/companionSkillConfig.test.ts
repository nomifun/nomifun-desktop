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

  test('checking a Skill opted out while auto but since demoted from auto-inject enables it', () => {
    // Backend effective set is (auto ∪ enabled) \ disabled_auto: once the
    // Skill left the auto set, deleting the opt-out alone changes nothing —
    // the name must land in `enabled` or the first click is a no-op.
    expect(
      toggleCompanionSkill({ enabled: [], disabled_auto: ['web-clip'] }, new Set(['cron']), 'web-clip', true)
    ).toEqual({ enabled: ['web-clip'], disabled_auto: [] });
  });

  test('a missing Skill already in disabled_auto re-enables as an opt-in on check', () => {
    expect(
      toggleCompanionSkill({ enabled: [], disabled_auto: ['removed-auto'] }, new Set(), 'removed-auto', true)
    ).toEqual({ enabled: ['removed-auto'], disabled_auto: [] });
  });

  test('unchecking a demoted Skill previously re-enabled removes it from enabled', () => {
    expect(
      toggleCompanionSkill({ enabled: ['web-clip'], disabled_auto: [] }, new Set(['cron']), 'web-clip', false)
    ).toEqual({ enabled: [], disabled_auto: [] });
  });
});
