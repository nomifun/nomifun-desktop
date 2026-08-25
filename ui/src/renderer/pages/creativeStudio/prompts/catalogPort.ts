/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpGet, httpPost } from '@/common/adapter/httpBridge';

import type { PromptLibraryPort } from './types';

interface PromptCatalogTransport {
  list(): Promise<unknown>;
  sync(force: boolean): Promise<unknown>;
}

export interface CreativePromptCatalogPortOptions {
  transport?: PromptCatalogTransport;
}

interface ParsedCatalogPage {
  items: unknown[];
  stale: boolean;
}

const listCatalog = httpGet<unknown>('/api/creative-studio/prompts', {
  timeoutMs: 15_000,
});
const syncCatalog = httpPost<unknown, { force: boolean }>(
  '/api/creative-studio/prompts/sync',
  (request) => request,
  { timeoutMs: 120_000 }
);

const defaultTransport: PromptCatalogTransport = {
  list: () => listCatalog.invoke(),
  sync: (force) => syncCatalog.invoke({ force }),
};

function record(value: unknown, field: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError(`${field} must be an object`);
  }
  return value as Record<string, unknown>;
}

function timestamp(value: unknown, field: string): number | null {
  if (value === null || value === undefined || value === '') return null;
  if (typeof value !== 'string') throw new TypeError(`${field} must be an RFC 3339 string`);
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed) || parsed < 0) {
    throw new TypeError(`${field} must be a valid RFC 3339 string`);
  }
  return parsed;
}

function mapCatalogItem(value: unknown): unknown {
  const item = record(value, 'prompt catalog item');
  return {
    id: item.id,
    source: 'catalog',
    title: item.title,
    description: null,
    prompt: item.prompt,
    category: item.category,
    tags: item.tags,
    knowledgeBaseIds: [],
    coverUrl: item.coverUrl ?? null,
    preview: item.preview || null,
    sourceUrl: item.sourceUrl ?? null,
    license: item.license ?? null,
    licenseUrl: item.licenseUrl ?? null,
    createdAt: timestamp(item.createdAt, 'createdAt'),
    updatedAt: timestamp(item.updatedAt, 'updatedAt'),
    savedToAssets: false,
  };
}

function parseCatalogPage(value: unknown): ParsedCatalogPage {
  const page = record(value, 'prompt catalog response');
  if (!Array.isArray(page.items)) throw new TypeError('prompt catalog items must be an array');
  if (typeof page.stale !== 'boolean') throw new TypeError('prompt catalog stale must be boolean');
  if (
    typeof page.total !== 'number' ||
    !Number.isSafeInteger(page.total) ||
    page.total < 0 ||
    page.total !== page.items.length
  ) {
    throw new TypeError('prompt catalog total is inconsistent');
  }
  return { items: page.items.map(mapCatalogItem), stale: page.stale };
}

function abortError(): Error {
  const error = new Error('Prompt catalog request was aborted');
  error.name = 'AbortError';
  return error;
}

function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted) throw abortError();
}

/**
 * Production prompt-catalog adapter. The first empty/stale read performs one
 * owner-only synchronization. Concurrent StrictMode mounts share that same
 * request; when refresh fails, an existing valid cache remains usable offline.
 */
export function createCreativePromptCatalogPort(
  options: CreativePromptCatalogPortOptions = {}
): PromptLibraryPort {
  const transport = options.transport ?? defaultTransport;
  let syncInFlight: Promise<ParsedCatalogPage> | null = null;

  const synchronize = (): Promise<ParsedCatalogPage> => {
    if (!syncInFlight) {
      syncInFlight = transport
        .sync(false)
        .then(parseCatalogPage)
        .finally(() => {
          syncInFlight = null;
        });
    }
    return syncInFlight;
  };

  return {
    async list(signal) {
      throwIfAborted(signal);
      const cached = parseCatalogPage(await transport.list());
      throwIfAborted(signal);
      if (!cached.stale && cached.items.length > 0) return cached.items;
      try {
        const refreshed = await synchronize();
        throwIfAborted(signal);
        return refreshed.items;
      } catch (error) {
        throwIfAborted(signal);
        if (cached.items.length > 0) return cached.items;
        throw error;
      }
    },
  };
}

export const creativePromptCatalogPort = createCreativePromptCatalogPort();
