/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  NormalizedPromptLibrary,
  PromptLibraryFacets,
  PromptLibraryFilters,
  PromptLibraryItem,
  PromptLibrarySelection,
  PromptLibrarySource,
} from './types';

const SOURCES = new Set<PromptLibrarySource>(['catalog', 'preset', 'asset']);
const UNSAFE_CONTROL = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/;

function record(value: unknown): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError('Prompt library item must be an object');
  }
  return value as Record<string, unknown>;
}

function text(value: unknown, field: string, maxLength: number): string {
  if (typeof value !== 'string') throw new TypeError(`${field} must be a string`);
  const normalized = value.trim();
  if (!normalized || normalized.length > maxLength || UNSAFE_CONTROL.test(normalized)) {
    throw new TypeError(`${field} is not displayable`);
  }
  return normalized;
}

function nullableText(value: unknown, field: string, maxLength: number): string | null {
  if (value === null || value === undefined || value === '') return null;
  return text(value, field, maxLength);
}

function stringList(value: unknown, field: string, limit: number): string[] {
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.length > limit) {
    throw new TypeError(`${field} must be a bounded string array`);
  }
  const seen = new Set<string>();
  const result: string[] = [];
  for (const entry of value) {
    const item = text(entry, field, 128);
    if (!seen.has(item)) {
      seen.add(item);
      result.push(item);
    }
  }
  return result;
}

function source(value: unknown): PromptLibrarySource {
  if (typeof value !== 'string' || !SOURCES.has(value as PromptLibrarySource)) {
    throw new TypeError('Prompt library source is invalid');
  }
  return value as PromptLibrarySource;
}

function timestamp(value: unknown): number | null {
  if (value === null || value === undefined) return null;
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw new TypeError('Prompt library updatedAt is invalid');
  }
  return value;
}

function nullableHttpsUrl(value: unknown, field: string): string | null {
  const normalized = nullableText(value, field, 4_096);
  if (normalized === null) return null;
  let url: URL;
  try {
    url = new URL(normalized);
  } catch {
    throw new TypeError(`${field} must be an absolute URL`);
  }
  if (url.protocol !== 'https:') throw new TypeError(`${field} must use HTTPS`);
  return url.toString();
}

export function parsePromptLibraryItem(value: unknown): PromptLibraryItem {
  const item = record(value);
  return {
    id: text(item.id, 'id', 255),
    source: source(item.source),
    title: text(item.title, 'title', 240),
    description: nullableText(item.description, 'description', 4_000),
    prompt: text(item.prompt, 'prompt', 1_000_000),
    category: nullableText(item.category, 'category', 120),
    tags: stringList(item.tags, 'tags', 48),
    knowledgeBaseIds: stringList(item.knowledgeBaseIds, 'knowledgeBaseIds', 128),
    coverUrl: nullableHttpsUrl(item.coverUrl, 'coverUrl'),
    preview: nullableText(item.preview, 'preview', 100_000),
    sourceUrl: nullableHttpsUrl(item.sourceUrl, 'sourceUrl'),
    license: nullableText(item.license, 'license', 120),
    licenseUrl: nullableHttpsUrl(item.licenseUrl, 'licenseUrl'),
    createdAt: timestamp(item.createdAt),
    updatedAt: timestamp(item.updatedAt),
  };
}

/** Keep valid records only and preserve the first occurrence of each stable ID. */
export function normalizePromptLibrary(value: unknown): NormalizedPromptLibrary {
  if (!Array.isArray(value)) throw new TypeError('Prompt library response must be an array');
  const ids = new Set<string>();
  const items: PromptLibraryItem[] = [];
  let invalidCount = 0;

  for (const candidate of value) {
    try {
      const item = parsePromptLibraryItem(candidate);
      if (ids.has(item.id)) {
        invalidCount += 1;
        continue;
      }
      ids.add(item.id);
      items.push(item);
    } catch {
      invalidCount += 1;
    }
  }

  return { items, invalidCount };
}

export function promptLibraryFacets(items: readonly PromptLibraryItem[]): PromptLibraryFacets {
  const categories = new Set<string>();
  const tags = new Set<string>();
  let hasUncategorized = false;
  for (const item of items) {
    if (item.category) categories.add(item.category);
    else hasUncategorized = true;
    item.tags.forEach((tag) => tags.add(tag));
  }
  return {
    // Preserve the catalog's stable source order. This keeps the most useful
    // upstream facets first and matches the order users see in the source
    // library instead of moving @handles ahead of common creative tags.
    categories: [...categories],
    tags: [...tags],
    hasUncategorized,
  };
}

export function filterPromptLibraryItems(
  items: readonly PromptLibraryItem[],
  filters: PromptLibraryFilters
): PromptLibraryItem[] {
  const queryTerms = (filters.query?.trim().toLocaleLowerCase() ?? '')
    .split(/\s+/u)
    .filter(Boolean);
  const selectedTags = filters.tags ?? [];
  return items.filter((item) => {
    if (filters.category !== undefined && item.category !== filters.category) return false;
    if (selectedTags.some((tag) => !item.tags.includes(tag))) return false;
    if (queryTerms.length === 0) return true;
    const searchableText = [item.title, item.description ?? '', item.prompt, item.category ?? '', ...item.tags]
      .join('\n')
      .toLocaleLowerCase();
    return queryTerms.every((term) => searchableText.includes(term));
  });
}

/** Match the source library's newest-first card order while keeping null dates stable. */
export function sortPromptLibraryItemsByUpdatedAt(
  items: readonly PromptLibraryItem[]
): PromptLibraryItem[] {
  return items
    .map((item, index) => ({ item, index }))
    .sort((left, right) => {
      const leftTime = left.item.updatedAt ?? left.item.createdAt ?? Number.NEGATIVE_INFINITY;
      const rightTime = right.item.updatedAt ?? right.item.createdAt ?? Number.NEGATIVE_INFINITY;
      return rightTime - leftTime || left.index - right.index;
    })
    .map(({ item }) => item);
}

export function toPromptLibrarySelection(item: PromptLibraryItem): PromptLibrarySelection {
  return {
    id: item.id,
    source: item.source,
    title: item.title,
    prompt: item.prompt,
    category: item.category,
    tags: [...item.tags],
    knowledgeBaseIds: [...item.knowledgeBaseIds],
    coverUrl: item.coverUrl,
    sourceUrl: item.sourceUrl,
    license: item.license,
    licenseUrl: item.licenseUrl,
  };
}
