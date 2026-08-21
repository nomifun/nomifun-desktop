/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Modal, Progress } from "@arco-design/web-react";
import { CheckOne, Delete, DividingLine, Refresh } from "@icon-park/react";
import React, { useEffect, useMemo, useRef, useState } from "react";

import type { CreativeAsset } from "../../assets";
import type { CreativeImageDimensions } from "./cropModel";
import {
  CREATIVE_IMAGE_DEFAULT_SPLIT,
  CREATIVE_IMAGE_SPLIT_MAX_GRID,
  addCreativeImageSplitLine,
  creativeImageSplitColumns,
  creativeImageSplitRows,
  creativeImageSplitTotal,
  moveCreativeImageSplitLine,
  removeCreativeImageSplitLine,
  resetCreativeImageSplitLines,
  setCreativeImageSplitCount,
  type CreativeImageSplitAxis,
  type CreativeImageSplitParams,
} from "./splitModel";
import styles from "./CreativeImageTools.module.css";

export interface CreativeImageSplitDialogProps {
  visible: boolean;
  asset: CreativeAsset | null;
  busy?: boolean;
  progress?: number | null;
  error?: string | null;
  onClose(): void;
  onConfirm(params: CreativeImageSplitParams): void;
}

interface ActiveSplitLine {
  axis: CreativeImageSplitAxis;
  index: number;
}

interface SplitPointerSession extends ActiveSplitLine {
  pointerId: number;
}

const finiteDimensions = (
  asset: CreativeAsset | null,
): CreativeImageDimensions | null => {
  if (
    !asset ||
    !Number.isFinite(asset.width) ||
    !Number.isFinite(asset.height) ||
    (asset.width ?? 0) <= 0 ||
    (asset.height ?? 0) <= 0
  ) {
    return null;
  }
  return { width: asset.width as number, height: asset.height as number };
};

const axisLines = (
  params: CreativeImageSplitParams,
  axis: CreativeImageSplitAxis,
): readonly number[] =>
  axis === "horizontal" ? params.horizontalLines : params.verticalLines;

const lineLabel = (
  axis: CreativeImageSplitAxis,
  index: number,
  value: number,
): string =>
  `${axis === "horizontal" ? "水平" : "垂直"}分割线 ${index + 1}，${Math.round(value * 100)}%`;

export const CreativeImageSplitDialogContent: React.FC<
  CreativeImageSplitDialogProps
