import { useCallback, useSyncExternalStore } from 'react';
import { resolveLocaleKey } from '@/common/utils';
import { resolveSkillDisplay, type LocalizableSkill } from './skillDisplay';
import {
  getInstalledMarketSnapshot,
  subscribeInstalledMarketState,
} from './skillMarketProvenance';

export const useInstalledMarketState = () =>
  useSyncExternalStore(subscribeInstalledMarketState, getInstalledMarketSnapshot);

/** Display-only: never changes a skill's canonical name, instructions or grants. */
export const useSkillDisplay = (language: string) => {
  const state = useInstalledMarketState();
  const localeKey = resolveLocaleKey(language);
  return useCallback(
    (skill: LocalizableSkill) => resolveSkillDisplay(skill, localeKey, state),
    [localeKey, state]
  );
};
