/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { parseAssetId } from '@/common/types/ids';

import { workshopAssetApi } from './api';
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
  return {
    prompt: optionalString(origin.prompt),
    model: optionalString(origin.model),
    providerId: optionalString(origin.provider_id),
    params: optionalRecord(origin.params),
    projectId: optionalString(origin.canvas_id),
    nodeId: optionalString(origin.node_id),
    generationTaskId: optionalString(origin.creation_task_id),
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

export class CreativeAssetClient implements CreativeAssetLibraryPort {
  constructor(private readonly api: WorkshopAssetApi = workshopAssetApi) {}

  private map(dto: WorkshopAssetDto): CreativeAsset {
    const asset = mapWorkshopAsset(dto);
    const assetId = parseAssetId(asset.id);
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
      items: page.items.map((item) => this.map(item)),
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
      })
    );
  }

  async update(assetId: string, patch: CreativeAssetPatch): Promise<CreativeAsset> {
    return this.map(await this.api.update(parseAssetId(assetId), toWorkshopPatch(patch)));
  }

  remove(assetId: string): Promise<void> {
    return this.api.remove(parseAssetId(assetId));
  }

  renameCollection(from: string, to: string): Promise<number> {
    return this.api.renameCollection(from, to);
  }

  url(assetId: string, variant: CreativeAssetVariant = 'original'): string {
    return this.api.fileUrl(parseAssetId(assetId), variant === 'thumbnail');
  }
}

export const creativeAssetClient = new CreativeAssetClient();
