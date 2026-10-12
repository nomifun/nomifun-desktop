import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';
import type { InstalledMarketState } from './skillMarketProvenance';

export type SkillDisplay = {
  name: string;
  description: string;
};

/**
 * The display metadata shared by regular, auto-injected, and lightweight skill
 * records. Keep localization resolution here so every UI surface follows the
 * same exact-locale → language-family → canonical fallback order.
 */
export type LocalizableSkill = {
  name: string;
  description?: string;
  name_i18n?: Record<string, string>;
  description_i18n?: Record<string, string>;
  source?: string;
  location?: string;
};

const resolveMapValue = (map: Record<string, string> | undefined, localeKey: string): string | undefined => {
  const exact = map?.[localeKey]?.trim();
  if (exact) return exact;
  const family = localeKey.toLowerCase().split('-')[0];
  const familyKey = Object.keys(map ?? {}).find((key) => key.toLowerCase().split('-')[0] === family);
  const familyValue = familyKey ? map?.[familyKey]?.trim() : undefined;
  return familyValue || undefined;
};

/** Market metadata is display data, never an executable name or inferred translation. */
export const resolveMarketSkillDisplay = (
  item: Pick<ISkillMarketItem, 'name' | 'description'>
): SkillDisplay => ({ name: item.name, description: item.description });

/** One resolver for every installed/catalog skill surface. */
export const resolveSkillDisplay = (
  skill: LocalizableSkill,
  localeKey: string,
  marketState: InstalledMarketState = {}
): SkillDisplay => {
  if (skill.source === 'custom' && skill.location) {
    const matches = Object.values(marketState).filter(
      (record) => record.skill_names.includes(skill.name)
    );
    if (matches.length === 1 && matches[0].skill_names.length === 1 && matches[0].presentation) {
      return resolveMarketSkillDisplay(matches[0].presentation);
    }
  }
  return {
    name: resolveMapValue(skill.name_i18n, localeKey) || skill.name,
    description: resolveMapValue(skill.description_i18n, localeKey) || skill.description || '',
  };
};
