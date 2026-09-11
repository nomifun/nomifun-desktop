import { describe, expect, test } from 'bun:test';
import type { PluginLibraryResponse } from '@/common/types/pluginPlatform';
import {
  pluginDraftSourceEdits,
  pluginPackageId,
  pluginProductCategory,
  pluginProductItems,
  pluginProductMatches,
} from './pluginProductModel';

describe('Plugin product model', () => {
  test('classifies user-facing capability themes without exposing runtime concepts', () => {
    expect(pluginProductCategory('网页摘要与知识搜索')).toBe('knowledge');
    expect(pluginProductCategory('Git repository code review')).toBe('development');
    expect(pluginProductCategory('自动发送通知')).toBe('automation');
  });

  test('merges a linked authoring project into its installed Plugin card', () => {
    const library = {
      library_revision: 3,
      plugins: [{
        mount_id: '0190f5fe-7c00-7a00-8000-000000000001',
        mount_revision: 1,
        display_name: '网页摘要',
        lifecycle: 'enabled',
        linked_project_id: '0190f5fe-7c00-7a00-8000-000000000002',
        contribution_count: 2,
        updated_at_ms: 10,
      }],
      projects: [{
        project_id: '0190f5fe-7c00-7a00-8000-000000000002',
        project_revision: 2,
        display_name: '网页摘要',
        linked_mount_id: '0190f5fe-7c00-7a00-8000-000000000001',
        source_state: 'editable',
        build_generation: 1,
        apply_mode: 'ask_before_apply',
        auto_apply_authorization_revision: 0,
        updated_at_ms: 20,
      }],
    } as unknown as PluginLibraryResponse;
    const items = pluginProductItems(library);
    expect(items).toHaveLength(1);
    expect(items[0]?.project?.project_id).toBe(library.projects[0]?.project_id);
    expect(items[0]?.status).toBe('enabled');
    expect(pluginProductMatches(items[0]!, '网页')).toBe(true);
  });

  test('keeps uninstalled projects as resumable drafts', () => {
    const library = {
      library_revision: 1,
      plugins: [],
      projects: [{
        project_id: '0190f5fe-7c00-7a00-8000-000000000003',
        project_revision: 1,
        display_name: '发票整理',
        source_state: 'editable',
        build_generation: 0,
        apply_mode: 'ask_before_apply',
        auto_apply_authorization_revision: 0,
        updated_at_ms: 5,
      }],
    } as unknown as PluginLibraryResponse;
    expect(pluginProductItems(library)[0]?.status).toBe('draft');
  });

  test('creates opaque product-owned package ids and exact source edits', () => {
    expect(pluginPackageId(123)).toBe('user.nomifun.plugin-3f');
    const edits = pluginDraftSourceEdits({
      assistant_message: 'done',
      display_name: 'Example',
      description: 'Example Plugin',
      package_id: 'user.nomifun.example',
      package_version: '0.1.0',
      language: 'type_script',
      manifest_content: '{}',
      source_path: 'src/main.ts',
      source_content: 'export async function activate() {}',
      dependencies: {},
      capabilities: [],
    });
    expect(edits.map((edit) => edit.path)).toEqual([
      'nomifun.plugin.json',
      'package.json',
      'src/main.ts',
    ]);
    expect(JSON.parse(edits[1]!.content)).toEqual({
      name: 'user.nomifun.example',
      version: '0.1.0',
      description: 'Example Plugin',
      private: true,
      type: 'module',
      dependencies: {},
    });
  });
});
