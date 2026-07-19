/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageToolGroup } from '@/common/chat/chatLib';
import { optionalDisplayText, toDisplayText } from '@/common/chat/displayText';

type ToolGroupItem = IMessageToolGroup['content'][number];

export type SuccessfulLegacyImage = {
  imgUrl: string;
  relativePath?: string;
};

const LEGACY_UNVERIFIED_IMAGE_MESSAGE =
  'Legacy image result was not backed by a committed artifact receipt';
const SHA256_RE = /^[a-f\d]{64}$/i;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);

const legacyImageClaim = (item: ToolGroupItem): { imgUrl: string; relativePath?: string } | undefined => {
  if (!isRecord(item.result_display) || !('img_url' in item.result_display)) return undefined;
  const imgUrl = optionalDisplayText(item.result_display.img_url);
  if (!imgUrl) return undefined;
  return {
    imgUrl,
    relativePath: optionalDisplayText(item.result_display.relative_path),
  };
};

/**
 * Old ToolGroup payloads predate durable artifact receipts. Extra fields are
 * intentionally inspected as untrusted data so a future migration can retain
 * an image only when both the 2PC marker and a complete matching receipt are
 * present. The receipt path, not `result_display.img_url`, becomes the source.
 */
const committedLegacyImage = (item: ToolGroupItem): SuccessfulLegacyImage | undefined => {
  const claim = legacyImageClaim(item);
  if (!claim) return undefined;
  const carrier = item as ToolGroupItem & {
    artifact_delivery_committed?: unknown;
    artifacts?: unknown;
  };
  if (carrier.artifact_delivery_committed !== true || !Array.isArray(carrier.artifacts)) {
    return undefined;
  }

  for (const value of carrier.artifacts) {
    if (!isRecord(value)) continue;
    const path = optionalDisplayText(value.path);
    const relativePath = optionalDisplayText(value.relative_path);
    const mimeType = optionalDisplayText(value.mime_type);
    const sha256 = optionalDisplayText(value.sha256);
    if (
      value.kind !== 'image' ||
      !path ||
      !relativePath ||
      !mimeType?.toLowerCase().startsWith('image/') ||
      typeof value.size_bytes !== 'number' ||
      !Number.isSafeInteger(value.size_bytes) ||
      value.size_bytes <= 0 ||
      !sha256 ||
      !SHA256_RE.test(sha256) ||
      (claim.imgUrl !== path && claim.imgUrl !== relativePath) ||
      (claim.relativePath !== undefined && claim.relativePath !== relativePath)
    ) {
      continue;
    }
    return { imgUrl: path, relativePath };
  }
  return undefined;
};

/**
 * A terminal label alone is not delivery proof. Legacy image URLs are rendered
 * only when a matching full receipt and the turn-level 2PC marker accompany
 * the same item.
 */
export const getSuccessfulLegacyImage = (item: ToolGroupItem): SuccessfulLegacyImage | undefined => {
  if (toDisplayText(item.status) !== 'Success') return undefined;
  return committedLegacyImage(item);
};

/** Strip receipt-less image claims and turn legacy false success into Error. */
export const enforceToolGroupArtifactTrust = (item: ToolGroupItem): ToolGroupItem => {
  if (!legacyImageClaim(item) || committedLegacyImage(item)) return item;
  const isFalseSuccess = toDisplayText(item.status) === 'Success';
  const description = optionalDisplayText(item.description);
  return {
    ...item,
    ...(isFalseSuccess ? { status: 'Error' as const } : {}),
    description: isFalseSuccess
      ? description
        ? `${description}: ${LEGACY_UNVERIFIED_IMAGE_MESSAGE}`
        : LEGACY_UNVERIFIED_IMAGE_MESSAGE
      : (description ?? ''),
    result_display: undefined,
  };
};

/** File-change result cards likewise represent output and require Success. */
export const isSuccessfulWriteFileResult = (item: ToolGroupItem): boolean =>
  toDisplayText(item.status) === 'Success' &&
  toDisplayText(item.name) === 'WriteFile' &&
  Boolean(
    item.result_display &&
      typeof item.result_display === 'object' &&
      'file_diff' in item.result_display
  );
