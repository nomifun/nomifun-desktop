import { ipcBridge } from '@/common';
import { ensureBackendMcpCatalog, subscribeMcpCatalogChanged } from '@/renderer/hooks/mcp/catalog';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { SessionCapabilityCatalog, SessionSkillOption } from './model';

const EMPTY_CATALOG: SessionCapabilityCatalog = {
  skills: [],
  autoSkillNames: new Set<string>(),
  mcpServers: [],
};

const mergeSkills = (
  available: Awaited<ReturnType<typeof ipcBridge.fs.listAvailableSkills.invoke>>,
  auto: Awaited<ReturnType<typeof ipcBridge.fs.listBuiltinAutoSkills.invoke>>
): { skills: SessionSkillOption[]; autoSkillNames: Set<string> } => {
  const autoSkillNames = new Set(auto.map((skill) => skill.name));
  const byName = new Map<string, SessionSkillOption>(
    available.map((skill) => [skill.name, { ...skill, auto: autoSkillNames.has(skill.name) }])
  );
  for (const skill of auto) {
    if (!byName.has(skill.name)) {
      byName.set(skill.name, {
        ...skill,
        location: skill.location ?? '',
        is_custom: false,
        source: 'builtin',
        auto: true,
      });
    }
  }
  const skills = Array.from(byName.values())
    .map((skill) => ({ ...skill, auto: autoSkillNames.has(skill.name) }))
    .sort((left, right) => {
      if (left.auto !== right.auto) return left.auto ? -1 : 1;
      if (left.source !== right.source) return left.source === 'builtin' ? -1 : 1;
      return left.name.localeCompare(right.name);
    });
  return { skills, autoSkillNames };
};

export const useSessionCapabilityCatalog = () => {
  const [catalog, setCatalog] = useState<SessionCapabilityCatalog>(EMPTY_CATALOG);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error>();
  const [reloadToken, setReloadToken] = useState(0);
  const hasLoaded = useRef(false);
  const retry = useCallback(() => setReloadToken((value) => value + 1), []);

  useEffect(() => {
    let active = true;
    let generation = 0;
    const reload = async (mcpOnly = false) => {
      // Increment synchronously on invalidation, so a pending older read can
      // never commit in the gap before React runs another effect.
      const requestGeneration = ++generation;
      if (!hasLoaded.current) {
        setLoading(true);
        setError(undefined);
      }
      try {
        if (mcpOnly && hasLoaded.current) {
          const mcp = await ensureBackendMcpCatalog();
          if (!active || requestGeneration !== generation) return;
          setCatalog((previous) => ({
            ...previous,
            mcpServers: mcp.allServers.filter((server) => !server.builtin),
          }));
        } else {
          const [available, auto, mcp] = await Promise.all([
            ipcBridge.fs.listAvailableSkills.invoke(),
            ipcBridge.fs.listBuiltinAutoSkills.invoke(),
            ensureBackendMcpCatalog(),
          ]);
          if (!active || requestGeneration !== generation) return;
          const merged = mergeSkills(available, auto);
          setCatalog({
            skills: merged.skills,
            autoSkillNames: merged.autoSkillNames,
            mcpServers: mcp.allServers.filter((server) => !server.builtin),
          });
        }
        hasLoaded.current = true;
        setError(undefined);
      } catch (cause) {
        if (!active || requestGeneration !== generation) return;
        const normalized = cause instanceof Error ? cause : new Error(String(cause));
        console.error('[SessionCapabilityPicker] Failed to load capability catalog:', normalized);
        setError(normalized);
      } finally {
        if (active && requestGeneration === generation) setLoading(false);
      }
    };
    const unsubscribe = subscribeMcpCatalogChanged(() => { void reload(true); });
    void reload();
    return () => { active = false; unsubscribe(); };
  }, [reloadToken]);

  return { catalog, loading, error, retry };
};
