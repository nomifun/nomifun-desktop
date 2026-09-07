/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  isCreativeCanvasUserNode,
  type CreativeCanvasNode,
  type CreativeSize,
} from '../../domain';
import { creativeStudioProductText } from './i18n';

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
  const visibleNodes = nodes.filter(isCreativeCanvasUserNode);
  const origin = {
    x: source.position.x + source.size.width + 80,
    y: source.position.y,
  };
  const stride = size.height + 40;
  const lowest = visibleNodes.reduce(
    (edge, node) => Math.max(edge, node.position.y + node.size.height),
    origin.y
  );
  const rows = Math.max(0, Math.ceil((lowest - origin.y) / stride));
  for (let row = 0; row <= rows; row += 1) {
    const position = { x: origin.x, y: origin.y + row * stride };
    if (!visibleNodes.some((node) => overlaps(node, position, size))) {
      return position;
    }
  }
  throw new Error(
    creativeStudioProductText(
      'creativeStudio.canvas.tasks.noSafePosition',
      '无法为图片任务节点找到安全的画布位置。'
    )
  );
}

/** Place generated output beside its user-facing input, never beside task metadata. */
export function canvasTaskResultPosition(
  nodes: readonly CreativeCanvasNode[],
  config: Extract<CreativeCanvasNode, { type: 'config' }>,
  size: CreativeSize
): { x: number; y: number } {
  const sourceNodeId = config.data.operation?.sourceNodeId;
  const source = sourceNodeId
    ? nodes.find(
        (node) => node.id === sourceNodeId && isCreativeCanvasUserNode(node)
      )
    : null;
  return nextCanvasImageTaskPosition(nodes, source ?? config, size);
}
