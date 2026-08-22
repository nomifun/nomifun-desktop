/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from "../../assets";
import type { CreativeCanvasNode, CreativeProjectDocument } from "../../domain";
import {
  isCreativeProjectRepositoryError,
  type CreativeProjectRepository,
} from "../../services";
import {
  CREATIVE_CANVAS_PRODUCT_NODE_SIZES,
  creativeNodeFromAsset,
} from "../../canvas/product/nodeFactory";
import type { DirectorProjectBaseline } from "./directorProjectPersistence";

const CAPTURE_COLUMN_GAP = 40;
const CAPTURE_ROW_GAP = 40;
const DIRECTOR_CAPTURE_OFFSET = 80;

export type DirectorCanvasTransferErrorCode =
  | "empty-transfer"
  | "duplicate-capture"
  | "capture-not-found"
  | "capture-asset-mismatch"
  | "capture-not-image";

export class DirectorCanvasTransferError extends Error {
  readonly code: DirectorCanvasTransferErrorCode;

  constructor(code: DirectorCanvasTransferErrorCode, message: string) {
    super(message);
    this.name = "DirectorCanvasTransferError";
    this.code = code;
  }
}

export interface DirectorCaptureAssetTransfer {
  captureId: string;
  asset: CreativeAsset;
}

export interface DirectorCanvasTransferPlan {
  document: CreativeProjectDocument;
  insertedNodes: Extract<CreativeCanvasNode, { type: "image" }>[];
  existingAssetIds: string[];
}

export interface TransferDirectorCapturesInput {
  baseline: DirectorProjectBaseline;
  captures: readonly DirectorCaptureAssetTransfer[];
  repository: CreativeProjectRepository;
}

export interface TransferDirectorCapturesResult extends DirectorCanvasTransferPlan {
  baseline: DirectorProjectBaseline;
}

export type DirectorCanvasTransferOutcome =
  | {
      status: "inserted" | "already-present" | "confirmed-after-response-loss";
      result: TransferDirectorCapturesResult;
      error: null;
    }
  | {
      status: "conflict" | "failed";
      result: TransferDirectorCapturesResult;
      error: Error;
    };

export interface ReconciledDirectorCaptureTransferInput extends TransferDirectorCapturesInput {
  reloadBaseline(): Promise<DirectorProjectBaseline>;
}

const directorNode = (
  baseline: DirectorProjectBaseline,
): Extract<CreativeCanvasNode, { type: "director" }> | null => {
  if (!baseline.directorNodeId) return null;
  return (
    baseline.document.nodes.find(
      (node): node is Extract<CreativeCanvasNode, { type: "director" }> =>
        node.id === baseline.directorNodeId && node.type === "director",
    ) ?? null
  );
};

const transferOrigin = (
  baseline: DirectorProjectBaseline,
): { x: number; y: number } => {
  const director = directorNode(baseline);
  if (director) {
    return {
      x: director.position.x + director.size.width + DIRECTOR_CAPTURE_OFFSET,
      y: director.position.y,
    };
  }
  if (baseline.document.nodes.length === 0) return { x: 0, y: 0 };
  const right = Math.max(
    ...baseline.document.nodes.map((node) => node.position.x + node.size.width),
  );
  const top = Math.min(
    ...baseline.document.nodes.map((node) => node.position.y),
  );
  return { x: right + DIRECTOR_CAPTURE_OFFSET, y: top };
};

function validateTransfers(
  baseline: DirectorProjectBaseline,
  captures: readonly DirectorCaptureAssetTransfer[],
): void {
  if (captures.length === 0) {
    throw new DirectorCanvasTransferError(
      "empty-transfer",
      "没有可发送到画布的截图。",
    );
  }
  const seen = new Set<string>();
  for (const transfer of captures) {
    if (seen.has(transfer.captureId)) {
      throw new DirectorCanvasTransferError(
        "duplicate-capture",
        `截图 ${transfer.captureId} 在同一次发送中重复。`,
      );
    }
    seen.add(transfer.captureId);
    const record = baseline.state.capture.records.find(
      (candidate) =>
        candidate.id === transfer.captureId && candidate.kind === "image",
    );
    if (!record || record.kind !== "image") {
      throw new DirectorCanvasTransferError(
        "capture-not-found",
        `导演场景中不存在图片截图 ${transfer.captureId}。`,
      );
    }
    if (record.assetId !== transfer.asset.id) {
      throw new DirectorCanvasTransferError(
        "capture-asset-mismatch",
        `截图 ${transfer.captureId} 的素材引用已经变化。`,
      );
    }
    if (transfer.asset.kind !== "image") {
      throw new DirectorCanvasTransferError(
        "capture-not-image",
        `截图 ${transfer.captureId} 没有解析为真实图片素材。`,
      );
    }
  }
}

export function directorCaptureAssetsPresent(
  document: CreativeProjectDocument,
  assetIds: readonly string[],
): boolean {
  const imageAssetIds = new Set(
    document.nodes.flatMap((node) =>
      node.type === "image" && node.data.assetId ? [node.data.assetId] : [],
    ),
  );
  return assetIds.every((assetId) => imageAssetIds.has(assetId));
}

