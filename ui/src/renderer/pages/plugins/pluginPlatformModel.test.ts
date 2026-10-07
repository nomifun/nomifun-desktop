import { expect, test } from 'bun:test';
import type { PluginSummary, PluginSurfaceDescriptor } from '@/common/types/pluginPlatform';
import { isPluginLibraryView, pluginLibraryCounts, pluginShape, pluginSurfaceAssetPath } from './pluginPlatformModel';
const plugin = (overrides: Partial<PluginSummary> = {}): PluginSummary => ({
  plugin_id: 'plugin-1', package_id: 'local.todo', display_name: 'Todo', description: '',
  enabled: true, revision: 2, active: { artifact_digest: 'a'.repeat(64), package_version: '1.0.0', data_generation: 'g2', data_version: 2 },
  has_ui: true, has_service: false, action_count: 0, binding_count: 0, runtime: { state: 'stopped' }, updated_at_ms: 20, ...overrides,
});
test('library counts separate saved plugins, creation records and trash without duplicating states', () => {
  expect(pluginShape(plugin())).toBe('ui_only');
  expect(pluginShape(plugin({ has_ui: false, has_service: true }))).toBe('headless');
  expect(pluginShape(plugin({ has_service: true }))).toBe('mixed');
  expect(pluginLibraryCounts([
    plugin(), plugin({ plugin_id: 'disabled', enabled: false, has_ui: false, has_service: true }),
    plugin({ plugin_id: 'mixed', has_service: true, runtime: { state: 'failed' } }),
    plugin({ plugin_id: 'trashed', trashed_at_ms: 50 }),
  ], 2)).toEqual({ all: 3, drafts: 2, trash: 1, ui_only: 1, headless: 1, mixed: 1 });
  expect(isPluginLibraryView('all')).toBe(true);
  expect(isPluginLibraryView('attention')).toBe(false);
  expect(isPluginLibraryView('enabled')).toBe(false);
});
test('installed and preview assets use the same fenced path and reject invalid descriptors', () => {
  const descriptor: PluginSurfaceDescriptor = { plugin_id: 'plugin-1', artifact_digest: 'a'.repeat(64),
    surface_session_id: 'surface-1', surface_generation: 2, entrypoint: 'ui/index.html', is_preview: false };
  expect(pluginSurfaceAssetPath(descriptor)).toBe('/api/plugins/plugin-1/surface/assets/surface-1/2/' + 'a'.repeat(64) + '/ui/index.html');
  expect(pluginSurfaceAssetPath({ ...descriptor, plugin_id: undefined, draft_id: 'draft-1', is_preview: true }))
    .toBe('/api/plugin-drafts/draft-1/surface/assets/surface-1/2/' + 'a'.repeat(64) + '/ui/index.html');
  for (const change of [{ entrypoint: '../secret' }, { entrypoint: '/secret' }, { entrypoint: 'ui/../secret' }, { artifact_digest: 'invalid' }, { surface_generation: 0 }, { plugin_id: undefined }]) {
    expect(pluginSurfaceAssetPath({ ...descriptor, ...change })).toBeNull();
  }
});
