import type {
  PluginDraftFile,
  PluginDraftSummary,
  PluginManifestSummary,
  PluginSummary,
  PluginSurfaceDescriptor,
} from '@/common/types/pluginPlatform';

export type PluginShape = 'ui_only' | 'headless' | 'mixed';
export type PluginLibraryView =
  | 'all'
  | 'enabled'
  | 'disabled'
  | 'drafts'
  | 'attention'
  | 'trash'
  | PluginShape;

export type PluginLibraryCounts = Record<PluginLibraryView, number>;

export type PluginLibraryEntry =
  | { kind: 'plugin'; key: string; plugin: PluginSummary }
  | { kind: 'draft'; key: string; draft: PluginDraftSummary };

export const PLUGIN_LIBRARY_VIEWS: readonly PluginLibraryView[] = [
  'all',
  'enabled',
  'disabled',
  'drafts',
  'attention',
  'trash',
  'ui_only',
  'headless',
  'mixed',
];

export function isPluginLibraryView(value: string | null): value is PluginLibraryView {
  return Boolean(value && PLUGIN_LIBRARY_VIEWS.includes(value as PluginLibraryView));
}

export function pluginShape(value: Pick<PluginSummary, 'has_ui' | 'has_service'>): PluginShape {
  if (value.has_ui && value.has_service) return 'mixed';
  return value.has_ui ? 'ui_only' : 'headless';
}

export function draftManifest(files: readonly PluginDraftFile[]): PluginManifestSummary | null {
  const file = files.find((candidate) => candidate.path === 'nomifun.plugin.json');
  if (!file?.text) return null;
  try {
    const manifest = JSON.parse(file.text) as Record<string, unknown>;
    const entrypoints = manifest.entrypoints as Record<string, unknown> | undefined;
    const rawActions = manifest.actions as Record<string, Record<string, unknown>> | undefined;
    const rawBindings = Array.isArray(manifest.bindings) ? manifest.bindings : [];
    if (
      manifest.schema !== 'nomifun.plugin/v1' ||
      typeof manifest.id !== 'string' ||
      typeof manifest.version !== 'string' ||
      typeof manifest.name !== 'string' ||
      typeof manifest.description !== 'string' ||
      typeof manifest.hostApi !== 'string' ||
      !entrypoints
    ) return null;
    const actions = Object.entries(rawActions ?? {}).map(([action_id, action]) => ({
      action_id,
      name: String(action.name ?? action_id),
      description: String(action.description ?? ''),
      input_schema: asSchema(action.input),
      output_schema: asSchema(action.output),
      effect: action.effect === 'write' || action.effect === 'external'
        ? action.effect as 'write' | 'external'
        : 'read' as const,
    }));
    const actionIds = new Set(actions.map((action) => action.action_id));
    const bindings = rawBindings.flatMap((candidate) => {
      if (!candidate || typeof candidate !== 'object') return [];
      const value = candidate as Record<string, unknown>;
      if (typeof value.point !== 'string' || typeof value.action !== 'string') return [];
      if (!actionIds.has(value.action)) return [];
      return [{
        point: value.point as PluginManifestSummary['bindings'][number]['point'],
        action_id: value.action,
        optional: value.optional === true,
        supported: true,
      }];
    });
    return {
      schema: manifest.schema,
      package_id: manifest.id,
      version: manifest.version,
      name: manifest.name,
      description: manifest.description,
      host_api: manifest.hostApi,
      entrypoints: {
        ...(typeof entrypoints.ui === 'string' ? { ui: entrypoints.ui } : {}),
        ...(typeof entrypoints.service === 'string' ? { service: entrypoints.service } : {}),
        ...(entrypoints.serviceMode === 'continuous'
          ? { service_mode: 'continuous' as const }
          : entrypoints.serviceMode === 'onDemand'
            ? { service_mode: 'on_demand' as const }
            : {}),
      },
      actions,
      bindings,
      data_version: Number.isSafeInteger(manifest.dataVersion) ? Number(manifest.dataVersion) : 0,
      config_schema: asSchema(manifest.configSchema),
      secret_slots: Array.isArray(manifest.secrets)
        ? manifest.secrets.filter((value): value is string => typeof value === 'string')
        : [],
      permissions: Array.isArray(manifest.permissions)
        ? manifest.permissions.filter((value): value is string => typeof value === 'string')
        : [],
    };
  } catch {
    return null;
  }
}

