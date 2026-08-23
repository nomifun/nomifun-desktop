/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import type { CreativeAsset } from "../../assets";
import type { CreativeModelCatalogSnapshot } from "../../models";
import { withCanvasTestI18n } from "../components/canvasI18nTestUtils";
import CreativeCanvasImageToolbar from "./CreativeCanvasImageToolbar";
import { CreativeImageCropDialogContent } from "./CreativeImageCropDialog";
import { CreativeImageMaskEditDialogContent } from "./CreativeImageMaskEditDialog";
import { CreativeImageSplitDialogContent } from "./CreativeImageSplitDialog";

const ASSET: CreativeAsset = {
  id: "018f7a3c-1234-7abc-8abc-1234567890ab",
  kind: "image",
  title: "导演截图",
  collection: null,
  tags: [],
  mimeType: "image/png",
  width: 1_920,
  height: 1_080,
  bytes: 1,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: "/api/creative-studio/files/asset",
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
};

const EMPTY_CATALOG: CreativeModelCatalogSnapshot = {
  status: "ready",
  providers: [],
  error: null,
};

const imageToolsCss = readFileSync(
  new URL("./CreativeImageTools.module.css", import.meta.url),
  "utf8",
);
const maskDialogSource = readFileSync(
  new URL("./CreativeImageMaskEditDialog.tsx", import.meta.url),
  "utf8",
);

const renderCanvas = (content: React.ReactNode) =>
  renderToStaticMarkup(withCanvasTestI18n(content));

