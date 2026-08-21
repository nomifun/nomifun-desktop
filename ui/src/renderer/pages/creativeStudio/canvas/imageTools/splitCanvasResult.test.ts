/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import { testDocument, testNode } from "../core/testFixtures";
import {
  createCreativeImageSplitCanvasLayout,
  creativeImageSplitNodePosition,
} from "./splitCanvasResult";

describe("creative image split canvas layout", () => {
  test("matches the source right-side grid geometry", () => {
    const source = testNode("image", 1, {
      x: 20,
      y: 30,
      width: 320,
      height: 240,
    });
    const layout = createCreativeImageSplitCanvasLayout(
      testDocument([source]),
      source,
      2,
      2,
    );
    expect(layout).toEqual({
      origin: { x: 436, y: 30 },
      cellSize: { width: 160, height: 120 },
      gap: 16,
      rows: 2,
      columns: 2,
    });
    expect(creativeImageSplitNodePosition(layout, 1, 1)).toEqual({
      x: 612,
      y: 166,
    });
  });

  test("moves the entire grid below arbitrary collisions without overlap", () => {
    const source = testNode("image", 1, {
      x: 0,
      y: 0,
      width: 320,
      height: 240,
    });
    const blocker = testNode("image", 2, {
      x: 400,
      y: -100,
      width: 1_000,
      height: 1_000,
    });
    const layout = createCreativeImageSplitCanvasLayout(
      testDocument([source, blocker]),
      source,
      2,
      2,
    );
    expect(layout.origin.x).toBe(416);
    expect(layout.origin.y).toBeGreaterThanOrEqual(900);
  });
});