function asSchema(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : { type: 'object' };
}

export function pluginLibraryEntries(
  plugins: readonly PluginSummary[],
  drafts: readonly PluginDraftSummary[],
): PluginLibraryEntry[] {
  const entries: PluginLibraryEntry[] = plugins.map((plugin) => ({
    kind: 'plugin', key: `plugin:${plugin.plugin_id}`, plugin,
  }));
  for (const draft of drafts) {
    entries.push({ kind: 'draft', key: `draft:${draft.draft_id}`, draft });
  }
  return entries.sort((left, right) => {
    const leftTime = left.kind === 'plugin' ? left.plugin.updated_at_ms : left.draft.updated_at_ms;
    const rightTime = right.kind === 'plugin' ? right.plugin.updated_at_ms : right.draft.updated_at_ms;
    return rightTime - leftTime;
  });
}

export function pluginNeedsAttention(plugin: PluginSummary): boolean {
  return Boolean(plugin.last_error || plugin.runtime.state === 'failed');
}

export function draftNeedsAttention(draft: PluginDraftSummary): boolean {
  return draft.status === 'failed';
}

export function pluginLibraryCounts(
  plugins: readonly PluginSummary[],
  drafts: readonly PluginDraftSummary[],
): PluginLibraryCounts {
  const active = plugins.filter((plugin) => plugin.trashed_at_ms === undefined);
  return {
    all: active.length + drafts.length,
    enabled: active.filter((plugin) => plugin.enabled).length,
    disabled: active.filter((plugin) => !plugin.enabled).length,
    drafts: drafts.length,
    attention: active.filter(pluginNeedsAttention).length + drafts.filter(draftNeedsAttention).length,
    trash: plugins.length - active.length,
    ui_only: active.filter((plugin) => pluginShape(plugin) === 'ui_only').length,
    headless: active.filter((plugin) => pluginShape(plugin) === 'headless').length,
    mixed: active.filter((plugin) => pluginShape(plugin) === 'mixed').length,
  };
}

export function pluginEntryMatchesView(
  entry: PluginLibraryEntry,
  view: PluginLibraryView,
): boolean {
  if (entry.kind === 'draft') {
    if (view === 'all' || view === 'drafts') return true;
    return view === 'attention' && draftNeedsAttention(entry.draft);
  }
  const { plugin } = entry;
  const trashed = plugin.trashed_at_ms !== undefined;
  if (view === 'trash') return trashed;
  if (trashed) return false;
  if (view === 'all') return true;
  if (view === 'enabled') return plugin.enabled;
  if (view === 'disabled') return !plugin.enabled;
  if (view === 'attention') return pluginNeedsAttention(plugin);
  if (view === 'drafts') return false;
  return pluginShape(plugin) === view;
}

export function requiresDataLossWarning(plugin: PluginSummary): boolean {
  return Boolean(
    plugin.previous &&
    plugin.previous.data_generation !== plugin.active.data_generation,
  );
}

export function pluginSurfaceAssetPath(descriptor: PluginSurfaceDescriptor): string | null {
  const owner = descriptor.is_preview
    ? descriptor.draft_id
      ? `/api/plugin-drafts/${encodeURIComponent(descriptor.draft_id)}`
      : null
    : descriptor.plugin_id
      ? `/api/plugins/${encodeURIComponent(descriptor.plugin_id)}`
      : null;
  const segments = descriptor.entrypoint.split('/');
  if (
    !owner ||
    !descriptor.surface_session_id ||
    !Number.isSafeInteger(descriptor.surface_generation) ||
    descriptor.surface_generation < 1 ||
    !/^[a-f0-9]{64}$/.test(descriptor.artifact_digest) ||
    descriptor.entrypoint.startsWith('/') ||
    descriptor.entrypoint.includes('\\') ||
    segments.some((segment) => !segment || segment === '.' || segment === '..')
  ) return null;
  return `${owner}/surface/assets/${encodeURIComponent(descriptor.surface_session_id)}/${descriptor.surface_generation}/${descriptor.artifact_digest}/${segments.map(encodeURIComponent).join('/')}`;
}
