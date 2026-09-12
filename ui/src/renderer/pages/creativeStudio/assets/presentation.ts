/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from './types';

const titleSegments = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

/** Display-only: keep the stored name and complete prompt available for editing. */
export function creativeAssetDisplayTitle(
  asset: Pick<CreativeAsset, 'id' | 'title' | 'origin' | 'textContent'>
): string {
  const title = asset.title.trim();
  const prompt = asset.origin?.prompt?.trim() ?? '';
  // Generated assets historically store the first 60 Unicode code points of
  // the prompt as their title. Only match that exact default, not any prefix.
  const generatedTitle = prompt && !asset.origin?.promptLibrarySource && !asset.origin?.promptCatalogId
    && title === Array.from(prompt).slice(0, 60).join('');
  if (title && !generatedTitle) return title;

  const description = prompt || asset.textContent?.trim() || '';
  const characters: string[] = [];
  for (const { segment } of titleSegments.segment(description)) {
    characters.push(segment);
    if (characters.length === 15) break;
  }
  return characters.join('') || title || asset.id;
}

export function creativeAssetTags(asset: Pick<CreativeAsset, 'tags'>): string[] {
  return [...new Set(asset.tags.map((tag) => tag.trim()).filter(Boolean))];
}

export const formatCreativeAssetBytes = (bytes: number | null): string => {
  if (bytes == null || bytes < 0 || !Number.isFinite(bytes)) return '—';
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
};
