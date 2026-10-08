import { useEffect, useMemo, useState } from 'react';
import { ipcBridge } from '@/common';
import { useSkillDisplay } from '@/renderer/services/skills/useSkillDisplay';
import { CREATIVE_STUDIO_PLANNING_SKILLS } from './planningSkills';

/** Read-only metadata loading is independent of canvas Session/Turn state. */
export const usePlanningSkillOptions = (language: string) => {
  const [catalog, setCatalog] = useState<Awaited<ReturnType<typeof ipcBridge.fs.listAvailableSkills.invoke>>>([]);
  const getSkillDisplay = useSkillDisplay(language);
  useEffect(() => {
    let disposed = false;
    void ipcBridge.fs.listAvailableSkills.invoke().then((skills) => {
      if (!disposed) setCatalog(skills);
    }).catch((error) => {
      console.error('Failed to load planning skill metadata:', error);
    });
    return () => { disposed = true; };
  }, []);

  return useMemo(() => {
    const byName = new Map(catalog.map((skill) => [skill.name, skill]));
    return CREATIVE_STUDIO_PLANNING_SKILLS.map(({ id }) => {
      // An unavailable catalog has only the canonical ID, not invented copy.
      const display = getSkillDisplay(byName.get(id) ?? { name: id });
      return { id, label: display.name, description: display.description };
    });
  }, [catalog, getSkillDisplay]);
};
