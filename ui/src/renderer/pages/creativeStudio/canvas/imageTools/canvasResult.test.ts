/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import {
  createEmptyCreativeProjectDocument,
  type CreativeCanvasNode,
} from "../../domain";
import { nextDerivedImagePosition } from "./canvasResult";

const PROJECT_ID = "018f7a3c-1234-7abc-8abc-1234567890ab";
const SOURCE: Extract<CreativeCanvasNode, { type: "image" }> = {
  id: "018f7a3c-1235-7abc-8abc-1234567890ab",
  type: "image",
  position: { x: 100, y: 80 },
  size: { width: 320, height: 220 },
  groupId: null,
  zIndex: 1,
  locked: false,
  data: {
    assetId: "018f7a3c-1236-7abc-8abc-1234567890ab",
    caption: "source",
    alt: "source",
    fit: "contain",
    naturalSize: { width: 1_920, height: 1_080 },
    composer: null,
  },
};

describe("cropped image canvas placement", () => {
  test("uses the source-right slot when it is empty", () => {
    const document = createEmptyCreativeProjectDocument(PROJECT_ID);
    document.nodes.push(SOURCE);
    expect(nextDerivedImagePosition(document, SOURCE, SOURCE.size)).toEqual({
      x: 500,
      y: 80,
    });
  });

  test("walks below arbitrarily tall collisions instead of overlapping", () => {
    const document = createEmptyCreativeProjectDocument(PROJECT_ID);
    document.nodes.push(SOURCE, {
      ...SOURCE,
      id: "018f7a3c-1237-7abc-8abc-1234567890ab",
      position: { x: 480, y: 40 },
      size: { width: 500, height: 1_000 },
    });
    const position = nextDerivedImagePosition(document, SOURCE, SOURCE.size);
    expect(position.x).toBe(500);
    expect(position.y).toBeGreaterThanOrEqual(1_040);
  });
});
