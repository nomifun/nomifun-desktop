/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type CreativeAssetKind = 'image' | 'video' | 'audio' | 'text';

export type CreativeAssetSort =
  | 'created_desc'
  | 'created_asc'
  | 'updated_desc'
  | 'name_asc'
  | 'size_desc';

export interface CreativeAssetOrigin {
  prompt?: string;
  model?: string;
  providerId?: string;
  params?: Record<string, unknown>;
  workbenchKind?: 'image' | 'video' | 'audio';
  canvasId?: string;
  nodeId?: string;
  generationTaskId?: string;
  /** Stable identity of a prompt-library item saved into My Assets. */
  promptLibrarySource?: CreativePromptLibrarySource;
  promptLibraryId?: string;
  /** Legacy catalog-only identity retained for existing saved assets. */
  promptCatalogId?: string;
  sourceUrl?: string;
  license?: string;
  licenseUrl?: string;
}

export type CreativePromptLibrarySource = 'catalog' | 'preset';

export interface CreativeCatalogPromptAssetOrigin {
  promptLibrarySource: 'catalog';
  promptLibraryId: string;
  promptCatalogId: string;
  sourceUrl?: string;
  license?: string;
  licenseUrl?: string;
}

export interface CreativePresetPromptAssetOrigin {
  promptLibrarySource: 'preset';
  promptLibraryId: string;
}

export type CreativePromptAssetOrigin =
  | CreativeCatalogPromptAssetOrigin
  | CreativePresetPromptAssetOrigin;

/** Product-facing asset shape. Backend snake_case is contained in api.ts/client.ts. */
export interface CreativeAsset {
  id: string;
  kind: CreativeAssetKind;
  title: string;
  collection: string | null;
  tags: string[];
  mimeType: string | null;
  width: number | null;
  height: number | null;
  bytes: number | null;
  inLibrary: boolean;
  textContent: string | null;
  origin: CreativeAssetOrigin | null;
  originalUrl: string;
  thumbnailUrl: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface CreativeAssetQuery {
  kind?: CreativeAssetKind;
  collection?: string;
  search?: string;
  inLibrary?: boolean;
  ungrouped?: boolean;
  tag?: string;
  sort?: CreativeAssetSort;
  page?: number;
  pageSize?: number;
}

export interface CreativeAssetPage {
  items: CreativeAsset[];
  total: number;
}

export interface CreativeAssetMetadata {
  title?: string;
  collection?: string;
  tags?: string[];
  inLibrary?: boolean;
}

export interface CreateCreativeTextAsset {
  title: string;
  textContent: string;
  collection?: string;
  tags?: string[];
  inLibrary?: boolean;
  origin?: CreativePromptAssetOrigin;
}

export interface CreativeAssetPatch {
  title?: string;
  collection?: string | null;
  tags?: string[];
  inLibrary?: boolean;
}

export type CreativeAssetVariant = 'original' | 'thumbnail';
export type CreativeAssetUploadProgress = (percent: number) => void;

/** Narrow capability boundary consumed by the Creative Studio product. */
export interface CreativeAssetPort {
  list(query?: CreativeAssetQuery): Promise<CreativeAssetPage>;
  upload(
    file: File,
    metadata?: CreativeAssetMetadata,
    signal?: AbortSignal,
    onProgress?: CreativeAssetUploadProgress
  ): Promise<CreativeAsset>;
  update(assetId: string, patch: CreativeAssetPatch): Promise<CreativeAsset>;
  remove(assetId: string): Promise<void>;
  url(assetId: string, variant?: CreativeAssetVariant): string;
}

/** Library-only extensions supported by the existing NomiFun asset service. */
export interface CreativeAssetLibraryPort extends CreativeAssetPort {
  createText(input: CreateCreativeTextAsset): Promise<CreativeAsset>;
  renameCollection(from: string, to: string): Promise<number>;
}

/** Prompt-library-specific membership mutation, kept out of general asset ports. */
export interface CreativePromptAssetPort {
  removePromptAsset(source: CreativePromptLibrarySource, promptId: string): Promise<number>;
}
