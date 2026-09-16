/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import type { CreativeSize } from '../../domain';

/** Preserve the node's short edge while fitting the complete media aspect ratio. */
export function canvasMediaNodeSize(
  media: { width: number | null; height: number | null },
  base: CreativeSize
): CreativeSize {
  const { width, height } = media;
  if (width === null || height === null || !Number.isFinite(width) ||
      !Number.isFinite(height) || width <= 0 || height <= 0) return { ...base };
  const shortEdge = Math.min(base.width, base.height);
  const scale = shortEdge / Math.min(width, height);
  const result = { width: width * scale, height: height * scale };
  return Number.isFinite(result.width) && Number.isFinite(result.height)
    ? result : { ...base };
}