describe("creative image tool surfaces", () => {
  test("shows only real implemented node actions when selected", () => {
    const html = renderCanvas(
      <CreativeCanvasImageToolbar
        nodeId="image-node"
        visible
        hasImageContent
        onInfo={() => undefined}
        onDelete={() => undefined}
        onUpload={() => undefined}
        onCrop={() => undefined}
        onDownload={() => undefined}
        onMaskEdit={() => undefined}
        onSplit={() => undefined}
      >
        <article>image</article>
      </CreativeCanvasImageToolbar>,
    );
    expect(
      html.includes(
        'aria-label="creativeStudio.canvas.imageTools.toolbar.label"',
      ),
    ).toBe(true);
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.toolbar.infoLabel",
      ),
    ).toBe(true);
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.toolbar.deleteLabel",
      ),
    ).toBe(true);
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.toolbar.cropLabel",
      ),
    ).toBe(true);
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.toolbar.downloadLabel",
      ),
    ).toBe(true);
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.toolbar.splitLabel",
      ),
    ).toBe(true);
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.toolbar.maskEditLabel",
      ),
    ).toBe(true);
    expect(
      html.includes("creativeStudio.canvas.imageTools.toolbar.maskEdit"),
    ).toBe(true);
    expect(html.includes("AI 超分")).toBe(false);
  });

  test("shows the source information, delete, and upload actions for an empty image", () => {
    const html = renderCanvas(
      <CreativeCanvasImageToolbar
        nodeId="empty-image-node"
        visible
        hasImageContent={false}
        onInfo={() => undefined}
        onDelete={() => undefined}
        onUpload={() => undefined}
        onCrop={() => undefined}
        onDownload={() => undefined}
        onMaskEdit={() => undefined}
        onSplit={() => undefined}
      >
        <article>empty image</article>
      </CreativeCanvasImageToolbar>,
    );
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.toolbar.infoLabel",
      ),
    ).toBe(true);
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.toolbar.deleteLabel",
      ),
    ).toBe(true);
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.toolbar.uploadLabel",
      ),
    ).toBe(true);
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.toolbar.downloadLabel",
      ),
    ).toBe(false);
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.toolbar.cropLabel",
      ),
    ).toBe(false);
  });

  test("keeps the node toolbar focused and viewport-safe", () => {
    expect(imageToolsCss.includes("background: #242424")).toBe(true);
    expect(imageToolsCss.includes("color: #f3f3f3")).toBe(true);
    expect(imageToolsCss.includes(".nodeToolbar[data-overlay='true']")).toBe(true);
    expect(imageToolsCss.includes("data-canvas-image-composer-anchor")).toBe(true);
  });

  test("renders the source mask editor geometry, controls, and exact task picker", () => {
    const html = renderCanvas(
      <CreativeImageMaskEditDialogContent
        visible
        asset={ASSET}
        catalog={EMPTY_CATALOG}
        model={null}
        onModelChange={() => undefined}
        onClose={() => undefined}
        onConfirm={() => undefined}
      />,
    );
    expect(html.includes("data-creative-image-mask-edit-dialog")).toBe(true);
    expect(
      html.includes("creativeStudio.canvas.imageTools.mask.heading"),
    ).toBe(true);
    expect(html.includes("1920 × 1080px")).toBe(true);
    expect(
      html.includes("creativeStudio.canvas.imageTools.mask.paint"),
    ).toBe(true);
    expect(
      html.includes("creativeStudio.canvas.imageTools.mask.erase"),
    ).toBe(true);
    expect(html.includes("100px")).toBe(true);
    expect(
      html.includes("creativeStudio.canvas.imageTools.mask.promptLabel"),
    ).toBe(true);
    expect(
      html.includes("creativeStudio.canvas.imageTools.mask.submit"),
    ).toBe(true);
    expect(maskDialogSource.includes('task: "image_edit"')).toBe(true);
  });

  test("locks the mask draft while preserving explicit safe retry and abandon actions", () => {
    const html = renderCanvas(
      <CreativeImageMaskEditDialogContent
        visible
        retryLocked
        asset={ASSET}
        catalog={EMPTY_CATALOG}
        model={null}
        onModelChange={() => undefined}
        onAbandon={() => undefined}
        onClose={() => undefined}
        onConfirm={() => undefined}
      />,
    );
    expect(
      html.includes("creativeStudio.canvas.imageTools.mask.safeRetry"),
    ).toBe(true);
    expect(
      html.includes("creativeStudio.canvas.imageTools.mask.abandon"),
    ).toBe(true);
    expect((html.match(/disabled=""/g)?.length ?? 0) >= 5).toBe(true);
  });

  test("renders the source crop controls, exact dimensions, and all handles", () => {
    const html = renderCanvas(
      <CreativeImageCropDialogContent
        visible
        asset={ASSET}
        onClose={() => undefined}
        onConfirm={() => undefined}
      />,
    );
    expect(html.includes("data-creative-image-crop-dialog")).toBe(true);
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.crop.metrics.size 1459 × 821",
      ),
    ).toBe(true);
    expect(
      html.includes(
        "creativeStudio.canvas.imageTools.crop.metrics.original 1920 × 1080",
      ),
    ).toBe(true);
    expect(
      html.includes("creativeStudio.canvas.imageTools.crop.confirm"),
    ).toBe(true);
    expect(
      html.split("creativeStudio.canvas.imageTools.crop.resizeBox").length -
        1,
    ).toBe(8);
    expect(
      html.includes("creativeStudio.canvas.imageTools.crop.aspectFree"),
    ).toBe(true);
    expect(html.includes("16:9")).toBe(true);
  });

  test("renders a real draggable 2 by 2 split with source dimensions", () => {
    const html = renderCanvas(
      <CreativeImageSplitDialogContent
        visible
        asset={ASSET}
        onClose={() => undefined}
        onConfirm={() => undefined}
      />,
    );
    expect(html.includes("data-creative-image-split-dialog")).toBe(true);
    expect(
      html.includes("creativeStudio.canvas.imageTools.split.summary 4"),
    ).toBe(true);
    expect(html.includes("1920 × 1080")).toBe(true);
    expect(html.includes("960 × 540")).toBe(true);
    expect(html.includes('data-split-axis="horizontal"')).toBe(true);
    expect(html.includes('data-split-axis="vertical"')).toBe(true);
    expect(
      html.includes("creativeStudio.canvas.imageTools.split.deleteLine"),
    ).toBe(true);
    expect(
      html.includes("creativeStudio.canvas.imageTools.split.generate"),
    ).toBe(true);
  });
});
