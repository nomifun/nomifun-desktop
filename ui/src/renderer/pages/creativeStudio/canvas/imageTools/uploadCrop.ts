/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeAsset,
  CreativeAssetPort,
  CreativeAssetUploadProgress,
} from "../../assets";
import { translateCreativeImageTool } from "./imageToolI18n";

export interface UploadCreativeImageCropInput {
  port: CreativeAssetPort;
  source: CreativeAsset;
  file: File;
  operationId: string;
  signal?: AbortSignal;
  onProgress?: CreativeAssetUploadProgress;
}

export interface UploadCreativeImageCropResult {
  asset: CreativeAsset;
  recoveredAfterResponseLoss: boolean;
}

const operationTag = (operationId: string): string =>
  `canvas-crop-operation:${operationId}`;

const requireImage = (asset: CreativeAsset): CreativeAsset => {
  if (asset.kind !== "image") {
    throw new Error(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.cropUploadNotImage",
      ),
    );
  }
  return asset;
};

/**
 * Upload a crop with a unique durable tag. If the POST response is lost after
 * the server commits, the exact tag lets the client reconcile the real asset
 * without uploading a duplicate on the same attempt.
 */
export async function uploadCreativeImageCrop(
  input: UploadCreativeImageCropInput,
): Promise<UploadCreativeImageCropResult> {
  const tag = operationTag(input.operationId);
  const title = translateCreativeImageTool(
    "creativeStudio.canvas.imageTools.assetTitles.crop",
    { title: input.source.title },
  );
  const tags = [...new Set([...input.source.tags, "canvas-crop", tag])];
  try {
    const asset = requireImage(
      await input.port.upload(
        input.file,
        {
          title,
          ...(input.source.collection
            ? { collection: input.source.collection }
            : {}),
          tags,
          inLibrary: true,
        },
        input.signal,
        input.onProgress,
      ),
    );
    return { asset, recoveredAfterResponseLoss: false };
  } catch (uploadError) {
    if (
      input.signal?.aborted ||
      (uploadError instanceof Error && uploadError.name === "AbortError")
    ) {
      throw uploadError;
    }
    try {
      const page = await input.port.list({
        kind: "image",
        tag,
        sort: "created_desc",
        page: 1,
        pageSize: 2,
      });
      const recovered = page.items.find((asset) => asset.tags.includes(tag));
      if (recovered) {
        return {
          asset: requireImage(recovered),
          recoveredAfterResponseLoss: true,
        };
      }
    } catch {
      // Preserve the authoritative upload failure when reconciliation is also unavailable.
    }
    throw uploadError;
  }
}
