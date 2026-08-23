/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import {
  CREATIVE_IMAGE_MASK_BRUSH_DEFAULT,
  creativeImageMaskEditPrompt,
  creativeImageMaskHasPaint,
  creativeImageMaskPoint,
  normalizeCreativeImageMaskBrush,
  validateCreativeImageMaskEdit,
} from "./maskModel";

describe("creative image mask model", () => {
  test("normalizes the source brush contract to 8..160 in two-pixel steps", () => {
    expect(normalizeCreativeImageMaskBrush(Number.NaN)).toBe(
      CREATIVE_IMAGE_MASK_BRUSH_DEFAULT,
    );
    expect(normalizeCreativeImageMaskBrush(3)).toBe(8);
    expect(normalizeCreativeImageMaskBrush(101)).toBe(102);
    expect(normalizeCreativeImageMaskBrush(900)).toBe(160);
  });

  test("maps rendered pointer coordinates into clamped natural pixels", () => {
    expect(
      creativeImageMaskPoint(
        { left: 100, top: 50, width: 400, height: 200 },
        { width: 2_000, height: 1_000 },
        { x: 300, y: 150 },
      ),
    ).toEqual({ x: 1_000, y: 500 });
    expect(
      creativeImageMaskPoint(
        { left: 100, top: 50, width: 400, height: 200 },
        { width: 2_000, height: 1_000 },
        { x: -20, y: 400 },
      ),
    ).toEqual({ x: 0, y: 1_000 });
  });

  test("detects alpha paint and preserves source validation precedence", () => {
    expect(creativeImageMaskHasPaint(new Uint8ClampedArray(12))).toBe(false);
    expect(
      creativeImageMaskHasPaint(
        new Uint8ClampedArray([0, 0, 0, 0, 0, 0, 0, 12]),
      ),
    ).toBe(true);
    expect(
      validateCreativeImageMaskEdit({
        prompt: " ",
        hasMask: false,
        hasModel: false,
      }),
    ).toBe("promptRequired");
    expect(
      validateCreativeImageMaskEdit({
        prompt: "换成红色外套",
        hasMask: false,
        hasModel: false,
      }),
    ).toBe("maskRequired");
    expect(
      validateCreativeImageMaskEdit({
        prompt: "换成红色外套",
        hasMask: true,
        hasModel: false,
      }),
    ).toBe("modelRequired");
  });

  test("builds the exact marked-reference instruction without blue leakage", () => {
    expect(creativeImageMaskEditPrompt("  换成红色外套  ")).toBe(
      "参考图中蓝色高亮覆盖区域是需要修改的位置，蓝色只是编辑标记，不要保留在最终图像中。只修改蓝色高亮区域，其他区域的构图、人物、文字、光影和风格保持不变。修改要求：换成红色外套",
    );
  });
});