> = ({
  visible,
  asset,
  busy = false,
  progress = null,
  error = null,
  onConfirm,
}) => {
  const dialogRef = useRef<HTMLDivElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const pointerRef = useRef<SplitPointerSession | null>(null);
  const [params, setParams] = useState<CreativeImageSplitParams>(
    CREATIVE_IMAGE_DEFAULT_SPLIT,
  );
  const [activeLine, setActiveLine] = useState<ActiveSplitLine | null>(null);
  const [dimensions, setDimensions] = useState<CreativeImageDimensions | null>(
    () => finiteDimensions(asset),
  );
  const [imageFailed, setImageFailed] = useState(false);

  useEffect(() => {
    if (!visible) return;
    setParams(CREATIVE_IMAGE_DEFAULT_SPLIT);
    setActiveLine(null);
    setDimensions(finiteDimensions(asset));
    setImageFailed(false);
    pointerRef.current = null;
    const focusFrame = window.requestAnimationFrame(() =>
      dialogRef.current?.focus({ preventScroll: true }),
    );
    return () => window.cancelAnimationFrame(focusFrame);
  }, [asset?.id, visible]);

  const rows = creativeImageSplitRows(params);
  const columns = creativeImageSplitColumns(params);
  const total = creativeImageSplitTotal(params);
  const averageSize = useMemo(
    () =>
      dimensions
        ? {
            width: Math.round(dimensions.width / columns),
            height: Math.round(dimensions.height / rows),
          }
        : null,
    [columns, dimensions, rows],
  );

  const updateCount = (axis: CreativeImageSplitAxis, value: string) => {
    const parsed = Number.parseInt(value, 10);
    setParams((current) =>
      setCreativeImageSplitCount(
        current,
        axis,
        Number.isFinite(parsed) ? parsed : 1,
      ),
    );
    setActiveLine(null);
  };

  const addLine = (axis: CreativeImageSplitAxis) => {
    const previous = axisLines(params, axis);
    const next = addCreativeImageSplitLine(params, axis);
    const nextLines = axisLines(next, axis);
    const addedIndex = nextLines.findIndex(
      (value) =>
        !previous.some((existing) => Math.abs(existing - value) < 1e-9),
    );
    setParams(next);
    if (addedIndex >= 0) setActiveLine({ axis, index: addedIndex });
  };

  const removeActiveLine = () => {
    if (!activeLine) return;
    setParams((current) =>
      removeCreativeImageSplitLine(current, activeLine.axis, activeLine.index),
    );
    setActiveLine(null);
  };

  const resetLines = () => {
    setParams((current) => resetCreativeImageSplitLines(current));
    setActiveLine(null);
  };

  const beginPointer = (
    line: ActiveSplitLine,
    event: React.PointerEvent<HTMLButtonElement>,
  ) => {
    if (busy || !dimensions) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    pointerRef.current = { ...line, pointerId: event.pointerId };
    setActiveLine(line);
  };

  const updateLineFromClient = (
    line: ActiveSplitLine,
    clientX: number,
    clientY: number,
  ) => {
    const bounds = stageRef.current?.getBoundingClientRect();
    if (!bounds || bounds.width <= 0 || bounds.height <= 0) return;
    const value =
      line.axis === "horizontal"
        ? (clientY - bounds.top) / bounds.height
        : (clientX - bounds.left) / bounds.width;
    setParams((current) =>
      moveCreativeImageSplitLine(current, line.axis, line.index, value),
    );
  };

  const movePointer = (event: React.PointerEvent<HTMLDivElement>) => {
    const session = pointerRef.current;
    if (!session || session.pointerId !== event.pointerId) return;
    updateLineFromClient(session, event.clientX, event.clientY);
  };

  const endPointer = (event: React.PointerEvent<HTMLDivElement>) => {
    if (pointerRef.current?.pointerId === event.pointerId) {
      pointerRef.current = null;
    }
  };

  const moveLineByKeyboard = (
    line: ActiveSplitLine,
    event: React.KeyboardEvent<HTMLButtonElement>,
  ) => {
    if (busy) return;
    const direction =
      line.axis === "horizontal"
        ? event.key === "ArrowUp"
          ? -1
          : event.key === "ArrowDown"
            ? 1
            : 0
        : event.key === "ArrowLeft"
          ? -1
          : event.key === "ArrowRight"
            ? 1
            : 0;
    if (direction === 0) return;
    event.preventDefault();
    event.stopPropagation();
    setParams((value) => {
      const current = axisLines(value, line.axis)[line.index];
      return moveCreativeImageSplitLine(
        value,
        line.axis,
        line.index,
        current + direction * (event.shiftKey ? 0.05 : 0.01),
      );
    });
    setActiveLine(line);
  };

  return (
    <div
      ref={dialogRef}
      className={styles.splitDialog}
      data-creative-image-split-dialog
      tabIndex={-1}
    >
      <p className={styles.splitSubtitle}>
        生成 {total} 个图片子节点，并按原图网格排列到画布右侧
      </p>
      <div className={styles.splitWorkspace}>
        <section className={styles.splitPreviewPane} aria-label="切图预览">
          {asset ? (
            <div className={styles.splitPreviewWell}>
              <div
                ref={stageRef}
                className={styles.splitStage}
                style={{
                  aspectRatio: dimensions
                    ? `${dimensions.width} / ${dimensions.height}`
                    : "16 / 9",
                }}
                onPointerMove={movePointer}
                onPointerUp={endPointer}
                onPointerCancel={endPointer}
              >
                {!imageFailed ? (
                  <img
                    src={asset.originalUrl}
                    alt={`${asset.title} 切图预览`}
                    draggable={false}
                    onLoad={(event) => {
                      const image = event.currentTarget;
                      if (image.naturalWidth > 0 && image.naturalHeight > 0) {
                        setDimensions({
                          width: image.naturalWidth,
                          height: image.naturalHeight,
                        });
                      }
                    }}
                    onError={() => setImageFailed(true)}
                  />
                ) : (
                  <div className={styles.cropImageError} role="alert">
                    无法载入原图，切图操作已停止。
                  </div>
                )}
                {dimensions && !imageFailed
                  ? (["horizontal", "vertical"] as const).flatMap((axis) =>
                      axisLines(params, axis).map((value, index) => {
                        const selected =
                          activeLine?.axis === axis &&
                          activeLine.index === index;
                        return (
                          <button
                            key={`${axis}-${index}`}
                            type="button"
                            className={styles.splitLine}
                            data-split-axis={axis}
                            data-selected={selected || undefined}
                            style={
                              axis === "horizontal"
                                ? { top: `${value * 100}%` }
                                : { left: `${value * 100}%` }
                            }
                            aria-label={lineLabel(axis, index, value)}
                            disabled={busy}
                            onPointerDown={(event) =>
                              beginPointer({ axis, index }, event)
                            }
                            onKeyDown={(event) =>
                              moveLineByKeyboard({ axis, index }, event)
                            }
                          />
                        );
                      }),
                    )
                  : null}
              </div>
            </div>
          ) : null}
          <div className={styles.splitOriginalSize} aria-live="polite">
            <span>原图</span>
            <strong>
              {dimensions
                ? `${dimensions.width} × ${dimensions.height} px`
                : "—"}
            </strong>
          </div>
        </section>

        <aside className={styles.splitControlPane} aria-label="切图设置">
          <section className={styles.splitControlSection}>
            <div className={styles.splitCountGrid}>
              <label>
                <span>行数</span>
                <input
                  type="number"
                  min={1}
                  max={CREATIVE_IMAGE_SPLIT_MAX_GRID}
                  value={rows}
                  disabled={busy}
                  onChange={(event) =>
                    updateCount("horizontal", event.currentTarget.value)
                  }
                />
              </label>
              <label>
                <span>列数</span>
                <input
                  type="number"
                  min={1}
                  max={CREATIVE_IMAGE_SPLIT_MAX_GRID}
                  value={columns}
                  disabled={busy}
                  onChange={(event) =>
                    updateCount("vertical", event.currentTarget.value)
                  }
                />
              </label>
            </div>
          </section>

          <section className={styles.splitControlSection}>
            <div className={styles.splitActionGrid}>
              <Button
                size="small"
                icon={<DividingLine theme="outline" size={14} />}
                disabled={busy || rows >= CREATIVE_IMAGE_SPLIT_MAX_GRID}
                aria-label="添加水平分割线"
                onClick={() => addLine("horizontal")}
              >
                横向线
              </Button>
              <Button
                size="small"
                icon={<DividingLine theme="outline" size={14} />}
                disabled={busy || columns >= CREATIVE_IMAGE_SPLIT_MAX_GRID}
                aria-label="添加垂直分割线"
                onClick={() => addLine("vertical")}
              >
                纵向线
              </Button>
              <Button
                size="small"
                status="danger"
                icon={<Delete theme="outline" size={14} />}
                disabled={busy || activeLine === null}
                onClick={removeActiveLine}
              >
                删除线
              </Button>
              <Button
                size="small"
                icon={<Refresh theme="outline" size={14} />}
                disabled={busy}
                onClick={resetLines}
              >
                重置线
              </Button>
            </div>
          </section>

          <section className={styles.splitSummary} aria-live="polite">
            <span>切片数量</span>
            <strong>{total} 个</strong>
            <span>平均约</span>
            <strong>
              {averageSize
                ? `${averageSize.width} × ${averageSize.height}`
                : "—"}
            </strong>
          </section>
          <Button
            type="primary"
            className={styles.splitGenerateButton}
            icon={<CheckOne theme="outline" size={14} />}
            loading={busy}
            disabled={!asset || !dimensions || imageFailed}
            onClick={() => onConfirm(params)}
          >
            生成子节点
          </Button>
        </aside>
      </div>

      {busy && progress != null ? (
        <Progress
          percent={Math.round(Math.min(100, Math.max(0, progress)))}
          size="small"
          aria-label="切图结果上传进度"
        />
      ) : null}
      {error ? (
        <div className={styles.cropError} role="alert">
          {error}
        </div>
      ) : null}
    </div>
  );
};

const CreativeImageSplitDialog: React.FC<CreativeImageSplitDialogProps> = (
  props,
) => (
  <Modal
    title="切分图片"
    visible={props.visible}
    className={`${styles.cropModal} ${styles.splitModal}`}
    style={{ width: 780, maxWidth: "calc(100vw - 32px)" }}
    footer={null}
    autoFocus={false}
    maskClosable={!props.busy}
    escToExit={!props.busy}
    closable={!props.busy}
    unmountOnExit
    getPopupContainer={() =>
      document.getElementById("creative-studio-portal-root") ?? document.body
    }
    onCancel={props.onClose}
  >
    <CreativeImageSplitDialogContent {...props} />
  </Modal>
);

export default CreativeImageSplitDialog;
