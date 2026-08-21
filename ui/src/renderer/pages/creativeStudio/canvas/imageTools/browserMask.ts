/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from "../../assets";
import {
  browserCreativeImageCropCodec,
  creativeImageCanvasToBlob,
  creativeImageSafeFileStem,
  type DecodedCreativeImage,
} from "./browserCrop";
import { CREATIVE_IMAGE_MASK_FILL } from "./maskModel";

export interface CreativeImageMaskSelection {
  width: number;
  height: number;
  source: CanvasImageSource;
}

export interface CreativeImageMaskCodec {
  load(url: string, signal?: AbortSignal): Promise<Blob>;
  decode(blob: Blob): Promise<DecodedCreativeImage>;
  encodeMarked(
    image: DecodedCreativeImage,
    selection: CreativeImageMaskSelection,
  ): Promise<Blob>;
}

export interface BuildCreativeImageMaskReferenceInput {
  asset: CreativeAsset;
  selection: CreativeImageMaskSelection;
  signal?: AbortSignal;
  codec?: CreativeImageMaskCodec;
}

export interface CreativeImageMaskReferenceFile {
  file: File;
  width: number;
  height: number;
  mimeType: "image/png";
}

export const browserCreativeImageMaskCodec: CreativeImageMaskCodec = {
  load: browserCreativeImageCropCodec.load,
  decode: browserCreativeImageCropCodec.decode,

  async encodeMarked(image, selection) {
    const canvas = document.createElement("canvas");
    canvas.width = image.width;
    canvas.height = image.height;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("浏览器无法创建局部编辑参考画布。");
    context.drawImage(image.source, 0, 0, canvas.width, canvas.height);
    context.fillStyle = CREATIVE_IMAGE_MASK_FILL;
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.globalCompositeOperation = "destination-in";
    context.drawImage(selection.source, 0, 0, canvas.width, canvas.height);
    context.globalCompositeOperation = "destination-over";
    context.drawImage(image.source, 0, 0, canvas.width, canvas.height);
    context.globalCompositeOperation = "source-over";
    return creativeImageCanvasToBlob(
      canvas,
      "image/png",
      "浏览器未能编码局部编辑参考图。",
    );
  },
};

/** Build the real blue-marked i2i reference consumed by image_edit. */
export async function buildCreativeImageMaskReference(
  input: BuildCreativeImageMaskReferenceInput,
): Promise<CreativeImageMaskReferenceFile> {
  if (input.asset.kind !== "image") {
    throw new Error("只有真实图片素材可以进行局部编辑。");
  }
  input.signal?.throwIfAborted();
  const codec = input.codec ?? browserCreativeImageMaskCodec;
  const source = await codec.load(input.asset.originalUrl, input.signal);
  input.signal?.throwIfAborted();
  const decoded = await codec.decode(source);
  try {
    input.signal?.throwIfAborted();
    if (
      input.selection.width !== decoded.width ||
      input.selection.height !== decoded.height
    ) {
      throw new Error("局部编辑遮罩尺寸与原图不一致，请重新打开编辑器。");
    }
    const output = await codec.encodeMarked(decoded, input.selection);
    input.signal?.throwIfAborted();
    return {
      file: new File(
        [output],
        `${creativeImageSafeFileStem(input.asset.title)}-mask-edit-reference.png`,
        { type: "image/png" },
      ),
      width: decoded.width,
      height: decoded.height,
      mimeType: "image/png",
    };
  } finally {
    decoded.close();
  }
}
