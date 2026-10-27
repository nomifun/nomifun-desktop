import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import { officeHub } from '@/common/adapter/officeHubBridge';
import type { ConnectorId } from '@/common/connectors';
import { parseMcpServerId } from '@/common/types/ids';
import { ensureBackendMcpCatalog, subscribeMcpCatalogChanged } from '@/renderer/hooks/mcp/catalog';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { SessionCapabilityCatalog, SessionSkillOption } from './model';

const EMPTY_CATALOG: SessionCapabilityCatalog = {
  skills: [],
  autoSkillNames: new Set<string>(),
  mcpServers: [],
  connectorsReady: {},
};

/** Connector-provided skills stay opt-in but only once the connector is usable. */
const loadConnectorReadiness = async (): Promise<Partial<Record<ConnectorId, boolean>>> => {
  try {
    const status = await officeHub.status();
    const ready = status.phase === 'ready' && status.connection.enabled && status.runtime.phase === 'ready' && !!status.actor;
    return { feishu: ready };
  } catch {
    return {};
  }
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

const loadAppSources = () => appHosting.mcpSources.invoke()
  .then(sources => ({ sources, error: undefined as string | undefined }))
  .catch((cause: unknown) => ({ sources: [], error: String(cause) }));
const mergeMcpSources = (mcp: Awaited<ReturnType<typeof ensureBackendMcpCatalog>>, apps: Awaited<ReturnType<typeof loadAppSources>>) => [
  ...mcp.allServers.filter(server => !server.builtin),
  ...apps.sources.map(source => ({
    mcp_server_id: parseMcpServerId(source.app_id), name: source.name, description: source.reason || undefined,
    enabled: source.state === 'running', transport: { type: 'http' as const, url: source.url }, tools: source.tools,
    last_test_status: source.connection_status === 'connected' ? 'connected' as const : source.connection_status === 'error' ? 'error' as const : 'disconnected' as const,
    original_json: '{}', created_at: 0, updated_at: 0, appSource: true,
  })),
];

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
          const [mcp, apps] = await Promise.all([ensureBackendMcpCatalog(), loadAppSources()]);
          if (!active || requestGeneration !== generation) return;
          setCatalog((previous) => ({
            ...previous,
            appSourceError: apps.error,
            mcpServers: mergeMcpSources(mcp, apps),
          }));
        } else {
          const [available, auto, mcp, apps, connectorsReady] = await Promise.all([
            ipcBridge.fs.listAvailableSkills.invoke(),
            ipcBridge.fs.listBuiltinAutoSkills.invoke(),
            ensureBackendMcpCatalog(),
            loadAppSources(),
            loadConnectorReadiness(),
          ]);
          if (!active || requestGeneration !== generation) return;
          const merged = mergeSkills(available, auto);
          setCatalog({
            skills: merged.skills,
            autoSkillNames: merged.autoSkillNames,
            appSourceError: apps.error,
            mcpServers: mergeMcpSources(mcp, apps),
            connectorsReady,
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
