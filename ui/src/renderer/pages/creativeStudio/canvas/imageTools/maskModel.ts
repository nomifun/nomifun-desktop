/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { translateCreativeImageTool } from "./imageToolI18n";

export const CREATIVE_IMAGE_MASK_BRUSH_MIN = 8;
export const CREATIVE_IMAGE_MASK_BRUSH_MAX = 160;
export const CREATIVE_IMAGE_MASK_BRUSH_STEP = 2;
export const CREATIVE_IMAGE_MASK_BRUSH_DEFAULT = 100;
export const CREATIVE_IMAGE_MASK_FILL = "rgba(37, 99, 235, 0.38)";

export type CreativeImageMaskDrawMode = "paint" | "erase";

export interface CreativeImageMaskCanvasSize {
  width: number;
  height: number;
}

export interface CreativeImageMaskClientBounds extends CreativeImageMaskCanvasSize {
  left: number;
  top: number;
}

export interface CreativeImageMaskPoint {
  x: number;
  y: number;
}

export type CreativeImageMaskValidationError =
  | "promptRequired"
  | "maskRequired"
  | "modelRequired";

const clamp = (value: number, minimum: number, maximum: number): number =>
  Math.min(maximum, Math.max(minimum, value));

export function normalizeCreativeImageMaskBrush(value: number): number {
  if (!Number.isFinite(value)) return CREATIVE_IMAGE_MASK_BRUSH_DEFAULT;
  const stepped =
    Math.round(value / CREATIVE_IMAGE_MASK_BRUSH_STEP) *
    CREATIVE_IMAGE_MASK_BRUSH_STEP;
  return clamp(
    stepped,
    CREATIVE_IMAGE_MASK_BRUSH_MIN,
    CREATIVE_IMAGE_MASK_BRUSH_MAX,
  );
}

/** Map a pointer in rendered CSS pixels into the natural-resolution mask. */
export function creativeImageMaskPoint(
  bounds: CreativeImageMaskClientBounds,
  canvas: CreativeImageMaskCanvasSize,
  client: CreativeImageMaskPoint,
): CreativeImageMaskPoint {
  const width = Math.max(1, bounds.width);
  const height = Math.max(1, bounds.height);
  return {
    x: clamp(((client.x - bounds.left) / width) * canvas.width, 0, canvas.width),
    y: clamp(((client.y - bounds.top) / height) * canvas.height, 0, canvas.height),
  };
}

export function creativeImageMaskHasPaint(
  rgba: ArrayLike<number>,
): boolean {
  for (let index = 3; index < rgba.length; index += 4) {
    if ((rgba[index] ?? 0) > 0) return true;
  }
  return false;
}

export function validateCreativeImageMaskEdit(input: {
  prompt: string;
  hasMask: boolean;
  hasModel: boolean;
}): CreativeImageMaskValidationError | null {
  if (!input.prompt.trim()) return "promptRequired";
  if (!input.hasMask) return "maskRequired";
  if (!input.hasModel) return "modelRequired";
  return null;
}

export function creativeImageMaskEditPrompt(prompt: string): string {
  const requirement = prompt.trim();
  if (!requirement) {
    throw new Error(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.mask.validation.promptRequired",
      ),
    );
  }
  return `参考图中蓝色高亮覆盖区域是需要修改的位置，蓝色只是编辑标记，不要保留在最终图像中。只修改蓝色高亮区域，其他区域的构图、人物、文字、光影和风格保持不变。修改要求：${requirement}`;
}
