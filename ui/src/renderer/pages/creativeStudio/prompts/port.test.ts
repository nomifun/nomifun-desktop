/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { Preset, PresetTag } from '@/common/types/agent/presetTypes';
import {
  parseAssetId,
  parseKnowledgeBaseId,
  parsePresetId,
  parsePresetTagId,
} from '@/common/types/ids';

import type { CreativeAsset, CreativeAssetLibraryPort } from '../assets';
import {
  createNomiPromptLibraryPort,
  mapNomiPresetToPromptLibraryItem,
  mapNomiTextAssetToPromptLibraryItem,
  promptAssetIdentity,
} from './port';

const PRESET_ID = parsePresetId('0190f5fe-7c00-7a00-8000-000000000001');
const SCENARIO_TAG_ID = parsePresetTagId('0190f5fe-7c00-7a00-8000-000000000002');
const KNOWLEDGE_ID = parseKnowledgeBaseId('0190f5fe-7c00-7a00-8000-000000000003');
const ASSET_ID = parseAssetId('0190f5fe-7c00-7a00-8000-000000000004');

function preset(overrides: Partial<Preset> = {}): Preset {
  return {
    preset_id: PRESET_ID,
    revision: 1,
    source: 'user',
    name: 'Storyboard helper',
    name_i18n: { 'zh-CN': '分镜助手' },
    description: 'Create a shot outline',
    description_i18n: { 'zh-CN': '整理镜头提纲' },
    instructions: 'Create a concise shot outline.',
    instructions_i18n: { 'zh-CN': '请整理一份简洁的镜头提纲。' },
    fallback_allowed: true,
    targets: ['conversation'],
    agent_preferences: [],
    model_preferences: [],
    included_skills: [],
    excluded_auto_skills: [],
    knowledge_policy: { enabled: true, writeback: false, grounded: true },
    knowledge_bases: [{ knowledge_base_id: KNOWLEDGE_ID, required: false }],
    examples: [],
    examples_i18n: {},
    audience_tag_ids: [],
    scenario_tag_ids: [SCENARIO_TAG_ID],
    enabled: true,
    auto_selectable: true,
    sort_order: 0,
    ...overrides,
  };
}

const TAG: PresetTag = {
  preset_tag_id: SCENARIO_TAG_ID,
  key: 'scenario.storyboard' as PresetTag['key'],
  dimension: 'scenario',
  label: 'Storyboard',
  label_i18n: { 'zh-CN': '分镜' },
  sort_order: 0,
  builtin: false,
};

const TEXT_ASSET: CreativeAsset = {
  id: ASSET_ID,
  kind: 'text',
  title: '构图笔记',
  collection: '视觉语言',
  tags: ['构图'],
  mimeType: null,
  width: null,
  height: null,
  bytes: null,
  inLibrary: true,
  textContent: '描述主体位置、景别与光线关系。',
  origin: null,
  originalUrl: '/api/creative-studio/files/asset',
  thumbnailUrl: null,
  createdAt: 10,
  updatedAt: 20,
};

function assets(items: CreativeAsset[]): CreativeAssetLibraryPort {
  return {
    list: async () => ({ items, total: items.length }),
    upload: async () => {
      throw new Error('not used');
    },
    createText: async () => {
      throw new Error('not used');
    },
    update: async () => {
      throw new Error('not used');
    },
    remove: async () => undefined,
    renameCollection: async () => 0,
    url: () => '',
  };
}

