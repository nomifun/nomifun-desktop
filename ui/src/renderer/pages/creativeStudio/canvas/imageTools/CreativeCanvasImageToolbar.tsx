/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CuttingOne, Download } from "@icon-park/react";
import React from "react";

import styles from "./CreativeImageTools.module.css";

export interface CreativeCanvasImageToolbarProps {
  children: React.ReactNode;
  visible: boolean;
  disabled?: boolean;
  onCrop(): void;
  onDownload(): void;
}

const iconProps = {
  theme: "outline" as const,
  size: 15,
  fill: "currentColor",
  strokeWidth: 3,
};

/** Source-density image tools layered above a selected canonical image node. */
const CreativeCanvasImageToolbar: React.FC<CreativeCanvasImageToolbarProps> = ({
  children,
  visible,
  disabled = false,
  onCrop,
  onDownload,
}) => (
  <div className={styles.nodeHost} data-canvas-image-tools-host>
    {children}
    {visible ? (
      <div
        className={styles.nodeToolbar}
        role="toolbar"
        aria-label="图片工具"
        onPointerDown={(event) => event.stopPropagation()}
        onDoubleClick={(event) => event.stopPropagation()}
      >
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
          <span>下载</span>
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
          <span>裁剪</span>
        </button>
      </div>
    ) : null}
  </div>
);

export default CreativeCanvasImageToolbar;
