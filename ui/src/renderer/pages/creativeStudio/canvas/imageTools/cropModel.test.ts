/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import {
  CREATIVE_IMAGE_DEFAULT_CROP,
  creativeImageCropToPixels,
  cropForCreativeImageAspect,
  moveCreativeImageCrop,
  normalizeCreativeImageCrop,
  resizeCreativeImageCrop,
} from "./cropModel";

const IMAGE = { width: 1_920, height: 1_080 };

describe("creative image crop model", () => {
  test("normalizes invalid and out-of-bounds crop rectangles", () => {
    expect(
      normalizeCreativeImageCrop({ x: -2, y: 3, width: 0, height: 4 }),
    ).toEqual({ x: 0, y: 0, width: 0.04, height: 1 });
  });

  test("centers exact pixel aspect presets inside the source image", () => {
    const square = cropForCreativeImageAspect(IMAGE, "1:1");
    const widescreen = cropForCreativeImageAspect(IMAGE, "16:9");
    expect(square.width).toBeCloseTo(
      square.height * (IMAGE.height / IMAGE.width),
    );
    expect(square.x).toBeCloseTo((1 - square.width) / 2);
    expect(widescreen).toEqual(CREATIVE_IMAGE_DEFAULT_CROP);
  });

  test("moves without allowing any edge outside the source image", () => {
    expect(
      moveCreativeImageCrop(CREATIVE_IMAGE_DEFAULT_CROP, { x: 10, y: -10 }),
    ).toEqual({ ...CREATIVE_IMAGE_DEFAULT_CROP, x: 0.24, y: 0 });
  });

  test("resizes freely from edges and corners with a non-zero minimum", () => {
    expect(
      resizeCreativeImageCrop(
        CREATIVE_IMAGE_DEFAULT_CROP,
        "north-west",
        { x: 0.1, y: 0.2 },
        IMAGE,
        "free",
      ),
    ).toEqual({ x: 0.22, y: 0.32, width: 0.66, height: 0.56 });
    const minimum = resizeCreativeImageCrop(
      CREATIVE_IMAGE_DEFAULT_CROP,
      "west",
      { x: 10, y: 0 },
      IMAGE,
      "free",
    );
    expect(minimum.width).toBeCloseTo(0.04);
  });

  test("keeps locked aspect ratios while resizing any handle", () => {
    for (const handle of ["east", "south", "south-east"] as const) {
      const resized = resizeCreativeImageCrop(
        cropForCreativeImageAspect(IMAGE, "1:1"),
        handle,
        { x: -0.1, y: -0.08 },
        IMAGE,
        "1:1",
      );
      expect(
        (resized.width * IMAGE.width) / (resized.height * IMAGE.height),
      ).toBeCloseTo(1);
      expect(resized.x).toBeGreaterThanOrEqual(0);
      expect(resized.y).toBeGreaterThanOrEqual(0);
      expect(resized.x + resized.width).toBeLessThanOrEqual(1);
      expect(resized.y + resized.height).toBeLessThanOrEqual(1);
    }
  });

  test("projects a normalized crop to safe integral decoded-image pixels", () => {
    expect(
      creativeImageCropToPixels(
        { x: 0.125, y: 0.25, width: 0.5, height: 0.5 },
        IMAGE,
      ),
    ).toEqual({ x: 240, y: 270, width: 960, height: 540 });
  });
});
