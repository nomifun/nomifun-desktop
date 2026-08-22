/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  CuttingOne,
  Delete,
  Download,
  GridNine,
  Info,
  MagicWand,
  Upload,
} from "@icon-park/react";
import React, { useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import styles from "./CreativeImageTools.module.css";

export interface CreativeCanvasImageToolbarProps {
  nodeId: string;
  children: React.ReactNode;
  visible: boolean;
  hasImageContent: boolean;
  disabled?: boolean;
  onInfo(): void;
  onDelete(): void;
  onUpload(): void;
  onCrop(): void;
  onDownload(): void;
  onMaskEdit?: () => void;
  onSplit(): void;
}

const iconProps = {
  theme: "outline" as const,
  size: 15,
  fill: "currentColor",
  strokeWidth: 3,
};

/** Source-density image tools layered above a selected canonical image node. */
const CreativeCanvasImageToolbar: React.FC<CreativeCanvasImageToolbarProps> = ({
  nodeId,
  children,
  visible,
  hasImageContent,
  disabled = false,
  onInfo,
  onDelete,
  onUpload,
  onCrop,
  onDownload,
  onMaskEdit,
  onSplit,
}) => {
  const hostRef = useRef<HTMLDivElement>(null);
  const toolbarRef = useRef<HTMLDivElement>(null);
  const [overlay, setOverlay] = useState(false);
  const [overlayTop, setOverlayTop] = useState(16);

  useLayoutEffect(() => {
    if (!visible) return;
    const host = hostRef.current;
    const toolbar = toolbarRef.current;
    const surface = host?.closest<HTMLElement>('[data-canvas-surface]');
    if (!host || !toolbar || !surface) return;

    const update = (): void => {
      const hostRect = host.getBoundingClientRect();
      const toolbarRect = toolbar.getBoundingClientRect();
      const surfaceRect = surface.getBoundingClientRect();
      const naturalLeft = hostRect.left + hostRect.width / 2 - toolbarRect.width / 2;
      const naturalRight = naturalLeft + toolbarRect.width;
      const nextOverlay =
        surfaceRect.width < toolbarRect.width + 24 ||
        naturalLeft < surfaceRect.left + 12 ||
        naturalRight > surfaceRect.right - 12;
      setOverlay((current) => (current === nextOverlay ? current : nextOverlay));
      if (!nextOverlay) return;
      const composer = Array.from(
        document.querySelectorAll<HTMLElement>('[data-canvas-image-composer]')
      ).find((candidate) => candidate.dataset.nodeId === nodeId);
      const composerBottom = composer?.getBoundingClientRect().bottom;
      const preferredTop = composerBottom
        ? composerBottom + 8
        : hostRect.top - toolbarRect.height - 10;
      const nextTop = Math.max(
        12,
        Math.min(window.innerHeight - toolbarRect.height - 12, preferredTop)
      );
      setOverlayTop((current) =>
        Math.abs(current - nextTop) < 0.5 ? current : nextTop
      );
    };

    update();
    const observer =
      typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(update);
    observer?.observe(host);
    observer?.observe(toolbar);
    observer?.observe(surface);
    window.addEventListener('resize', update);
    return () => {
      observer?.disconnect();
      window.removeEventListener('resize', update);
    };
  }, [nodeId, overlay, visible]);

  const toolbar = visible ? (
    <div
      ref={toolbarRef}
      className={styles.nodeToolbar}
      role="toolbar"
      aria-label="图片工具"
      data-overlay={overlay || undefined}
      style={
        {
          '--creative-canvas-image-toolbar-overlay-top': `${overlayTop}px`,
        } as React.CSSProperties
      }
      onPointerDown={(event) => event.stopPropagation()}
      onDoubleClick={(event) => event.stopPropagation()}
    >
        <button
          type="button"
          aria-label="查看节点信息"
          disabled={disabled}
          onClick={(event) => {
            event.stopPropagation();
            onInfo();
          }}
        >
          <Info {...iconProps} />
          <span className={styles.toolLabel}>信息</span>
        </button>
        <button
          type="button"
          aria-label="移除节点"
          data-danger
          disabled={disabled}
          onClick={(event) => {
            event.stopPropagation();
            onDelete();
          }}
        >
          <Delete {...iconProps} />
          <span className={styles.toolLabel}>删除</span>
        </button>
        {!hasImageContent ? (
          <button
            type="button"
            aria-label="上传图片"
            disabled={disabled}
            onClick={(event) => {
              event.stopPropagation();
              onUpload();
            }}
          >
            <Upload {...iconProps} />
            <span className={styles.toolLabel}>上传图片</span>
          </button>
        ) : (
          <>
            <button
              type="button"
              aria-label="下载图片"
              disabled={disabled}
              onClick={(event) => {
                event.stopPropagation();
                onDownload();
              }}
            >
              <Download {...iconProps} />
              <span className={styles.toolLabel}>下载</span>
            </button>
            <button
              type="button"
              aria-label="裁剪并生成新节点"
              disabled={disabled}
              onClick={(event) => {
                event.stopPropagation();
                onCrop();
              }}
            >
              <CuttingOne {...iconProps} />
              <span className={styles.toolLabel}>裁剪</span>
            </button>
            {onMaskEdit ? (
              <button
                type="button"
                aria-label="对图片进行局部修改"
                disabled={disabled}
                onClick={(event) => {
                  event.stopPropagation();
                  onMaskEdit();
                }}
              >
                <MagicWand {...iconProps} />
                <span className={styles.toolLabel}>局部编辑</span>
              </button>
            ) : null}
            <button
              type="button"
              aria-label="切分并生成图片子节点"
              disabled={disabled}
              onClick={(event) => {
                event.stopPropagation();
                onSplit();
              }}
            >
              <GridNine {...iconProps} />
              <span className={styles.toolLabel}>切图</span>
            </button>
          </>
        )}
    </div>
  ) : null;

  return (
    <div ref={hostRef} className={styles.nodeHost} data-canvas-image-tools-host>
      {children}
      {overlay && toolbar && typeof document !== 'undefined'
        ? createPortal(toolbar, document.body)
        : toolbar}
    </div>
  );
};

export default CreativeCanvasImageToolbar;
