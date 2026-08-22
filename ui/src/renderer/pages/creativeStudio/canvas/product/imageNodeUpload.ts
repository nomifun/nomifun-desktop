/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeAsset,
  CreativeAssetPort,
  CreativeAssetUploadProgress,
} from '../../assets';
import { CREATIVE_ASSET_MANUAL_UPLOAD_LIMIT_BYTES } from '../../assets/page/model';
import type { CreativeCanvasNode, CreativeSize } from '../../domain';
import { clearCanvasImageComposeDraftModel } from './canvasImageComposerCanvas';

type ImageNode = Extract<CreativeCanvasNode, { type: 'image' }>;

export const CREATIVE_CANVAS_IMAGE_UPLOAD_MAX_SIZE = 640;

export interface UploadCanvasImageNodeAssetInput {
  port: CreativeAssetPort;
  file: File;
  operationId: string;
  signal?: AbortSignal;
  onProgress?: CreativeAssetUploadProgress;
}

export interface UploadCanvasImageNodeAssetResult {
  asset: CreativeAsset;
  recoveredAfterResponseLoss: boolean;
}

const positive = (value: number | null): value is number =>
  value !== null && Number.isFinite(value) && value > 0;

const operationTag = (operationId: string): string =>
  `canvas-image-node-upload:${operationId}`;

const requireImageAsset = (asset: CreativeAsset): CreativeAsset => {
  if (asset.kind !== 'image' || !asset.id.trim()) {
    throw new Error('上传结果不是有效的真实图片素材。');
  }
  return asset;
};

export async function uploadCanvasImageNodeAsset(
  input: UploadCanvasImageNodeAssetInput
): Promise<UploadCanvasImageNodeAssetResult> {
  if (!input.file.type.trim().toLocaleLowerCase().startsWith('image/')) {
    throw new Error('该节点只接受真实图片文件。');
  }
  if (input.file.size > CREATIVE_ASSET_MANUAL_UPLOAD_LIMIT_BYTES) {
    throw new Error('单个图片不能超过 64 MB。');
  }
  const tag = operationTag(input.operationId);
  try {
    const asset = requireImageAsset(
      await input.port.upload(
        input.file,
        {
          title: input.file.name,
          tags: ['canvas-node-upload', tag],
          inLibrary: true,
        },
        input.signal,
        input.onProgress
      )
    );
    return { asset, recoveredAfterResponseLoss: false };
  } catch (uploadError) {
    if (
      input.signal?.aborted ||
      (uploadError instanceof Error && uploadError.name === 'AbortError')
    ) {
      throw uploadError;
    }
    try {
      const page = await input.port.list({
        kind: 'image',
        tag,
        sort: 'created_desc',
        page: 1,
        pageSize: 2,
      });
      const recovered = page.items.find((asset) => asset.tags.includes(tag));
      if (recovered) {
        return {
          asset: requireImageAsset(recovered),
          recoveredAfterResponseLoss: true,
        };
      }
    } catch {
      // Preserve the authoritative upload failure when reconciliation is unavailable.
    }
    throw uploadError;
  }
}

export function uploadedCanvasImageNodeSize(
  asset: CreativeAsset,
  fallback: CreativeSize
): CreativeSize {
  if (!positive(asset.width) || !positive(asset.height)) return { ...fallback };
  const scale = Math.min(
    1,
    CREATIVE_CANVAS_IMAGE_UPLOAD_MAX_SIZE / asset.width,
    CREATIVE_CANVAS_IMAGE_UPLOAD_MAX_SIZE / asset.height
  );
  return {
    width: asset.width * scale,
    height: asset.height * scale,
  };
}

/** Fill one still-empty image node with a real uploaded NomiFun asset. */
export function fillEmptyCanvasImageNodeFromAsset(
  node: ImageNode,
  asset: CreativeAsset
): ImageNode {
  if (node.data.assetId !== null) {
    throw new Error('图片节点已关联素材，未覆盖现有内容。');
  }
  if (asset.kind !== 'image' || !asset.id.trim()) {
    throw new Error('上传结果不是有效的真实图片素材。');
  }
  const naturalSize =
    positive(asset.width) && positive(asset.height)
      ? { width: asset.width, height: asset.height }
      : null;
  return clearCanvasImageComposeDraftModel({
    ...node,
    size: uploadedCanvasImageNodeSize(asset, node.size),
    data: {
      ...node.data,
      assetId: asset.id,
      caption: asset.title,
      alt: asset.title,
      fit: 'contain',
      naturalSize,
    },
  });
}
