/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeCanvasNode, CreativeSize } from "../../domain";

const DERIVED_IMAGE_GAP = 80;
const DERIVED_IMAGE_ROW_GAP = 40;

const overlaps = (
  node: CreativeCanvasNode,
  position: { x: number; y: number },
  size: CreativeSize,
): boolean =>
  position.x < node.position.x + node.size.width &&
  position.x + size.width > node.position.x &&
  position.y < node.position.y + node.size.height &&
  position.y + size.height > node.position.y;

/** Find the first collision-free derived-image slot to the source node's right. */
export function nextDerivedImagePosition(
  document: { readonly nodes: readonly CreativeCanvasNode[] },
  source: Extract<CreativeCanvasNode, { type: "image" }>,
  size: CreativeSize,
): { x: number; y: number } {
  const origin = {
    x: source.position.x + source.size.width + DERIVED_IMAGE_GAP,
    y: source.position.y,
  };
  const rowStride = size.height + DERIVED_IMAGE_ROW_GAP;
  const lowestOccupiedEdge = document.nodes.reduce(
    (lowest, node) => Math.max(lowest, node.position.y + node.size.height),
    origin.y,
  );
  const firstGuaranteedFreeRow = Math.max(
    0,
    Math.ceil((lowestOccupiedEdge - origin.y) / rowStride),
  );
  for (let row = 0; row <= firstGuaranteedFreeRow; row += 1) {
    const position = { x: origin.x, y: origin.y + row * rowStride };
    if (!document.nodes.some((node) => overlaps(node, position, size))) {
      return position;
    }
  }
  throw new Error("无法为裁剪结果找到安全的画布位置。");
}
