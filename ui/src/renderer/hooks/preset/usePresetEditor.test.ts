import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { customSkillNamesFromIncluded } from './usePresetEditor';
import type { SkillInfo } from '@/renderer/pages/settings/PresetSettings/types';

const skill = (name: string, source: SkillInfo['source']): SkillInfo => ({
  name,
  source,
  description: '',
  location: '',
  is_custom: source === 'custom',
});

describe('preset editor skill grouping', () => {
  test('treats package-only included skills as custom without moving builtin skills', () => {
    expect(
      customSkillNamesFromIncluded(
        [skill('builtin-search', 'builtin'), skill('local-plan', 'custom'), skill('extension-chat', 'extension')],
        ['builtin-search', 'local-plan', 'package-copywriter', 'extension-chat']
      )
    ).toEqual(['local-plan', 'package-copywriter']);
  });

  test('does not save builtin auto-injected skills as regular included skills', () => {
    const source = readFileSync(new URL('./usePresetEditor.ts', import.meta.url), 'utf8');

    expect(source.includes('!builtinAutoNames.has(skillName)')).toBe(true);
  });

  test('does not re-add removed pending custom skills while saving', () => {
    const source = readFileSync(new URL('./usePresetEditor.ts', import.meta.url), 'utf8');

    expect(source.includes('.filter((skill) => selectedSkills.includes(skill.name))')).toBe(true);
    expect(source.includes('.map((skill) => skill.name)')).toBe(true);
  });
});
