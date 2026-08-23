/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset, CreativeAssetPort } from "../../assets";
import type { SplitCreativeImageFile } from "./browserSplit";
import { translateCreativeImageTool } from "./imageToolI18n";

export interface UploadedCreativeImageSplitPiece {
  row: number;
  column: number;
  width: number;
  height: number;
  asset: CreativeAsset;
  recoveredAfterResponseLoss: boolean;
}

export interface UploadCreativeImageSplitInput {
  port: CreativeAssetPort;
  source: CreativeAsset;
  pieces: readonly SplitCreativeImageFile[];
  operationId: string;
  signal?: AbortSignal;
  concurrency?: number;
  onProgress?(percent: number): void;
}

const requireImage = (asset: CreativeAsset): CreativeAsset => {
  if (asset.kind !== "image") {
    throw new Error(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.splitUploadNotImage",
      ),
    );
  }
  return asset;
};

const splitOperationTag = (
  operationId: string,
  row: number,
  column: number,
): string => `canvas-split-operation:${operationId}:${row + 1}-${column + 1}`;

const uploadOne = async (
  input: UploadCreativeImageSplitInput,
  piece: SplitCreativeImageFile,
  onProgress: (percent: number) => void,
): Promise<UploadedCreativeImageSplitPiece> => {
  const tag = splitOperationTag(input.operationId, piece.row, piece.column);
  const title = `${input.source.title} ${piece.row + 1}-${piece.column + 1}`;
  const tags = [...new Set([...input.source.tags, "canvas-split", tag])];
  try {
    const asset = requireImage(
      await input.port.upload(
        piece.file,
        {
          title,
          ...(input.source.collection
            ? { collection: input.source.collection }
            : {}),
          tags,
          inLibrary: true,
        },
        input.signal,
        onProgress,
      ),
    );
    return {
      row: piece.row,
      column: piece.column,
      width: piece.width,
      height: piece.height,
      asset,
      recoveredAfterResponseLoss: false,
    };
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
          row: piece.row,
          column: piece.column,
          width: piece.width,
          height: piece.height,
          asset: requireImage(recovered),
          recoveredAfterResponseLoss: true,
        };
      }
    } catch {
      // Preserve the authoritative upload failure when reconciliation is unavailable.
    }
    throw uploadError;
  }
};

const rollbackUploaded = async (
  port: CreativeAssetPort,
  pieces: readonly UploadedCreativeImageSplitPiece[],
): Promise<number> => {
  const ids = [...new Set(pieces.map((piece) => piece.asset.id))];
  const removals = await Promise.allSettled(
    ids.map((assetId) => port.remove(assetId)),
  );
  return removals.filter((result) => result.status === "rejected").length;
};

/**
 * Upload a split with bounded concurrency and all-known-output compensation.
 * Every piece has an operation tag so response loss is reconciled per file.
 */
export async function uploadCreativeImageSplit(
  input: UploadCreativeImageSplitInput,
): Promise<UploadedCreativeImageSplitPiece[]> {
  if (input.source.kind !== "image") {
    throw new Error(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.imageRequiredForSplit",
      ),
    );
  }
  if (input.pieces.length === 0) {
    throw new Error(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.noSplitPieces",
      ),
    );
  }
  input.signal?.throwIfAborted();
  const concurrency = Math.min(
    4,
    Math.max(1, Math.round(input.concurrency ?? 3)),
    input.pieces.length,
  );
  const progress = Array.from({ length: input.pieces.length }, () => 0);
  let lastProgress = 0;
  const publishProgress = (index: number, percent: number) => {
    progress[index] = Math.min(100, Math.max(progress[index], percent));
    const next =
      progress.reduce((sum, value) => sum + value, 0) / progress.length;
    lastProgress = Math.max(lastProgress, next);
    input.onProgress?.(lastProgress);
  };
  const results = new Array<UploadedCreativeImageSplitPiece | undefined>(
    input.pieces.length,
  );
  let nextIndex = 0;
  let failed = false;
  let firstFailure: unknown;

  const worker = async () => {
    while (!failed) {
      const index = nextIndex;
      if (index >= input.pieces.length) return;
      nextIndex += 1;
      try {
        results[index] = await uploadOne(
          input,
          input.pieces[index],
          (percent) => publishProgress(index, percent),
        );
        publishProgress(index, 100);
      } catch (error) {
        if (!failed) {
          failed = true;
          firstFailure = error;
        }
      }
    }
  };

  await Promise.all(Array.from({ length: concurrency }, () => worker()));
  const uploaded = results.filter(
    (piece): piece is UploadedCreativeImageSplitPiece => piece !== undefined,
  );
  if (failed) {
    const cleanupFailures = await rollbackUploaded(input.port, uploaded);
    const message =
      firstFailure instanceof Error
        ? firstFailure.message
        : String(firstFailure);
    throw new Error(
      cleanupFailures > 0
        ? translateCreativeImageTool(
            "creativeStudio.canvas.imageTools.errors.splitCleanupFailedWithCause",
            { message, count: cleanupFailures },
          )
        : message,
      { cause: firstFailure },
    );
  }
  input.onProgress?.(100);
  return results as UploadedCreativeImageSplitPiece[];
}

export async function removeUploadedCreativeImageSplit(
  port: CreativeAssetPort,
  pieces: readonly UploadedCreativeImageSplitPiece[],
): Promise<void> {
  const failures = await rollbackUploaded(port, pieces);
  if (failures > 0) {
    throw new Error(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.splitCleanupFailed",
        { count: failures },
      ),
    );
  }
}
