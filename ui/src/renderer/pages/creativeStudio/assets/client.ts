/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { parseAssetId } from '@/common/types/ids';

import { workshopAssetApi } from './api';
import { isCreativeAssetDeleted } from './types';
import { notifyCreativeAssetDeleted } from './assetDeletion';
import { invalidateCreativeAssetQueryCache } from './creativeAssetQueryCache';
import type {
  WorkshopAssetApi,
  WorkshopAssetDto,
  WorkshopAssetListQuery,
  WorkshopAssetPatch,
  WorkshopAssetUploadMetadata,
} from './api';
import type {
  CreateCreativeTextAsset,
  CreativeAsset,
  CreativeAssetKind,
  CreativeAssetLibraryPort,
  CreativeAssetMetadata,
  CreativeAssetOrigin,
  CreativeAssetPage,
  CreativeAssetPatch,
  CreativePromptAssetPort,
  CreativeAssetQuery,
  CreativeAssetUploadProgress,
  CreativeAssetVariant,
} from './types';

const ASSET_KINDS = new Set<CreativeAssetKind>(['image', 'video', 'audio', 'text']);

function assetKind(value: unknown): CreativeAssetKind {
  if (typeof value === 'string' && ASSET_KINDS.has(value as CreativeAssetKind)) {
    return value as CreativeAssetKind;
  }
  throw new TypeError(`Unknown creative asset kind: ${String(value)}`);
}

function stringArray(value: unknown, field: string): string[] {
  if (!Array.isArray(value) || value.some((entry) => typeof entry !== 'string')) {
    throw new TypeError(`Invalid creative asset ${field}`);
  }
  return [...value];
}

function optionalString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined;
}

function optionalRecord(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? { ...(value as Record<string, unknown>) }
    : undefined;
}

function mapOrigin(value: unknown): CreativeAssetOrigin | null {
  if (value === null || value === undefined) return null;
  if (typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError('Invalid creative asset origin');
  }
  const origin = value as Record<string, unknown>;
  const workbenchKind =
    origin.workbench_kind === 'image' ||
    origin.workbench_kind === 'video' ||
    origin.workbench_kind === 'audio'
      ? origin.workbench_kind
      : undefined;
  const canonicalCanvasId = optionalString(origin.canvas_id);
  const legacyCanvasId = workbenchKind
    ? undefined
    : optionalString(origin.project_id);
  if (
    canonicalCanvasId &&
    legacyCanvasId &&
    canonicalCanvasId !== legacyCanvasId
  ) {
    throw new TypeError('Invalid creative asset Canvas origin');
  }
  const promptLibrarySource =
    origin.prompt_library_source === 'catalog' || origin.prompt_library_source === 'preset'
      ? origin.prompt_library_source
      : undefined;
  const promptLibraryId = optionalString(origin.prompt_library_id);
  if (
    (origin.prompt_library_source !== undefined || origin.prompt_library_id !== undefined) &&
    (!promptLibrarySource || !promptLibraryId?.trim())
  ) {
    throw new TypeError('Invalid creative asset prompt-library origin');
  }
  return {
    prompt: optionalString(origin.prompt),
    model: optionalString(origin.model),
    providerId: optionalString(origin.provider_id),
    params: optionalRecord(origin.params),
    workbenchKind,
    canvasId: canonicalCanvasId ?? legacyCanvasId,
    nodeId: optionalString(origin.node_id),
    generationTaskId: optionalString(origin.creation_task_id),
    promptLibrarySource,
    promptLibraryId,
    promptCatalogId: optionalString(origin.prompt_catalog_id),
    sourceUrl: optionalString(origin.source_url),
    license: optionalString(origin.license),
    licenseUrl: optionalString(origin.license_url),
  };
}

function requireString(value: unknown, field: string): string {
  if (typeof value !== 'string') throw new TypeError(`Invalid creative asset ${field}`);
  return value;
}

function nullableString(value: unknown, field: string): string | null {
  if (value === null) return null;
  return requireString(value, field);
}

function nullableNumber(value: unknown, field: string): number | null {
  if (value === null) return null;
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw new TypeError(`Invalid creative asset ${field}`);
  }
  return value;
}

function requireNumber(value: unknown, field: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw new TypeError(`Invalid creative asset ${field}`);
  }
  return value;
}

function requireCount(value: unknown, field: string): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw new TypeError(`Invalid creative asset ${field}`);
  }
  return value;
}

function requireBoolean(value: unknown, field: string): boolean {
  if (typeof value !== 'boolean') throw new TypeError(`Invalid creative asset ${field}`);
  return value;
}

export function mapWorkshopAsset(dto: WorkshopAssetDto): CreativeAsset {
  return {
    id: parseAssetId(dto.asset_id),
    kind: assetKind(dto.kind),
    title: requireString(dto.title, 'title'),
    collection: nullableString(dto.collection, 'collection'),
    tags: stringArray(dto.tags, 'tags'),
    mimeType: nullableString(dto.mime, 'mime'),
    width: nullableNumber(dto.width, 'width'),
    height: nullableNumber(dto.height, 'height'),
    bytes: nullableNumber(dto.bytes, 'bytes'),
    inLibrary: requireBoolean(dto.in_library, 'in_library'),
    textContent: nullableString(dto.text_content, 'text_content'),
    origin: mapOrigin(dto.origin),
    originalUrl: requireString(dto.url, 'url'),
    thumbnailUrl: nullableString(dto.thumb_url, 'thumb_url'),
    createdAt: requireNumber(dto.created_at, 'created_at'),
    updatedAt: requireNumber(dto.updated_at, 'updated_at'),
    deletedAt: nullableNumber(dto.deleted_at ?? null, 'deleted_at'),
  };
}

