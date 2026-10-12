import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { ipcBridge } from '@/common';
import { resolveSkillDisplay } from '@/renderer/services/skills/skillDisplay';
import { usePlanningSkillOptions } from './usePlanningSkillOptions';
import { CREATIVE_STUDIO_PLANNING_SKILLS } from './planningSkills';

afterEach(cleanup);

const catalog = CREATIVE_STUDIO_PLANNING_SKILLS.map(({ id }) => ({
  name: id, description: `${id} canonical description`,
  name_i18n: { 'zh-CN': `${id} 中文标题` }, description_i18n: { 'zh-CN': `${id} 中文说明` },
  source: 'builtin' as const, is_custom: false, location: `${id}/SKILL.md`,
}));

describe('canvas planning skill presentation', () => {
  test('loads the same catalog as the library and follows its shared locale resolver without changing IDs', async () => {
    const listing = spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue(catalog);
    try {
      const { result, rerender } = renderHook(({ language }) => usePlanningSkillOptions(language), { initialProps: { language: 'en-US' } });
      await waitFor(() => expect(result.current[0].description).toBe(catalog[0].description));
      const expected = (language: string) => catalog.map((skill) => {
        const display = resolveSkillDisplay(skill, language);
        return { id: skill.name, label: display.name, description: display.description };
      });
      expect(result.current).toEqual(expected('en-US'));
      await act(async () => rerender({ language: 'zh-CN' }));
      expect(result.current).toEqual(expected('zh-CN'));
      expect(listing).toHaveBeenCalledTimes(1);
    } finally { listing.mockRestore(); }
  });

  test('a failed metadata lookup neither invents descriptions nor removes planning identities', async () => {
    const failure = new Error('Catalog unavailable');
    const listing = spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockRejectedValue(failure);
    const logging = spyOn(console, 'error').mockImplementation(() => {});
    try {
      const { result } = renderHook(() => usePlanningSkillOptions('zh-CN'));
      await waitFor(() => expect(logging).toHaveBeenCalledWith('Failed to load planning skill metadata:', failure));
      expect(result.current).toEqual(CREATIVE_STUDIO_PLANNING_SKILLS.map(({ id }) => ({ id, label: id, description: '' })));
    } finally { listing.mockRestore(); logging.mockRestore(); }
  });
});