function intersectsExistingNode(
  document: CreativeProjectDocument,
  position: { x: number; y: number },
): boolean {
  const width = CREATIVE_CANVAS_PRODUCT_NODE_SIZES.image.width;
  const height = CREATIVE_CANVAS_PRODUCT_NODE_SIZES.image.height;
  return document.nodes.some(
    (node) =>
      position.x < node.position.x + node.size.width &&
      position.x + width > node.position.x &&
      position.y < node.position.y + node.size.height &&
      position.y + height > node.position.y,
  );
}

function nextTransferPosition(
  document: CreativeProjectDocument,
  origin: { x: number; y: number },
): { x: number; y: number } {
  const rowStride =
    CREATIVE_CANVAS_PRODUCT_NODE_SIZES.image.height + CAPTURE_ROW_GAP;
  const lowestOccupiedEdge = document.nodes.reduce(
    (lowest, node) =>
      Math.max(lowest, node.position.y + node.size.height),
    origin.y,
  );
  // At this row the candidate begins at or below every existing node, so a
  // finite canonical document always has a free slot even when one very large
  // node covers many grid cells.
  const firstGuaranteedFreeRow = Math.max(
    0,
    Math.ceil((lowestOccupiedEdge - origin.y) / rowStride),
  );
  for (let row = 0; row <= firstGuaranteedFreeRow; row += 1) {
    for (let column = 0; column < 2; column += 1) {
      const position = {
        x:
          origin.x +
          column *
            (CREATIVE_CANVAS_PRODUCT_NODE_SIZES.image.width +
              CAPTURE_COLUMN_GAP),
        y: origin.y + row * rowStride,
      };
      if (!intersectsExistingNode(document, position)) return position;
    }
  }
  throw new Error("无法为导演截图找到安全的画布位置。");
}

/**
 * Build canonical image nodes beside the Director node. One capture asset maps
 * to at most one canvas image, making response-loss reconciliation idempotent.
 */
export function planDirectorCapturesForCanvas(
  baseline: DirectorProjectBaseline,
  captures: readonly DirectorCaptureAssetTransfer[],
): DirectorCanvasTransferPlan {
  validateTransfers(baseline, captures);
  const document = structuredClone(baseline.document);
  const existingAssetIds: string[] = [];
  const insertedNodes: Extract<CreativeCanvasNode, { type: "image" }>[] = [];
  const origin = transferOrigin(baseline);
  const uniqueAssets = new Set<string>();

  for (const transfer of captures) {
    if (uniqueAssets.has(transfer.asset.id)) continue;
    uniqueAssets.add(transfer.asset.id);
    if (directorCaptureAssetsPresent(document, [transfer.asset.id])) {
      existingAssetIds.push(transfer.asset.id);
      continue;
    }
    const position = nextTransferPosition(document, origin);
    const node = creativeNodeFromAsset(
      transfer.asset,
      { document, viewport: { x: 0, y: 0, zoom: 1 } },
      { width: 1, height: 1 },
      { position },
    );
    if (node.type !== "image") {
      throw new DirectorCanvasTransferError(
        "capture-not-image",
        `截图 ${transfer.captureId} 未生成图片节点。`,
      );
    }
    document.nodes.push(node);
    insertedNodes.push(node);
  }

  return { document, insertedNodes, existingAssetIds };
}

export async function transferDirectorCapturesToCanvas(
  input: TransferDirectorCapturesInput,
): Promise<TransferDirectorCapturesResult> {
  const plan = planDirectorCapturesForCanvas(input.baseline, input.captures);
  if (plan.insertedNodes.length === 0) {
    return { ...plan, baseline: input.baseline };
  }
  const project = await input.repository.save(
    input.baseline.project.projectId,
    input.baseline.project.revision,
    plan.document,
  );
  return {
    ...plan,
    baseline: {
      ...input.baseline,
      project,
      document: plan.document,
    },
  };
}

/**
 * Reconcile an uncertain CAS response by loading the authoritative project.
 * A committed image node is accepted as success; a genuine stale revision is
 * surfaced as a conflict without duplicating the capture on retry.
 */
export async function transferDirectorCapturesWithReconciliation(
  input: ReconciledDirectorCaptureTransferInput,
): Promise<DirectorCanvasTransferOutcome> {
  try {
    const result = await transferDirectorCapturesToCanvas(input);
    return {
      status: result.insertedNodes.length > 0 ? "inserted" : "already-present",
      result,
      error: null,
    };
  } catch (cause) {
    if (!isCreativeProjectRepositoryError(cause)) throw cause;
    const error = cause instanceof Error ? cause : new Error(String(cause));
    let baseline: DirectorProjectBaseline;
    try {
      baseline = await input.reloadBaseline();
    } catch {
      throw error;
    }
    const assetIds = [
      ...new Set(input.captures.map((capture) => capture.asset.id)),
    ];
    const result: TransferDirectorCapturesResult = {
      baseline,
      document: baseline.document,
      insertedNodes: [],
      existingAssetIds: assetIds.filter((assetId) =>
        directorCaptureAssetsPresent(baseline.document, [assetId]),
      ),
    };
    if (directorCaptureAssetsPresent(baseline.document, assetIds)) {
      return {
        status: "confirmed-after-response-loss",
        result,
        error: null,
      };
    }
    return {
      status: cause.kind === "revision-conflict" ? "conflict" : "failed",
      result,
      error,
    };
  }
}
