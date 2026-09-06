/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeAsset,
  CreativeAssetLibraryPort,
} from '../assets';
import type { PromptLibraryItem, PromptLibraryPort } from './types';

export interface NomiPromptLibraryPortOptions {
  locale?: string;
  assets?: CreativeAssetLibraryPort | null;
  catalog?: PromptLibraryPort | null;
  assetPageSize?: number;
}

function abortError(): Error {
  const error = new Error('Prompt library request was aborted');
  error.name = 'AbortError';
  return error;
}

function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted) throw abortError();
}

function promptIdentityKey(source: 'catalog', id: string): string {
  return `${source}\u0000${id}`;
}

/** Resolve the provenance of a prompt copied into My Assets. */
export function promptAssetIdentity(asset: CreativeAsset): { source: 'catalog'; id: string } | null {
  const source = asset.origin?.promptLibrarySource;
  const id = asset.origin?.promptLibraryId?.trim();
  return source === 'catalog' && id ? { source, id } : null;
}

export function mapNomiTextAssetToPromptLibraryItem(asset: CreativeAsset): PromptLibraryItem | null {
  if (asset.kind !== 'text' || !asset.inLibrary || !asset.textContent?.trim()) return null;
  if (promptAssetIdentity(asset)) return null;

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
    sourceUrl: asset.origin?.sourceUrl ?? null,
    license: asset.origin?.license ?? null,
    licenseUrl: asset.origin?.licenseUrl ?? null,
    createdAt: asset.createdAt,
    updatedAt: asset.updatedAt,
    savedToAssets: true,
  };
}

function markSavedPrompt(value: unknown, saved: ReadonlySet<string>): unknown {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return value;
  const item = value as Record<string, unknown>;
  const source = item.source;
  const id = typeof item.id === 'string' ? item.id.trim() : '';
  if (source !== 'catalog' || !id) return value;
  return {
    ...item,
    savedToAssets: saved.has(promptIdentityKey('catalog', id)),
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
    if (
      response.items.length === 0 ||
      result.length >= response.total ||
      response.items.length < pageSize
    ) {
      break;
    }
  }
  return result;
}

/**
 * The Creative Studio prompt library is intentionally independent from Agent
 * authoring. It combines the dedicated prompt catalog with user-owned text
 * assets; Agent documents are not prompt records.
 */
export function createNomiPromptLibraryPort(
  options: NomiPromptLibraryPortOptions = {}
): PromptLibraryPort {
  const pageSize = Math.max(1, Math.min(200, Math.trunc(options.assetPageSize ?? 100)));

  return {
    async list(signal) {
      throwIfAborted(signal);
      const [catalogData, assetData] = await Promise.all([
        options.catalog ? options.catalog.list(signal) : Promise.resolve([]),
        options.assets ? loadTextAssets(options.assets, pageSize, signal) : Promise.resolve([]),
      ]);
      throwIfAborted(signal);
      if (!Array.isArray(catalogData)) {
        throw new TypeError('Prompt catalog adapter must return an array');
      }

      const savedPrompts = new Set(
        assetData
          .map(promptAssetIdentity)
          .filter((identity): identity is { source: 'catalog'; id: string } => identity !== null)
          .map((identity) => promptIdentityKey(identity.source, identity.id))
      );

      return [
        ...catalogData.map((item) => markSavedPrompt(item, savedPrompts)),
        ...assetData
          .map(mapNomiTextAssetToPromptLibraryItem)
          .filter((item): item is PromptLibraryItem => item !== null),
      ].map((item) => markSavedPrompt(item, savedPrompts));
    },
  };
}
