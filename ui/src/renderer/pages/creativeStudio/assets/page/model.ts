/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset, CreativeAssetKind, CreativeAssetQuery } from '../types';
import type { CreativeAssetKindFilter, CreativeTextAssetFormValue } from '../components';

export const CREATIVE_ASSET_MANUAL_UPLOAD_LIMIT_BYTES = 64 * 1024 * 1024;
export const CREATIVE_ASSET_MANUAL_UPLOAD_ACCEPT = 'image/*,video/*';

export type CreativeAssetUploadRejection =
  | 'audio_unsupported'
  | 'file_too_large'
  | 'unsupported_media_type';

export interface CreativeAssetUploadCandidate {
  name: string;
  size: number;
  type: string;
}

export interface CreativeAssetUploadValidation {
  accepted: boolean;
  rejection: CreativeAssetUploadRejection | null;
}

export interface CreativeAssetEditDraft {
  title: string;
  collection: string;
  tags: string[];
  inLibrary: boolean;
}

export interface CreativeCollectionRenameDraft {
  from: string;
  to: string;
}

export const EMPTY_CREATIVE_TEXT_ASSET_FORM: CreativeTextAssetFormValue = {
  title: '',
  textContent: '',
  collection: '',
  tags: [],
  inLibrary: true,
};

const uniqueTrimmedTags = (tags: readonly string[]): string[] => {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const rawTag of tags) {
    const tag = rawTag.trim();
    if (!tag || seen.has(tag)) continue;
    seen.add(tag);
    result.push(tag);
  }
  return result;
};

export function buildGlobalCreativeAssetQuery(
  search: string,
  kind: CreativeAssetKindFilter
): Omit<CreativeAssetQuery, 'page' | 'pageSize'> {
  const normalizedSearch = search.trim();
  return {
    inLibrary: true,
    kind: kind === 'all' ? undefined : kind,
    search: normalizedSearch || undefined,
    sort: 'updated_desc',
  };
}

export function validateCreativeAssetManualUpload(
  file: CreativeAssetUploadCandidate
): CreativeAssetUploadValidation {
  if (file.size > CREATIVE_ASSET_MANUAL_UPLOAD_LIMIT_BYTES) {
    return { accepted: false, rejection: 'file_too_large' };
  }

  const mime = file.type.trim().toLocaleLowerCase();
  if (mime.startsWith('audio/')) {
    return { accepted: false, rejection: 'audio_unsupported' };
  }
  if (!mime.startsWith('image/') && !mime.startsWith('video/')) {
    return { accepted: false, rejection: 'unsupported_media_type' };
  }
  return { accepted: true, rejection: null };
}

export function manualUploadRejectionMessage(rejection: CreativeAssetUploadRejection): string {
  switch (rejection) {
    case 'audio_unsupported':
      return '暂不支持手动上传音频；通过音频工作台生成的音频仍会进入素材库。';
    case 'file_too_large':
      return '单个素材不能超过 64 MB。';
    case 'unsupported_media_type':
      return '手动上传仅支持图片和视频文件。';
  }
}

export function creativeAssetEditDraft(asset: CreativeAsset): CreativeAssetEditDraft {
  return {
    title: asset.title,
    collection: asset.collection ?? '',
    tags: [...asset.tags],
    inLibrary: asset.inLibrary,
  };
}

export function normalizeCreativeAssetEditDraft(draft: CreativeAssetEditDraft): CreativeAssetEditDraft {
  return {
    title: draft.title.trim(),
    collection: draft.collection.trim(),
    tags: uniqueTrimmedTags(draft.tags),
    inLibrary: draft.inLibrary,
  };
}

export function normalizeCreativeTextAssetForm(
  value: CreativeTextAssetFormValue
): CreativeTextAssetFormValue {
  return {
    title: value.title.trim(),
    textContent: value.textContent.trim(),
    collection: value.collection.trim(),
    tags: uniqueTrimmedTags(value.tags),
    inLibrary: value.inLibrary,
  };
}

export function validateCreativeCollectionRename(
  draft: CreativeCollectionRenameDraft
): string | null {
  const from = draft.from.trim();
  const to = draft.to.trim();
  if (!from) return '请输入当前合集名称。';
  if (from === to) return '新合集名称需要与当前名称不同。';
  return null;
}

export function creativeAssetDownloadName(asset: Pick<CreativeAsset, 'title' | 'mimeType' | 'kind'>): string {
  const safeTitle = asset.title.trim().replace(/[\\/:*?"<>|]+/g, '-').replace(/[. ]+$/g, '') || 'asset';
  const mimeExtension = asset.mimeType?.split('/')[1]?.split(/[;+]/)[0]?.trim().toLocaleLowerCase();
  const extension = mimeExtension === 'jpeg'
    ? 'jpg'
    : mimeExtension === 'quicktime'
      ? 'mov'
      : mimeExtension || fallbackExtension(asset.kind);
  return `${safeTitle}.${extension}`;
}

function fallbackExtension(kind: CreativeAssetKind): string {
  switch (kind) {
    case 'image':
      return 'png';
    case 'video':
      return 'mp4';
    case 'audio':
      return 'mp3';
    case 'text':
      return 'txt';
  }
}
