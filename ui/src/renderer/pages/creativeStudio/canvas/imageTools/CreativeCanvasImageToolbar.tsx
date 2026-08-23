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
import { useTranslation } from "react-i18next";

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
  const { t } = useTranslation();
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
      aria-label={t("creativeStudio.canvas.imageTools.toolbar.label")}
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
          aria-label={t(
            "creativeStudio.canvas.imageTools.toolbar.infoLabel",
          )}
          disabled={disabled}
          onClick={(event) => {
            event.stopPropagation();
            onInfo();
          }}
        >
          <Info {...iconProps} />
          <span className={styles.toolLabel}>
            {t("creativeStudio.canvas.imageTools.toolbar.info")}
          </span>
        </button>
        <button
          type="button"
          aria-label={t(
            "creativeStudio.canvas.imageTools.toolbar.deleteLabel",
          )}
          data-danger
          disabled={disabled}
          onClick={(event) => {
            event.stopPropagation();
            onDelete();
          }}
        >
          <Delete {...iconProps} />
          <span className={styles.toolLabel}>
            {t("creativeStudio.canvas.imageTools.toolbar.delete")}
          </span>
        </button>
        {!hasImageContent ? (
          <button
            type="button"
            aria-label={t(
              "creativeStudio.canvas.imageTools.toolbar.uploadLabel",
            )}
            disabled={disabled}
            onClick={(event) => {
              event.stopPropagation();
              onUpload();
            }}
          >
            <Upload {...iconProps} />
            <span className={styles.toolLabel}>
              {t("creativeStudio.canvas.imageTools.toolbar.upload")}
            </span>
          </button>
        ) : (
          <>
            <button
              type="button"
              aria-label={t(
                "creativeStudio.canvas.imageTools.toolbar.downloadLabel",
              )}
              disabled={disabled}
              onClick={(event) => {
                event.stopPropagation();
                onDownload();
              }}
            >
              <Download {...iconProps} />
              <span className={styles.toolLabel}>
                {t("creativeStudio.canvas.imageTools.toolbar.download")}
              </span>
            </button>
            <button
              type="button"
              aria-label={t(
                "creativeStudio.canvas.imageTools.toolbar.cropLabel",
              )}
              disabled={disabled}
              onClick={(event) => {
                event.stopPropagation();
                onCrop();
              }}
            >
              <CuttingOne {...iconProps} />
              <span className={styles.toolLabel}>
                {t("creativeStudio.canvas.imageTools.toolbar.crop")}
              </span>
            </button>
            {onMaskEdit ? (
              <button
                type="button"
                aria-label={t(
                  "creativeStudio.canvas.imageTools.toolbar.maskEditLabel",
                )}
                disabled={disabled}
                onClick={(event) => {
                  event.stopPropagation();
                  onMaskEdit();
                }}
              >
                <MagicWand {...iconProps} />
                <span className={styles.toolLabel}>
                  {t("creativeStudio.canvas.imageTools.toolbar.maskEdit")}
                </span>
              </button>
            ) : null}
            <button
              type="button"
              aria-label={t(
                "creativeStudio.canvas.imageTools.toolbar.splitLabel",
              )}
              disabled={disabled}
              onClick={(event) => {
                event.stopPropagation();
                onSplit();
              }}
            >
              <GridNine {...iconProps} />
              <span className={styles.toolLabel}>
                {t("creativeStudio.canvas.imageTools.toolbar.split")}
              </span>
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
