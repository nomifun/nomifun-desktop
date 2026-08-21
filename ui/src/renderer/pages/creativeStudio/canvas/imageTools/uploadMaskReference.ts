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

export interface UploadCreativeImageMaskReferenceInput {
  port: CreativeAssetPort;
  source: CreativeAsset;
  file: File;
  operationId: string;
  signal?: AbortSignal;
  onProgress?: CreativeAssetUploadProgress;
}

export interface UploadCreativeImageMaskReferenceResult {
  asset: CreativeAsset;
  recoveredAfterResponseLoss: boolean;
}

const operationTag = (operationId: string): string =>
  `canvas-mask-edit-operation:${operationId}`;

const requireHiddenReference = (
  asset: CreativeAsset,
  tag: string,
): CreativeAsset => {
  if (asset.kind !== "image") {
    throw new Error("局部编辑参考图上传返回了非图片素材，已停止生成。");
  }
  if (asset.inLibrary || !asset.tags.includes(tag)) {
    throw new Error("局部编辑参考图未按隐藏操作标签保存，已停止生成。");
  }
  return asset;
};

/**
 * Upload a hidden, durable marked reference. The operation tag reconciles a
 * server commit whose HTTP response was lost without submitting a duplicate.
 */
export async function uploadCreativeImageMaskReference(
  input: UploadCreativeImageMaskReferenceInput,
): Promise<UploadCreativeImageMaskReferenceResult> {
  if (input.source.kind !== "image") {
    throw new Error("只有真实图片素材可以上传局部编辑参考图。");
  }
  const tag = operationTag(input.operationId);
  const tags = [
    ...new Set([
      ...input.source.tags,
      "canvas-mask-edit-reference",
      tag,
    ]),
  ];
  try {
    const asset = requireHiddenReference(
      await input.port.upload(
        input.file,
        {
          title: `${input.source.title} · 局部编辑标记`,
          ...(input.source.collection
            ? { collection: input.source.collection }
            : {}),
          tags,
          inLibrary: false,
        },
        input.signal,
        input.onProgress,
      ),
      tag,
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
        inLibrary: false,
        sort: "created_desc",
        page: 1,
        pageSize: 2,
      });
      const recovered = page.items.find((asset) => asset.tags.includes(tag));
      if (recovered) {
        return {
          asset: requireHiddenReference(recovered, tag),
          recoveredAfterResponseLoss: true,
        };
      }
    } catch {
      // Preserve the authoritative upload failure if reconciliation also fails.
    }
    throw uploadError;
  }
}

export async function removeCreativeImageMaskReference(
  port: CreativeAssetPort,
  asset: CreativeAsset,
): Promise<void> {
  const tagged = asset.tags.some((tag) =>
    tag.startsWith("canvas-mask-edit-operation:"),
  );
  if (asset.kind !== "image" || asset.inLibrary || !tagged) {
    throw new Error("拒绝清理未验证的局部编辑参考素材。");
  }
  await port.remove(asset.id);
}
