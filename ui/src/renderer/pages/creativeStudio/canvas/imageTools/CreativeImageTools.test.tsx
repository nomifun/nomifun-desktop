/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import type { CreativeAsset } from "../../assets";
import type { CreativeModelCatalogSnapshot } from "../../models";
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

describe("creative image tool surfaces", () => {
  test("shows only real implemented node actions when selected", () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasImageToolbar
        visible
        onCrop={() => undefined}
        onDownload={() => undefined}
        onMaskEdit={() => undefined}
        onSplit={() => undefined}
      >
        <article>image</article>
      </CreativeCanvasImageToolbar>,
    );
    expect(html.includes('aria-label="图片工具"')).toBe(true);
    expect(html.includes("裁剪并生成新节点")).toBe(true);
    expect(html.includes("下载图片")).toBe(true);
    expect(html.includes("切分并生成图片子节点")).toBe(true);
    expect(html.includes("对图片进行局部修改")).toBe(true);
    expect(html.includes("局部编辑")).toBe(true);
    expect(html.includes("AI 超分")).toBe(false);
  });

  test("renders the source mask editor geometry, controls, and exact task picker", () => {
    const html = renderToStaticMarkup(
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
    expect(html.includes("局部遮罩编辑")).toBe(true);
    expect(html.includes("1920 × 1080px")).toBe(true);
    expect(html.includes("画笔")).toBe(true);
    expect(html.includes("擦除")).toBe(true);
    expect(html.includes("100px")).toBe(true);
    expect(html.includes("修改要求")).toBe(true);
    expect(html.includes("AI 修改")).toBe(true);
    expect(html.includes("image_edit")).toBe(true);
  });

  test("locks the mask draft while preserving explicit safe retry and abandon actions", () => {
    const html = renderToStaticMarkup(
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
    expect(html.includes("安全重试")).toBe(true);
    expect(html.includes("放弃本次")).toBe(true);
    expect((html.match(/disabled=""/g)?.length ?? 0) >= 5).toBe(true);
  });

  test("renders the source crop controls, exact dimensions, and all handles", () => {
    const html = renderToStaticMarkup(
      <CreativeImageCropDialogContent
        visible
        asset={ASSET}
        onClose={() => undefined}
        onConfirm={() => undefined}
      />,
    );
    expect(html.includes("data-creative-image-crop-dialog")).toBe(true);
    expect(html.includes("裁剪尺寸 1459 × 821")).toBe(true);
    expect(html.includes("原图 1920 × 1080")).toBe(true);
    expect(html.includes("确认裁剪")).toBe(true);
    expect(html.split("调整裁剪框：").length - 1).toBe(8);
    expect(html.includes("自由比例")).toBe(true);
    expect(html.includes("16:9")).toBe(true);
  });

  test("renders a real draggable 2 by 2 split with source dimensions", () => {
    const html = renderToStaticMarkup(
      <CreativeImageSplitDialogContent
        visible
        asset={ASSET}
        onClose={() => undefined}
        onConfirm={() => undefined}
      />,
    );
    expect(html.includes("data-creative-image-split-dialog")).toBe(true);
    expect(html.includes("生成 4 个图片子节点")).toBe(true);
    expect(html.includes("1920 × 1080")).toBe(true);
    expect(html.includes("960 × 540")).toBe(true);
    expect(html.includes('data-split-axis="horizontal"')).toBe(true);
    expect(html.includes('data-split-axis="vertical"')).toBe(true);
    expect(html.includes("删除线")).toBe(true);
    expect(html.includes("生成子节点")).toBe(true);
  });
});
