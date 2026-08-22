/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { Preset, PresetTag } from '@/common/types/agent/presetTypes';
import { presetSupportsTarget } from '@/common/types/agent/presetTypes';

import type { CreativeAsset, CreativeAssetLibraryPort } from '../assets';
import type { PromptLibraryItem, PromptLibraryPort } from './types';

export interface NomiPromptLibraryPortOptions {
  locale?: string;
  includePresets?: boolean;
  assets?: CreativeAssetLibraryPort | null;
  catalog?: PromptLibraryPort | null;
  assetPageSize?: number;
  loadPresets?: () => Promise<Preset[]>;
  loadPresetTags?: () => Promise<PresetTag[]>;
}
const defaultPresetLoader = (): Promise<Preset[]> => ipcBridge.presets.list.invoke();
const defaultPresetTagLoader = (): Promise<PresetTag[]> => ipcBridge.presetTags.list.invoke();

function abortError(): Error {
  const error = new Error('Prompt library request was aborted');
  error.name = 'AbortError';
  return error;
}

function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted) throw abortError();
}

function localeCandidates(locale: string): string[] {
  const normalized = locale.trim();
  const language = normalized.split('-')[0]?.toLowerCase();
  const regional = language === 'zh' ? 'zh-CN' : language === 'en' ? 'en-US' : '';
  return [...new Set([normalized, regional, 'zh-CN', 'en-US'].filter(Boolean))];
}

function localized(
  values: Record<string, string> | undefined,
  locale: string,
  fallback: string | undefined
): string {
  for (const candidate of localeCandidates(locale)) {
    const value = values?.[candidate]?.trim();
    if (value) return value;
  }
  return fallback?.trim() ?? '';
}

function tagLabels(tags: readonly PresetTag[], locale: string): Map<string, string> {
  return new Map(
    tags.map((tag) => [
      tag.preset_tag_id,
      localized(tag.label_i18n, locale, tag.label),
    ])
  );
}

export function mapNomiPresetToPromptLibraryItem(
  preset: Preset,
  tags: readonly PresetTag[],
  locale = 'zh-CN'
): PromptLibraryItem | null {
  if (!preset.enabled || !presetSupportsTarget(preset, 'conversation')) return null;
  const prompt = localized(preset.instructions_i18n, locale, preset.instructions);
  if (!prompt) return null;
  const labels = tagLabels(tags, locale);
  const scenarioTags = preset.scenario_tag_ids
    .map((id) => labels.get(id))
    .filter((label): label is string => Boolean(label));
  const audienceTags = preset.audience_tag_ids
    .map((id) => labels.get(id))
    .filter((label): label is string => Boolean(label));

  return {
    id: preset.preset_id,
    source: 'preset',
    title: localized(preset.name_i18n, locale, preset.name),
    description: localized(preset.description_i18n, locale, preset.description) || null,
    prompt,
    category: scenarioTags[0] ?? null,
    tags: [...new Set([...scenarioTags, ...audienceTags])],
    knowledgeBaseIds: preset.knowledge_bases.map((binding) => binding.knowledge_base_id),
    coverUrl: null,
    preview: null,
    sourceUrl: null,
    license: null,
    licenseUrl: null,
    createdAt: null,
    updatedAt: null,
  };
}

export function mapNomiTextAssetToPromptLibraryItem(asset: CreativeAsset): PromptLibraryItem | null {
  if (asset.kind !== 'text' || !asset.inLibrary || !asset.textContent?.trim()) return null;
  const promptOrigin = asset.origin;
  return {
    id: asset.id,
    source: 'asset',
    title: asset.title,
    description: null,
    prompt: asset.textContent,
    category: asset.collection,
    tags: [...asset.tags],
    knowledgeBaseIds: [],
    coverUrl: null,
    preview: null,
    sourceUrl: promptOrigin?.sourceUrl ?? null,
    license: promptOrigin?.license ?? null,
    licenseUrl: promptOrigin?.licenseUrl ?? null,
    createdAt: asset.createdAt,
    updatedAt: asset.updatedAt,
  };
}

async function loadTextAssets(
  assets: CreativeAssetLibraryPort,
  pageSize: number,
  signal?: AbortSignal
): Promise<CreativeAsset[]> {
  const result: CreativeAsset[] = [];
  for (let page = 1; page <= 50; page += 1) {
    throwIfAborted(signal);
    const response = await assets.list({ kind: 'text', inLibrary: true, page, pageSize });
    result.push(...response.items);
    if (response.items.length === 0 || result.length >= response.total || response.items.length < pageSize) {
      break;
    }
  }
  return result;
}

/**
 * Read prompt material from NomiFun-owned services only. Presets are the stable
 * built-in source; callers may opt into the existing Creative Asset port to
 * include user-owned text assets without adding a parallel backend.
 */
export function createNomiPromptLibraryPort(
  options: NomiPromptLibraryPortOptions = {}
): PromptLibraryPort {
  const locale = options.locale ?? 'zh-CN';
  const includePresets = options.includePresets ?? true;
  const loadPresets = options.loadPresets ?? defaultPresetLoader;
  const loadPresetTags = options.loadPresetTags ?? defaultPresetTagLoader;
  const pageSize = Math.max(1, Math.min(200, Math.trunc(options.assetPageSize ?? 100)));

  return {
    async list(signal) {
      throwIfAborted(signal);
      const [catalogData, presetData, assetData] = await Promise.all([
        options.catalog ? options.catalog.list(signal) : Promise.resolve([]),
        includePresets
          ? Promise.all([loadPresets(), loadPresetTags()])
          : Promise.resolve<[Preset[], PresetTag[]]>([[], []]),
        options.assets ? loadTextAssets(options.assets, pageSize, signal) : Promise.resolve([]),
      ]);
      throwIfAborted(signal);
      if (!Array.isArray(catalogData)) {
        throw new TypeError('Prompt catalog adapter must return an array');
      }
      const [presets, tags] = presetData;
      return [
        ...catalogData,
        ...presets
          .map((preset) => mapNomiPresetToPromptLibraryItem(preset, tags, locale))
          .filter((item): item is PromptLibraryItem => item !== null),
        ...assetData
          .map(mapNomiTextAssetToPromptLibraryItem)
          .filter((item): item is PromptLibraryItem => item !== null),
      ];
    },
  };
}
