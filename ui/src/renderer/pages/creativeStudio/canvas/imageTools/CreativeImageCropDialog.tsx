/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Modal, Progress } from "@arco-design/web-react";
import { CheckOne, CloseOne } from "@icon-park/react";
import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { CreativeAsset } from "../../assets";
import {
  CREATIVE_IMAGE_DEFAULT_CROP,
  creativeImageCropToPixels,
  cropForCreativeImageAspect,
  moveCreativeImageCrop,
  resizeCreativeImageCrop,
  type CreativeImageCropAspect,
  type CreativeImageCropHandle,
  type CreativeImageCropRect,
  type CreativeImageDimensions,
} from "./cropModel";
import styles from "./CreativeImageTools.module.css";

export interface CreativeImageCropDialogProps {
  visible: boolean;
  asset: CreativeAsset | null;
  busy?: boolean;
  progress?: number | null;
  error?: string | null;
  onClose(): void;
  onConfirm(crop: CreativeImageCropRect): void;
}

interface CropPointerSession {
  pointerId: number;
  handle: CreativeImageCropHandle;
  clientX: number;
  clientY: number;
  crop: CreativeImageCropRect;
}

const HANDLE_LABEL_KEYS: Record<
  Exclude<CreativeImageCropHandle, "move">,
  string
> = {
  "north-west": "creativeStudio.canvas.imageTools.crop.handles.northWest",
  north: "creativeStudio.canvas.imageTools.crop.handles.north",
  "north-east": "creativeStudio.canvas.imageTools.crop.handles.northEast",
  east: "creativeStudio.canvas.imageTools.crop.handles.east",
  "south-east": "creativeStudio.canvas.imageTools.crop.handles.southEast",
  south: "creativeStudio.canvas.imageTools.crop.handles.south",
  "south-west": "creativeStudio.canvas.imageTools.crop.handles.southWest",
  west: "creativeStudio.canvas.imageTools.crop.handles.west",
};

const HANDLES = Object.keys(HANDLE_LABEL_KEYS) as Exclude<
  CreativeImageCropHandle,
  "move"
>[];

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

const ratioLabel = (width: number, height: number): string => {
  const gcd = (left: number, right: number): number =>
    right === 0 ? left : gcd(right, left % right);
  const divisor = gcd(width, height);
  return `${Math.round(width / divisor)}:${Math.round(height / divisor)}`;
};

export const CreativeImageCropDialogContent: React.FC<
  CreativeImageCropDialogProps