export function toWorkshopAssetQuery(query: CreativeAssetQuery = {}): WorkshopAssetListQuery {
  return {
    kind: query.kind,
    collection: query.ungrouped ? undefined : query.collection,
    q: query.search,
    in_library: query.inLibrary,
    ungrouped: query.ungrouped,
    tag: query.tag,
    sort: query.sort,
    page: query.page,
    page_size: query.pageSize,
  };
}

function toWorkshopUploadMetadata(metadata: CreativeAssetMetadata): WorkshopAssetUploadMetadata {
  return {
    title: metadata.title,
    collection: metadata.collection,
    tags: metadata.tags,
    in_library: metadata.inLibrary,
  };
}

function toWorkshopPatch(patch: CreativeAssetPatch): WorkshopAssetPatch {
  return {
    title: patch.title,
    // Rust's `Option<String>` cannot distinguish a missing JSON property from
    // JSON null. Its documented clearing contract is a present blank string.
    collection: patch.collection === null ? '' : patch.collection,
    tags: patch.tags,
    in_library: patch.inLibrary,
  };
}

export class CreativeAssetClient implements CreativeAssetLibraryPort, CreativePromptAssetPort {
  // Deletion is irreversible. Late GET/list/update responses must not restore it.
  private readonly deletedAssets = new Map<string, number>();
  constructor(private readonly api: WorkshopAssetApi = workshopAssetApi) {}

  private recordDeletion(assetId: string, deletedAt: number): void {
    const known = this.deletedAssets.has(assetId);
    this.deletedAssets.set(assetId, deletedAt);
    if (known) return;
    invalidateCreativeAssetQueryCache(this);
    notifyCreativeAssetDeleted(this, assetId);
  }

  private map(dto: WorkshopAssetDto): CreativeAsset {
    const asset = mapWorkshopAsset(dto);
    const assetId = parseAssetId(asset.id);
    if (isCreativeAssetDeleted(asset)) this.recordDeletion(assetId, asset.deletedAt!);
    const deletedAt = this.deletedAssets.get(assetId);
    if (deletedAt !== undefined) {
      return { ...asset, deletedAt, originalUrl: '', thumbnailUrl: null, textContent: null, inLibrary: false };
    }
    return {
      ...asset,
      // DTO paths are backend-relative. Always use the API's URL resolver so
      // desktop webviews target the loopback backend instead of the app bundle.
      originalUrl: this.api.fileUrl(assetId),
      thumbnailUrl: dto.thumb_url ? this.api.fileUrl(assetId, true) : null,
    };
  }

  async list(query: CreativeAssetQuery = {}): Promise<CreativeAssetPage> {
    const page = await this.api.list(toWorkshopAssetQuery(query));
    return {
      items: page.items.map((item) => this.map(item)).filter((asset) => !isCreativeAssetDeleted(asset)),
      total: requireNumber(page.total, 'page total'),
    };
  }

  async get(assetId: string): Promise<CreativeAsset> {
    return this.map(await this.api.get(parseAssetId(assetId)));
  }

  async upload(
    file: File,
    metadata: CreativeAssetMetadata = {},
    signal?: AbortSignal,
    onProgress?: CreativeAssetUploadProgress
  ): Promise<CreativeAsset> {
    return this.map(
      await this.api.upload(file, toWorkshopUploadMetadata(metadata), signal, onProgress)
    );
  }

  async createText(input: CreateCreativeTextAsset): Promise<CreativeAsset> {
    return this.map(
      await this.api.createText({
        kind: 'text',
        title: input.title,
        text_content: input.textContent,
        collection: input.collection,
        tags: input.tags,
        in_library: input.inLibrary,
        origin: input.origin
          ? input.origin.promptLibrarySource === 'catalog'
            ? {
                prompt_library_source: input.origin.promptLibrarySource,
                prompt_library_id: input.origin.promptLibraryId,
                prompt_catalog_id: input.origin.promptCatalogId,
                source_url: input.origin.sourceUrl,
                license: input.origin.license,
                license_url: input.origin.licenseUrl,
              }
            : {
                prompt_library_source: 'preset',
                prompt_library_id: input.origin.promptLibraryId,
              }
          : undefined,
      })
    );
  }

  async removePromptAsset(
    source: 'catalog' | 'preset',
    promptId: string
  ): Promise<number> {
    const response = await this.api.removePromptAsset({
      prompt_library_source: source,
      prompt_library_id: promptId,
    });
    return requireCount(response.matched, 'matched prompt assets');
  }

  async update(assetId: string, patch: CreativeAssetPatch): Promise<CreativeAsset> {
    return this.map(await this.api.update(parseAssetId(assetId), toWorkshopPatch(patch)));
  }

  async remove(assetId: string): Promise<void> {
    const id = parseAssetId(assetId);
    try {
      await this.api.remove(id);
    } catch (reason) {
      // File cleanup can fail after the durable tombstone commits. Confirm it
      // through metadata, notify open views, and keep the original failure so
      // the dialog can retry cleanup for this exact id.
      try { await this.get(id); } catch { /* Unavailable metadata is not proof of deletion. */ }
      throw reason;
    }
    this.recordDeletion(id, Date.now());
  }

  renameCollection(from: string, to: string): Promise<number> {
    return this.api.renameCollection(from, to);
  }

  url(assetId: string, variant: CreativeAssetVariant = 'original'): string {
    return this.api.fileUrl(parseAssetId(assetId), variant === 'thumbnail');
  }
}

export const creativeAssetClient = new CreativeAssetClient();
