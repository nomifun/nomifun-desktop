import { describe, expect, test } from 'bun:test';
import type { PluginDraftFile, PluginSummary, PluginSurfaceDescriptor } from '@/common/types/pluginPlatform';
import {
  draftManifest,
  pluginLibraryEntries,
  pluginShape,
  pluginSurfaceAssetPath,
  requiresDataLossWarning,
} from './pluginPlatformModel';

const plugin = (overrides: Partial<PluginSummary> = {}): PluginSummary => ({
  plugin_id: 'plugin-1', package_id: 'local.todo', display_name: 'Todo', description: '',
  enabled: true, revision: 2,
  active: { artifact_digest: 'a'.repeat(64), package_version: '1.0.0', data_generation: 'g2', data_version: 2 },
  previous: { artifact_digest: 'b'.repeat(64), package_version: '0.9.0', data_generation: 'g1', data_version: 1 },
  has_ui: true, has_service: false, action_count: 0, binding_count: 0,
  runtime: { state: 'stopped' }, updated_at_ms: 20, ...overrides,
});

describe('Unified Plugin UI model', () => {
  test('represents UI-only, headless and mixed packages without parallel identities', () => {
    expect(pluginShape(plugin())).toBe('ui_only');
    expect(pluginShape(plugin({ has_ui: false, has_service: true }))).toBe('headless');
    expect(pluginShape(plugin({ has_service: true }))).toBe('mixed');
    const entries = pluginLibraryEntries([plugin()], [
      { draft_id: 'draft-ready', revision: 1, display_name: 'Ready', description: '', status: 'ready', updated_at_ms: 30 },
      { draft_id: 'draft-generating', revision: 2, display_name: 'Generating', description: '', status: 'generating', updated_at_ms: 40 },
      { draft_id: 'draft-failed', revision: 3, display_name: 'Failed', description: '', status: 'failed', updated_at_ms: 50 },
    ]);
    expect(entries.map((entry) => entry.kind)).toEqual(['draft', 'draft', 'draft', 'plugin']);
    expect(entries.filter((entry) => entry.kind === 'draft').map((entry) => entry.draft.status))
      .toEqual(['failed', 'generating', 'ready']);
  });

  test('reads the canonical package manifest directly from Draft files', () => {
    const files: PluginDraftFile[] = [{
      path: 'nomifun.plugin.json', media_type: 'application/json', digest: 'c'.repeat(64), size_bytes: 10,
      text: JSON.stringify({ schema: 'nomifun.plugin/v1', id: 'local.todo', version: '1.0.0', name: 'Todo',
        description: 'Tasks', hostApi: '>=1 <2', entrypoints: { ui: 'ui/index.html', service: 'service/main.mjs', serviceMode: 'onDemand' },
        actions: { add: { name: 'Add', description: 'Add task', input: {type:'object'}, output: {type:'object'}, effect: 'write' } },
        bindings: [{ point: 'agent.tool', action: 'add' }], dataVersion: 1, configSchema: {type:'object'},
        secrets: ['api_key'], permissions: ['network'] }),
    }];
    expect(draftManifest(files)).toMatchObject({ package_id: 'local.todo', data_version: 1,
      entrypoints: { ui: 'ui/index.html', service: 'service/main.mjs', service_mode: 'on_demand' },
      actions: [{ action_id: 'add', effect: 'write' }], bindings: [{ point: 'agent.tool', action_id: 'add' }] });
  });

  test('uses one artifact-fenced asset route for installed and preview surfaces', () => {
    const descriptor: PluginSurfaceDescriptor = { plugin_id: 'plugin-1', artifact_digest: 'a'.repeat(64),
      surface_session_id: 'surface-1', surface_generation: 2, entrypoint: 'ui/index.html', is_preview: false };
    expect(pluginSurfaceAssetPath(descriptor)).toBe(`/api/plugins/plugin-1/surface/assets/surface-1/2/${'a'.repeat(64)}/ui/index.html`);
    expect(pluginSurfaceAssetPath({ ...descriptor, plugin_id: undefined, draft_id: 'draft-1', is_preview: true }))
      .toBe(`/api/plugin-drafts/draft-1/surface/assets/surface-1/2/${'a'.repeat(64)}/ui/index.html`);
    expect(pluginSurfaceAssetPath({ ...descriptor, entrypoint: '../secret' })).toBeNull();
    expect(requiresDataLossWarning(plugin())).toBe(true);
    expect(requiresDataLossWarning(plugin({ previous: { ...plugin().active } }))).toBe(false);
  });
});