> = ({
  visible,
  asset,
  busy = false,
  progress = null,
  error = null,
  onClose,
  onConfirm,
}) => {
  const { t } = useTranslation();
  const stageRef = useRef<HTMLDivElement>(null);
  const pointerRef = useRef<CropPointerSession | null>(null);
  const [crop, setCrop] = useState<CreativeImageCropRect>(
    CREATIVE_IMAGE_DEFAULT_CROP,
  );
  const [aspect, setAspect] = useState<CreativeImageCropAspect>("free");
  const [dimensions, setDimensions] = useState<CreativeImageDimensions | null>(
    () => finiteDimensions(asset),
  );
  const [imageFailed, setImageFailed] = useState(false);

  useEffect(() => {
    if (!visible) return;
    setCrop(CREATIVE_IMAGE_DEFAULT_CROP);
    setAspect("free");
    setDimensions(finiteDimensions(asset));
    setImageFailed(false);
    pointerRef.current = null;
  }, [asset?.id, visible]);

  const pixels = useMemo(
    () => (dimensions ? creativeImageCropToPixels(crop, dimensions) : null),
    [crop, dimensions],
  );

  const beginPointer = (
    handle: CreativeImageCropHandle,
    event: React.PointerEvent<HTMLElement>,
  ) => {
    if (busy || !dimensions || !stageRef.current) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    pointerRef.current = {
      pointerId: event.pointerId,
      handle,
      clientX: event.clientX,
      clientY: event.clientY,
      crop,
    };
  };

  const applyKeyboardDelta = (
    handle: CreativeImageCropHandle,
    event: React.KeyboardEvent<HTMLElement>,
  ) => {
    if (!dimensions || busy) return;
    const step = event.shiftKey ? 0.04 : 0.01;
    const delta =
      event.key === "ArrowLeft"
        ? { x: -step, y: 0 }
        : event.key === "ArrowRight"
          ? { x: step, y: 0 }
          : event.key === "ArrowUp"
            ? { x: 0, y: -step }
            : event.key === "ArrowDown"
              ? { x: 0, y: step }
              : null;
    if (!delta) return;
    event.preventDefault();
    event.stopPropagation();
    setCrop((current) =>
      handle === "move"
        ? moveCreativeImageCrop(current, delta)
        : resizeCreativeImageCrop(current, handle, delta, dimensions, aspect),
    );
  };

  const updatePointer = (event: React.PointerEvent<HTMLDivElement>) => {
    const session = pointerRef.current;
    const stage = stageRef.current;
    if (
      !session ||
      session.pointerId !== event.pointerId ||
      !stage ||
      !dimensions
    )
      return;
    const bounds = stage.getBoundingClientRect();
    if (bounds.width <= 0 || bounds.height <= 0) return;
    const delta = {
      x: (event.clientX - session.clientX) / bounds.width,
      y: (event.clientY - session.clientY) / bounds.height,
    };
    setCrop(
      session.handle === "move"
        ? moveCreativeImageCrop(session.crop, delta)
        : resizeCreativeImageCrop(
            session.crop,
            session.handle,
            delta,
            dimensions,
            aspect,
          ),
    );
  };

  const endPointer = (event: React.PointerEvent<HTMLDivElement>) => {
    if (pointerRef.current?.pointerId === event.pointerId) {
      pointerRef.current = null;
    }
  };

  const selectAspect = (next: CreativeImageCropAspect) => {
    setAspect(next);
    if (dimensions && next !== "free") {
      setCrop(cropForCreativeImageAspect(dimensions, next));
    }
  };

  const reset = () => {
    setAspect("free");
    setCrop(CREATIVE_IMAGE_DEFAULT_CROP);
  };

  return (
    <div className={styles.cropDialog} data-creative-image-crop-dialog>
      {asset ? (
        <div
          ref={stageRef}
          className={styles.cropStage}
          style={{
            aspectRatio: dimensions
              ? `${dimensions.width} / ${dimensions.height}`
              : "16 / 9",
          }}
          onPointerMove={updatePointer}
          onPointerUp={endPointer}
          onPointerCancel={endPointer}
        >
          {!imageFailed ? (
            <img
              src={asset.originalUrl}
              alt={t("creativeStudio.canvas.imageTools.crop.previewAlt", {
                title: asset.title,
              })}
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
              {t("creativeStudio.canvas.imageTools.crop.loadFailed")}
            </div>
          )}
          {dimensions && !imageFailed ? (
            <div
              className={styles.cropBox}
              style={{
                left: `${crop.x * 100}%`,
                top: `${crop.y * 100}%`,
                width: `${crop.width * 100}%`,
                height: `${crop.height * 100}%`,
              }}
              role="group"
              aria-label={t(
                "creativeStudio.canvas.imageTools.crop.moveBox",
              )}
              tabIndex={busy ? -1 : 0}
              onPointerDown={(event) => beginPointer("move", event)}
              onKeyDown={(event) => applyKeyboardDelta("move", event)}
            >
              <span className={styles.cropGridVerticalOne} aria-hidden="true" />
              <span className={styles.cropGridVerticalTwo} aria-hidden="true" />
              <span
                className={styles.cropGridHorizontalOne}
                aria-hidden="true"
              />
              <span
                className={styles.cropGridHorizontalTwo}
                aria-hidden="true"
              />
              {HANDLES.map((handle) => (
                <button
                  key={handle}
                  type="button"
                  className={styles.cropHandle}
                  data-crop-handle={handle}
                  aria-label={t(
                    "creativeStudio.canvas.imageTools.crop.resizeBox",
                    { handle: t(HANDLE_LABEL_KEYS[handle]) },
                  )}
                  disabled={busy}
                  onPointerDown={(event) => beginPointer(handle, event)}
                  onKeyDown={(event) => applyKeyboardDelta(handle, event)}
                />
              ))}
            </div>
          ) : null}
        </div>
      ) : null}

      <div className={styles.cropMetaBar}>
        <div className={styles.cropMetrics} aria-live="polite">
          <span>
            {t("creativeStudio.canvas.imageTools.crop.metrics.size", {
              value: pixels ? `${pixels.width} × ${pixels.height}` : "—",
            })}
          </span>
          <span>
            {t("creativeStudio.canvas.imageTools.crop.metrics.ratio", {
              value: pixels
                ? ratioLabel(pixels.width, pixels.height)
                : "—",
            })}
          </span>
          <span>
            {t("creativeStudio.canvas.imageTools.crop.metrics.original", {
              value: dimensions
                ? `${dimensions.width} × ${dimensions.height}`
                : "—",
            })}
          </span>
        </div>
        <label className={styles.aspectSelect}>
          <span>
            {t("creativeStudio.canvas.imageTools.crop.aspectLabel")}
          </span>
          <select
            value={aspect}
            disabled={busy || !dimensions}
            onChange={(event) =>
              selectAspect(event.target.value as CreativeImageCropAspect)
            }
          >
            <option value="free">
              {t("creativeStudio.canvas.imageTools.crop.aspectFree")}
            </option>
            <option value="1:1">1:1</option>
            <option value="4:3">4:3</option>
            <option value="16:9">16:9</option>
          </select>
        </label>
      </div>

      {busy && progress != null ? (
        <Progress
          percent={Math.round(Math.min(100, Math.max(0, progress)))}
          size="small"
          aria-label={t(
            "creativeStudio.canvas.imageTools.crop.uploadProgress",
          )}
        />
      ) : null}
      {error ? (
        <div className={styles.cropError} role="alert">
          {error}
        </div>
      ) : null}

      <footer className={styles.cropFooter}>
        <Button disabled={busy} onClick={reset}>
          {t("creativeStudio.canvas.actions.reset")}
        </Button>
          <Button
            icon={<CloseOne theme="outline" size={14} />}
            disabled={busy}
            onClick={onClose}
          >
            {t("creativeStudio.canvas.actions.cancel")}
          </Button>
          <Button
            type="primary"
            icon={<CheckOne theme="outline" size={14} />}
            loading={busy}
          disabled={!asset || !dimensions || imageFailed}
          onClick={() => onConfirm(crop)}
        >
          {t("creativeStudio.canvas.imageTools.crop.confirm")}
        </Button>
      </footer>
    </div>
  );
};

const CreativeImageCropDialog: React.FC<CreativeImageCropDialogProps> = (
  props,
) => {
  const { t } = useTranslation();

  return (
    <Modal
      title={t("creativeStudio.canvas.imageTools.crop.title")}
      visible={props.visible}
      className={styles.cropModal}
      style={{ width: 780, maxWidth: "calc(100vw - 32px)" }}
      footer={null}
      maskClosable={!props.busy}
      escToExit={!props.busy}
      closable={!props.busy}
      unmountOnExit
      getPopupContainer={() =>
        document.getElementById("creative-studio-portal-root") ??
        document.body
      }
      onCancel={props.onClose}
    >
      <CreativeImageCropDialogContent {...props} />
    </Modal>
  );
};

export default CreativeImageCropDialog;
