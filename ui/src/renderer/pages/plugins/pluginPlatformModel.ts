import type { PluginSummary, PluginSurfaceDescriptor } from '@/common/types/pluginPlatform';

export type PluginShape = 'ui_only' | 'headless' | 'mixed';
export type PluginLibraryView = 'all' | 'drafts' | 'trash' | PluginShape;
export type PluginLibraryCounts = Record<PluginLibraryView, number>;
const PLUGIN_LIBRARY_VIEWS: readonly PluginLibraryView[] = ['all', 'drafts', 'trash', 'ui_only', 'headless', 'mixed'];

export function isPluginLibraryView(value: string | null): value is PluginLibraryView {
  return Boolean(value && PLUGIN_LIBRARY_VIEWS.includes(value as PluginLibraryView));
}
export function pluginShape(value: Pick<PluginSummary, 'has_ui' | 'has_service'>): PluginShape {
  if (value.has_ui && value.has_service) return 'mixed';
  return value.has_ui ? 'ui_only' : 'headless';
}
export function pluginLibraryCounts(plugins: readonly PluginSummary[], creationCount = 0): PluginLibraryCounts {
  const active = plugins.filter(plugin => plugin.trashed_at_ms === undefined);
  return {
    all: active.length, drafts: creationCount, trash: plugins.length - active.length,
    ui_only: active.filter(plugin => pluginShape(plugin) === 'ui_only').length,
    headless: active.filter(plugin => pluginShape(plugin) === 'headless').length,
    mixed: active.filter(plugin => pluginShape(plugin) === 'mixed').length,
  };
}
export function pluginSurfaceAssetPath(descriptor: PluginSurfaceDescriptor): string | null {
  const owner = descriptor.is_preview
    ? descriptor.draft_id ? '/api/plugin-drafts/' + encodeURIComponent(descriptor.draft_id) : null
    : descriptor.plugin_id ? '/api/plugins/' + encodeURIComponent(descriptor.plugin_id) : null;
  const segments = descriptor.entrypoint.split('/');
  if (!owner || !descriptor.surface_session_id || !Number.isSafeInteger(descriptor.surface_generation) ||
    descriptor.surface_generation < 1 || !/^[a-f0-9]{64}$/.test(descriptor.artifact_digest) ||
    descriptor.entrypoint.startsWith('/') || descriptor.entrypoint.includes('\\') ||
    segments.some(segment => !segment || segment === '.' || segment === '..')) return null;
  return owner + '/surface/assets/' + encodeURIComponent(descriptor.surface_session_id) + '/' + descriptor.surface_generation +
    '/' + descriptor.artifact_digest + '/' + segments.map(encodeURIComponent).join('/');
}