describe('Nomi prompt library port', () => {
  test('maps an enabled conversation preset and preserves knowledge bindings', () => {
    expect(mapNomiPresetToPromptLibraryItem(preset(), [TAG], 'zh-CN')).toEqual({
      id: PRESET_ID,
      source: 'preset',
      title: '分镜助手',
      description: '整理镜头提纲',
      prompt: '请整理一份简洁的镜头提纲。',
      category: '分镜',
      tags: ['分镜'],
      knowledgeBaseIds: [KNOWLEDGE_ID],
      coverUrl: null,
      preview: null,
      sourceUrl: null,
      license: null,
      licenseUrl: null,
      createdAt: null,
      updatedAt: null,
      savedToAssets: false,
    });
    expect(mapNomiPresetToPromptLibraryItem(preset({ enabled: false }), [TAG], 'zh-CN')).toBeNull();
    expect(
      mapNomiPresetToPromptLibraryItem(preset({ targets: ['cron'] }), [TAG], 'zh-CN')
    ).toBeNull();
  });

  test('accepts only independent library text assets with real content', () => {
    expect(mapNomiTextAssetToPromptLibraryItem(TEXT_ASSET)?.prompt).toBe(
      '描述主体位置、景别与光线关系。'
    );
    expect(mapNomiTextAssetToPromptLibraryItem({ ...TEXT_ASSET, kind: 'image' })).toBeNull();
    expect(mapNomiTextAssetToPromptLibraryItem({ ...TEXT_ASSET, inLibrary: false })).toBeNull();
    expect(mapNomiTextAssetToPromptLibraryItem({ ...TEXT_ASSET, textContent: ' ' })).toBeNull();
    const legacyMirror = {
      ...TEXT_ASSET,
      origin: {
        promptCatalogId: 'catalog-prompt-1',
        sourceUrl: 'https://example.test/source',
        license: 'CC0-1.0',
        licenseUrl: 'https://example.test/license',
      },
    };
    expect(promptAssetIdentity(legacyMirror)).toEqual({
      source: 'catalog',
      id: 'catalog-prompt-1',
    });
    expect(mapNomiTextAssetToPromptLibraryItem(legacyMirror)).toBeNull();

    const presetMirror = {
      ...TEXT_ASSET,
      origin: {
        promptLibrarySource: 'preset' as const,
        promptLibraryId: PRESET_ID,
      },
    };
    expect(promptAssetIdentity(presetMirror)).toEqual({
      source: 'preset',
      id: PRESET_ID,
    });
    expect(mapNomiTextAssetToPromptLibraryItem(presetMirror)).toBeNull();
  });

  test('combines injected Nomi preset and asset services without inventing data', async () => {
    const calls: string[] = [];
    const port = createNomiPromptLibraryPort({
      locale: 'zh-CN',
      loadPresets: async () => {
        calls.push('presets');
        return [preset()];
      },
      loadPresetTags: async () => {
        calls.push('tags');
        return [TAG];
      },
      assets: assets([TEXT_ASSET, { ...TEXT_ASSET, id: parseAssetId('0190f5fe-7c00-7a00-8000-000000000005'), kind: 'video' }]),
    });

    const result = await port.list();
    expect(Array.isArray(result)).toBe(true);
    expect((result as Array<{ source: string }>).map((item) => item.source)).toEqual([
      'preset',
      'asset',
    ]);
    expect(calls).toEqual(['presets', 'tags']);
  });

  test('filters saved mirrors and marks their original catalog/preset entries as added', async () => {
    const catalogId = 'catalog-prompt-1';
    const catalogItem = {
      id: catalogId,
      source: 'catalog',
      title: 'Catalog prompt',
      description: null,
      prompt: 'Catalog prompt body',
      category: null,
      tags: [],
      knowledgeBaseIds: [],
      coverUrl: null,
      preview: null,
      sourceUrl: 'https://example.test/source',
      license: 'MIT',
      licenseUrl: 'https://example.test/license',
      createdAt: null,
      updatedAt: null,
      savedToAssets: false,
    };
    const catalogMirror: CreativeAsset = {
      ...TEXT_ASSET,
      origin: {
        promptLibrarySource: 'catalog',
        promptLibraryId: catalogId,
        promptCatalogId: catalogId,
      },
    };
    const presetMirror: CreativeAsset = {
      ...TEXT_ASSET,
      id: parseAssetId('0190f5fe-7c00-7a00-8000-000000000006'),
      origin: {
        promptLibrarySource: 'preset',
        promptLibraryId: PRESET_ID,
      },
    };
    const port = createNomiPromptLibraryPort({
      locale: 'zh-CN',
      catalog: { list: async () => [catalogItem] },
      loadPresets: async () => [preset()],
      loadPresetTags: async () => [TAG],
      assets: assets([catalogMirror, presetMirror, TEXT_ASSET]),
    });

    const result = (await port.list()) as Array<{
      id: string;
      source: string;
      savedToAssets: boolean;
    }>;
    expect(result.map(({ source, id }) => `${source}:${id}`)).toEqual([
      `catalog:${catalogId}`,
      `preset:${PRESET_ID}`,
      `asset:${ASSET_ID}`,
    ]);
    expect(result.slice(0, 2).every((item) => item.savedToAssets)).toBe(true);
  });

  test('reflects a server-side removal after the prompt port reloads', async () => {
    const catalogId = 'catalog-removable';
    const catalogItem = {
      id: catalogId,
      source: 'catalog',
      title: 'Removable prompt',
      description: null,
      prompt: 'Prompt body',
      category: null,
      tags: [],
      knowledgeBaseIds: [],
      coverUrl: null,
      preview: null,
      sourceUrl: 'https://example.test/source',
      license: 'MIT',
      licenseUrl: 'https://example.test/license',
      createdAt: null,
      updatedAt: null,
      savedToAssets: false,
    };
    const mirror: CreativeAsset = {
      ...TEXT_ASSET,
      origin: {
        promptLibrarySource: 'catalog',
        promptLibraryId: catalogId,
        promptCatalogId: catalogId,
      },
    };
    let visible = true;
    const mutableAssets: CreativeAssetLibraryPort = {
      ...assets([]),
      list: async (query) => {
        const items = visible && query?.inLibrary ? [mirror] : [];
        return { items, total: items.length };
      },
    };
    const port = createNomiPromptLibraryPort({
      includePresets: false,
      catalog: { list: async () => [catalogItem] },
      assets: mutableAssets,
    });

    const before = (await port.list()) as Array<{ source: string; savedToAssets: boolean }>;
    expect(before).toHaveLength(1);
    expect(before[0]).toMatchObject({ source: 'catalog', savedToAssets: true });

    visible = false;
    const after = (await port.list()) as Array<{ source: string; savedToAssets: boolean }>;
    expect(after).toHaveLength(1);
    expect(after[0]).toMatchObject({ source: 'catalog', savedToAssets: false });
  });
});
