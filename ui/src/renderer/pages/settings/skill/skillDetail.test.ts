/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { SkillInfo } from '@/renderer/pages/settings/PresetSettings/types';
import { describe, expect, test } from 'bun:test';
import { readSkillContent, stripSkillFrontmatter } from './skillDetail';

const skill = (overrides: Partial<SkillInfo> = {}): SkillInfo => ({
  name: 'example',
  description: 'Example skill',
  location: '/tmp/example/SKILL.md',
  is_custom: true,
  source: 'custom',
  ...overrides,
});

describe('skill detail content', () => {
  test('uses the embedded-resource reader for built-in skills', async () => {
    const builtinCalls: string[] = [];
    const fileCalls: string[] = [];
    const readBuiltinSkill = async (location: string) => {
      builtinCalls.push(location);
      return 'builtin content';
    };
    const readFile = async (location: string) => {
      fileCalls.push(location);
      return 'file content';
    };

    const content = await readSkillContent(
      skill({ source: 'builtin', is_custom: false, relative_location: 'example/SKILL.md' }),
      { readBuiltinSkill, readFile }
    );

    expect(content).toBe('builtin content');
    expect(builtinCalls).toEqual(['example/SKILL.md']);
    expect(fileCalls).toEqual([]);
  });

  test('uses the listed absolute path for custom and extension skills', async () => {
    const builtinCalls: string[] = [];
    const fileCalls: string[] = [];
    const readBuiltinSkill = async (location: string) => {
      builtinCalls.push(location);
      return 'builtin content';
    };
    const readFile = async (location: string) => {
      fileCalls.push(location);
      return '# Custom skill';
    };

    const content = await readSkillContent(skill(), { readBuiltinSkill, readFile });

    expect(content).toBe('# Custom skill');
    expect(fileCalls).toEqual(['/tmp/example/SKILL.md']);
    expect(builtinCalls).toEqual([]);
  });

  test('rejects a missing skill file instead of rendering an empty document', async () => {
    let error: unknown;
    try {
      await readSkillContent(skill(), {
        readBuiltinSkill: async () => '',
        readFile: async () => null,
      });
    } catch (caught) {
      error = caught;
    }
    expect(error instanceof Error).toBe(true);
    expect((error as Error).message).toBe('SKILL_CONTENT_NOT_FOUND');
  });

  test('removes only the leading YAML frontmatter from preview content', () => {
    const source = '---\nname: example\ndescription: Example\n---\n\n# Instructions\n\nKeep this --- marker.';
    expect(stripSkillFrontmatter(source)).toBe('\n# Instructions\n\nKeep this --- marker.');
    expect(stripSkillFrontmatter('# No frontmatter')).toBe('# No frontmatter');
  });
});
