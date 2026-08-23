/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeCanvasNode,
  CreativePoint,
  CreativeSize,
} from "../../domain";
import { translateCreativeImageTool } from "./imageToolI18n";

export const CREATIVE_IMAGE_SPLIT_NODE_GAP = 16;
export const CREATIVE_IMAGE_SPLIT_ORIGIN_GAP = 96;
const SPLIT_GRID_ROW_GAP = 40;

export interface CreativeImageSplitCanvasLayout {
  origin: CreativePoint;
  cellSize: CreativeSize;
  gap: number;
  rows: number;
  columns: number;
}

const overlapsRect = (
  node: CreativeCanvasNode,
  origin: CreativePoint,
  size: CreativeSize,
): boolean =>
  origin.x < node.position.x + node.size.width &&
  origin.x + size.width > node.position.x &&
  origin.y < node.position.y + node.size.height &&
  origin.y + size.height > node.position.y;

/** Find a collision-free source-shaped grid origin to the source node's right. */
export function createCreativeImageSplitCanvasLayout(
  document: { readonly nodes: readonly CreativeCanvasNode[] },
  source: Extract<CreativeCanvasNode, { type: "image" }>,
  rows: number,
  columns: number,
  gap = CREATIVE_IMAGE_SPLIT_NODE_GAP,
): CreativeImageSplitCanvasLayout {
  if (
    !Number.isInteger(rows) ||
    rows < 1 ||
    !Number.isInteger(columns) ||
    columns < 1
  ) {
    throw new Error(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.splitGridPositive",
      ),
    );
  }
  const cellSize = {
    width: Math.max(1, source.size.width / columns),
    height: Math.max(1, source.size.height / rows),
  };
  const gridSize = {
    width: cellSize.width * columns + gap * (columns - 1),
    height: cellSize.height * rows + gap * (rows - 1),
  };
  const initial = {
    x: source.position.x + source.size.width + CREATIVE_IMAGE_SPLIT_ORIGIN_GAP,
    y: source.position.y,
  };
  const stride = gridSize.height + SPLIT_GRID_ROW_GAP;
  const lowestOccupiedEdge = document.nodes.reduce(
    (lowest, node) => Math.max(lowest, node.position.y + node.size.height),
    initial.y,
  );
  const guaranteedFreeRow = Math.max(
    0,
    Math.ceil((lowestOccupiedEdge - initial.y) / stride),
  );
  for (let row = 0; row <= guaranteedFreeRow; row += 1) {
    const origin = { x: initial.x, y: initial.y + row * stride };
    if (!document.nodes.some((node) => overlapsRect(node, origin, gridSize))) {
      return { origin, cellSize, gap, rows, columns };
    }
  }
  throw new Error(
    translateCreativeImageTool(
      "creativeStudio.canvas.imageTools.errors.splitPlacementFailed",
    ),
  );
}

export function creativeImageSplitNodePosition(
  layout: CreativeImageSplitCanvasLayout,
  row: number,
  column: number,
): CreativePoint {
  if (
    !Number.isInteger(row) ||
    row < 0 ||
    row >= layout.rows ||
    !Number.isInteger(column) ||
    column < 0 ||
    column >= layout.columns
  ) {
    throw new Error(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.splitPositionOutOfRange",
      ),
    );
  }
  return {
    x: layout.origin.x + column * (layout.cellSize.width + layout.gap),
    y: layout.origin.y + row * (layout.cellSize.height + layout.gap),
  };
}
