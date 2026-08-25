/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type PromptLibrarySource = 'catalog' | 'preset' | 'asset';

/** Validated product data. Remote media is restricted to absolute HTTPS URLs and rendered as data. */
export interface PromptLibraryItem {
  id: string;
  source: PromptLibrarySource;
  title: string;
  description: string | null;
  prompt: string;
  category: string | null;
  tags: string[];
  knowledgeBaseIds: string[];
  coverUrl: string | null;
  preview: string | null;
  sourceUrl: string | null;
  license: string | null;
  licenseUrl: string | null;
  createdAt: number | null;
  updatedAt: number | null;
  /** True when this catalog/preset prompt already has a provenance-linked asset. */
  savedToAssets: boolean;
}
/** Immutable payload handed to a canvas/editor integration. */
export interface PromptLibrarySelection {
  id: string;
  source: PromptLibrarySource;
  title: string;
  prompt: string;
  category: string | null;
  tags: string[];
  knowledgeBaseIds: string[];
  coverUrl: string | null;
  sourceUrl: string | null;
  license: string | null;
  licenseUrl: string | null;
}

/**
 * Narrow read-only boundary for prompt data. `unknown` is intentional: every
 * implementation, including NomiFun's own adapters, passes through the same
 * runtime validator before anything is rendered.
 */
export interface PromptLibraryPort {
  list(signal?: AbortSignal): Promise<unknown>;
}

export interface PromptLibraryFacets {
  categories: string[];
  tags: string[];
  hasUncategorized: boolean;
}

export interface PromptLibraryFilters {
  query?: string;
  /** `undefined` means all; `null` means uncategorized. */
  category?: string | null;
  /** Selected tags use intersection semantics. */
  tags?: readonly string[];
}

export interface NormalizedPromptLibrary {
  items: PromptLibraryItem[];
  invalidCount: number;
}
