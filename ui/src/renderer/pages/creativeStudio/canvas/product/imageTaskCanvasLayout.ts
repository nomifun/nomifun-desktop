/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeCanvasNode, CreativeSize } from '../../domain';

const overlaps = (
  node: CreativeCanvasNode,
  position: { x: number; y: number },
  size: CreativeSize
): boolean =>
  position.x < node.position.x + node.size.width &&
  position.x + size.width > node.position.x &&
  position.y < node.position.y + node.size.height &&
  position.y + size.height > node.position.y;

/** Find the first collision-free slot to the right of a source node. */
export function nextCanvasImageTaskPosition(
  nodes: readonly CreativeCanvasNode[],
  source: CreativeCanvasNode,
  size: CreativeSize
): { x: number; y: number } {
  const origin = {
    x: source.position.x + source.size.width + 80,
    y: source.position.y,
  };
  const stride = size.height + 40;
  const lowest = nodes.reduce(
    (edge, node) => Math.max(edge, node.position.y + node.size.height),
    origin.y
  );
  const rows = Math.max(0, Math.ceil((lowest - origin.y) / stride));
  for (let row = 0; row <= rows; row += 1) {
    const position = { x: origin.x, y: origin.y + row * stride };
    if (!nodes.some((node) => overlaps(node, position, size))) return position;
  }
  throw new Error('无法为图片任务节点找到安全的画布位置。');
}
