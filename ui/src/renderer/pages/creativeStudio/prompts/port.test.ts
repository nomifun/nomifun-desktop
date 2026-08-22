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
    });
    expect(mapNomiPresetToPromptLibraryItem(preset({ enabled: false }), [TAG], 'zh-CN')).toBeNull();
    expect(
      mapNomiPresetToPromptLibraryItem(preset({ targets: ['cron'] }), [TAG], 'zh-CN')
    ).toBeNull();
  });

  test('accepts only library text assets with real content', () => {
    expect(mapNomiTextAssetToPromptLibraryItem(TEXT_ASSET)?.prompt).toBe(
      '描述主体位置、景别与光线关系。'
    );
    expect(mapNomiTextAssetToPromptLibraryItem({ ...TEXT_ASSET, kind: 'image' })).toBeNull();
    expect(mapNomiTextAssetToPromptLibraryItem({ ...TEXT_ASSET, inLibrary: false })).toBeNull();
    expect(mapNomiTextAssetToPromptLibraryItem({ ...TEXT_ASSET, textContent: ' ' })).toBeNull();
    expect(
      mapNomiTextAssetToPromptLibraryItem({
        ...TEXT_ASSET,
        origin: {
          promptCatalogId: 'catalog-prompt-1',
          sourceUrl: 'https://example.test/source',
          license: 'CC0-1.0',
          licenseUrl: 'https://example.test/license',
        },
      })
    ).toMatchObject({
      sourceUrl: 'https://example.test/source',
      license: 'CC0-1.0',
      licenseUrl: 'https://example.test/license',
    });
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
});
