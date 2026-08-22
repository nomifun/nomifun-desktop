/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from "../../assets";
import {
  browserCreativeImageCropCodec,
  creativeImageExtensionForMime,
  creativeImageOutputMimeType,
  creativeImageSafeFileStem,
  type CreativeImageCropCodec,
} from "./browserCrop";
import {
  creativeImageSplitPieces,
  type CreativeImageSplitParams,
} from "./splitModel";

export interface SplitCreativeImageAssetInput {
  asset: CreativeAsset;
  params: CreativeImageSplitParams;
  signal?: AbortSignal;
  codec?: CreativeImageCropCodec;
}

export interface SplitCreativeImageFile {
  row: number;
  column: number;
  file: File;
  width: number;
  height: number;
  mimeType: string;
}

/** Decode once, then encode a gapless row-major set of real upload files. */
export async function splitCreativeImageAsset(
  input: SplitCreativeImageAssetInput,
): Promise<SplitCreativeImageFile[]> {
  if (input.asset.kind !== "image") {
    throw new Error("只有真实图片素材可以切图。");
  }
  input.signal?.throwIfAborted();
  const codec = input.codec ?? browserCreativeImageCropCodec;
  const sourceBlob = await codec.load(input.asset.originalUrl, input.signal);
  input.signal?.throwIfAborted();
  const decoded = await codec.decode(sourceBlob);
  try {
    const pieces = creativeImageSplitPieces(input.params, decoded);
    const mimeType = creativeImageOutputMimeType(input.asset);
    const extension = creativeImageExtensionForMime(mimeType);
    const stem = creativeImageSafeFileStem(input.asset.title);
    const files: SplitCreativeImageFile[] = [];
    for (const piece of pieces) {
      input.signal?.throwIfAborted();
      const output = await codec.encode(decoded, piece.crop, mimeType);
      input.signal?.throwIfAborted();
      files.push({
        row: piece.row,
        column: piece.column,
        file: new File(
          [output],
          `${stem}-${piece.row + 1}-${piece.column + 1}.${extension}`,
          { type: mimeType },
        ),
        width: piece.crop.width,
        height: piece.crop.height,
        mimeType,
      });
    }
    return files;
  } finally {
    decoded.close();
  }
}
