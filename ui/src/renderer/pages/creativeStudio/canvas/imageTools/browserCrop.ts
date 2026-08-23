/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from "../../assets";
import {
  creativeImageCropToPixels,
  type CreativeImageCropPixels,
  type CreativeImageCropRect,
} from "./cropModel";
import { translateCreativeImageTool } from "./imageToolI18n";

export interface DecodedCreativeImage {
  width: number;
  height: number;
  source: CanvasImageSource;
  close(): void;
}

export interface CreativeImageCropCodec {
  load(url: string, signal?: AbortSignal): Promise<Blob>;
  decode(blob: Blob): Promise<DecodedCreativeImage>;
  encode(
    image: DecodedCreativeImage,
    crop: CreativeImageCropPixels,
    mimeType: string,
  ): Promise<Blob>;
}

export interface CropCreativeImageAssetInput {
  asset: CreativeAsset;
  crop: CreativeImageCropRect;
  signal?: AbortSignal;
  codec?: CreativeImageCropCodec;
}

export interface CroppedCreativeImageFile {
  file: File;
  width: number;
  height: number;
  mimeType: string;
}

export const creativeImageOutputMimeType = (asset: CreativeAsset): string => {
  switch (asset.mimeType?.toLowerCase()) {
    case "image/jpeg":
      return "image/jpeg";
    case "image/webp":
      return "image/webp";
    default:
      return "image/png";
  }
};

export const creativeImageExtensionForMime = (mimeType: string): string => {
  switch (mimeType) {
    case "image/jpeg":
      return "jpg";
    case "image/webp":
      return "webp";
    default:
      return "png";
  }
};

export const creativeImageSafeFileStem = (title: string): string =>
  (title.trim() ||
    translateCreativeImageTool(
      "creativeStudio.canvas.imageTools.fileNames.untitledImage",
    ))
    .replace(/[<>:"/\\|?*\u0000-\u001f]/gu, "-")
    .replace(/\s+/gu, " ")
    .slice(0, 80);

export const creativeImageCanvasToBlob = (
  canvas: HTMLCanvasElement,
  mimeType: string,
  failureMessage?: string,
): Promise<Blob> =>
  new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => {
        if (blob) resolve(blob);
        else {
          reject(
            new Error(
              failureMessage ??
                translateCreativeImageTool(
                  "creativeStudio.canvas.imageTools.errors.encodeCropFailed",
                ),
            ),
          );
        }
      },
      mimeType,
      mimeType === "image/png" ? undefined : 0.92,
    );
  });

export const browserCreativeImageCropCodec: CreativeImageCropCodec = {
  async load(url, signal) {
    const response = await fetch(url, {
      method: "GET",
      credentials: "include",
      ...(signal ? { signal } : {}),
    });
    if (!response.ok) {
      throw new Error(
        translateCreativeImageTool(
          "creativeStudio.canvas.imageTools.errors.loadSourceFailed",
          { status: response.status },
        ),
      );
    }
    return response.blob();
  },

  async decode(blob) {
    const bitmap = await createImageBitmap(blob);
    return {
      width: bitmap.width,
      height: bitmap.height,
      source: bitmap,
      close: () => bitmap.close(),
    };
  },

  async encode(image, crop, mimeType) {
    const canvas = document.createElement("canvas");
    canvas.width = crop.width;
    canvas.height = crop.height;
    const context = canvas.getContext("2d", {
      alpha: mimeType !== "image/jpeg",
    });
    if (!context) {
      throw new Error(
        translateCreativeImageTool(
          "creativeStudio.canvas.imageTools.errors.createCropCanvasFailed",
        ),
      );
    }
    context.drawImage(
      image.source,
      crop.x,
      crop.y,
      crop.width,
      crop.height,
      0,
      0,
      crop.width,
      crop.height,
    );
    return creativeImageCanvasToBlob(canvas, mimeType);
  },
};

/**
 * Decode and crop the authoritative NomiFun asset into a new uploadable file.
 * The source asset is never mutated and every decoded browser resource is
 * closed on success, failure, and cancellation.
 */
export async function cropCreativeImageAsset(
  input: CropCreativeImageAssetInput,
): Promise<CroppedCreativeImageFile> {
  if (input.asset.kind !== "image") {
    throw new Error(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.imageRequiredForCrop",
      ),
    );
  }
  input.signal?.throwIfAborted();
  const codec = input.codec ?? browserCreativeImageCropCodec;
  const sourceBlob = await codec.load(input.asset.originalUrl, input.signal);
  input.signal?.throwIfAborted();
  const decoded = await codec.decode(sourceBlob);
  try {
    input.signal?.throwIfAborted();
    const pixels = creativeImageCropToPixels(input.crop, decoded);
    const mimeType = creativeImageOutputMimeType(input.asset);
    const output = await codec.encode(decoded, pixels, mimeType);
    input.signal?.throwIfAborted();
    const name = translateCreativeImageTool(
      "creativeStudio.canvas.imageTools.fileNames.crop",
      {
        stem: creativeImageSafeFileStem(input.asset.title),
        extension: creativeImageExtensionForMime(mimeType),
      },
    );
    return {
      file: new File([output], name, { type: mimeType }),
      width: pixels.width,
      height: pixels.height,
      mimeType,
    };
  } finally {
    decoded.close();
  }
}
