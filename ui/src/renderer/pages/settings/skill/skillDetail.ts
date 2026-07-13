import type { SkillInfo } from '@/renderer/pages/settings/PresetSettings/types';

export type SkillContentReaders = {
  readBuiltinSkill: (relativeLocation: string) => Promise<string>;
  readFile: (location: string) => Promise<string | null>;
};

/**
 * Read the canonical SKILL.md for a listed skill.
 *
 * Built-in skills use the dedicated embedded-resource route. Custom and
 * extension skills use the absolute location returned by GET /api/skills.
 */
export const readSkillContent = async (skill: SkillInfo, readers: SkillContentReaders): Promise<string> => {
  if (skill.source === 'builtin' && skill.relative_location) {
    return readers.readBuiltinSkill(skill.relative_location);
  }

  const content = await readers.readFile(skill.location);
  if (content === null) throw new Error('SKILL_CONTENT_NOT_FOUND');
  return content;
};

/** Hide YAML metadata in preview mode while preserving the exact source. */
export const stripSkillFrontmatter = (content: string): string => {
  const normalized = content.startsWith('\uFEFF') ? content.slice(1) : content;
  if (!normalized.startsWith('---')) return normalized;

  const match = normalized.match(/^---[\t ]*\r?\n[\s\S]*?\r?\n---[\t ]*(?:\r?\n|$)/);
  return match ? normalized.slice(match[0].length) : normalized;
};
