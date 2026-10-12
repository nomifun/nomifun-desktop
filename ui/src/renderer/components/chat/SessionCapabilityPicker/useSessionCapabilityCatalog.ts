import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import { parseMcpServerId } from '@/common/types/ids';
import { ensureBackendMcpCatalog } from '@/renderer/hooks/mcp/catalog';
import { useCallback, useEffect, useState } from 'react';
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
  const retry = useCallback(() => setReloadToken((value) => value + 1), []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(undefined);
    void Promise.all([
      ipcBridge.fs.listAvailableSkills.invoke(),
      ipcBridge.fs.listBuiltinAutoSkills.invoke(),
      ensureBackendMcpCatalog(),
      appHosting.mcpSources.invoke().then((sources) => ({ sources, error: undefined as string | undefined }))
        .catch((cause: unknown) => ({ sources: [], error: String(cause) })),
    ])
      .then(([available, auto, mcp, apps]) => {
        if (cancelled) return;
        const merged = mergeSkills(available, auto);
        setCatalog({
          skills: merged.skills,
          autoSkillNames: merged.autoSkillNames,
          appSourceError: apps.error,
          mcpServers: [...mcp.allServers.filter((server) => !server.builtin), ...apps.sources.map((source) => ({
            mcp_server_id: parseMcpServerId(source.app_id), name: source.name, description: source.reason || undefined,
            enabled: source.state === 'running', transport: { type: 'http' as const, url: source.url }, tools: source.tools,
            last_test_status: source.connection_status === 'connected' ? 'connected' as const : source.connection_status === 'error' ? 'error' as const : 'disconnected' as const,
            original_json: '{}', created_at: 0, updated_at: 0, appSource: true,
          }))],
        });
      })
      .catch((cause) => {
        if (cancelled) return;
        const normalized = cause instanceof Error ? cause : new Error(String(cause));
        console.error('[SessionCapabilityPicker] Failed to load capability catalog:', normalized);
        setCatalog(EMPTY_CATALOG);
        setError(normalized);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [reloadToken]);

  return { catalog, loading, error, retry };
};
