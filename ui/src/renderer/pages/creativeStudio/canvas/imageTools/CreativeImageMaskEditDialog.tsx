/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Input, Modal, Progress, Slider } from "@arco-design/web-react";
import { CloseOne, Erase, MagicWand, Paint, Refresh } from "@icon-park/react";
import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { CreativeAsset } from "../../assets";
import {
  CreativeModelSelect,
  type CreativeModelCatalogSnapshot,
  type CreativeModelSelectionRef,
} from "../../models";
import type { CreativeImageMaskSelection } from "./browserMask";
import {
  CREATIVE_IMAGE_MASK_BRUSH_DEFAULT,
  CREATIVE_IMAGE_MASK_BRUSH_MAX,
  CREATIVE_IMAGE_MASK_BRUSH_MIN,
  CREATIVE_IMAGE_MASK_BRUSH_STEP,
  CREATIVE_IMAGE_MASK_FILL,
  creativeImageMaskHasPaint,
  creativeImageMaskPoint,
  normalizeCreativeImageMaskBrush,
  validateCreativeImageMaskEdit,
  type CreativeImageMaskDrawMode,
  type CreativeImageMaskPoint,
  type CreativeImageMaskValidationError,
} from "./maskModel";
import styles from "./CreativeImageTools.module.css";

export interface CreativeImageMaskEditSubmit {
  prompt: string;
  model: CreativeModelSelectionRef;
  selection: CreativeImageMaskSelection;
}

export interface CreativeImageMaskEditDialogProps {
  visible: boolean;
  asset: CreativeAsset | null;
  catalog: CreativeModelCatalogSnapshot;
  model: CreativeModelSelectionRef | null;
  busy?: boolean;
  /** Preserve the exact mask, prompt, model, and idempotency key after an uncertain POST. */
  retryLocked?: boolean;
  progress?: number | null;
  error?: string | null;
  onModelChange(model: CreativeModelSelectionRef): void;
  onOpenModelSettings?: () => void;
  onAbandon?: () => void;
  onClose(): void;
  onConfirm(input: CreativeImageMaskEditSubmit): void;
}

interface MaskPointerSession {
  pointerId: number;
  last: CreativeImageMaskPoint | null;
}

interface MaskDimensions {
  width: number;
  height: number;
}

const iconProps = {
  theme: "outline" as const,
  size: 15,
  fill: "currentColor",
  strokeWidth: 3,
};

const VALIDATION_ERROR_KEYS: Record<
  CreativeImageMaskValidationError,
  string
> = {
  promptRequired:
    "creativeStudio.canvas.imageTools.mask.validation.promptRequired",
  maskRequired:
    "creativeStudio.canvas.imageTools.mask.validation.maskRequired",
  modelRequired:
    "creativeStudio.canvas.imageTools.mask.validation.modelRequired",
};

const finiteDimensions = (
  asset: CreativeAsset | null,
): MaskDimensions | null =>
  asset &&
  asset.kind === "image" &&
  Number.isFinite(asset.width) &&
  Number.isFinite(asset.height) &&
  (asset.width ?? 0) > 0 &&
  (asset.height ?? 0) > 0
    ? { width: asset.width as number, height: asset.height as number }
    : null;

const clearCanvas = (canvas: HTMLCanvasElement | null): void => {
  const context = canvas?.getContext("2d");
  if (!canvas || !context) return;
  context.clearRect(0, 0, canvas.width, canvas.height);
};

const maskHasPaint = (canvas: HTMLCanvasElement): boolean => {
  const context = canvas.getContext("2d");
  return context
    ? creativeImageMaskHasPaint(
        context.getImageData(0, 0, canvas.width, canvas.height).data,
      )
    : false;
};

const maskEdge = (
  data: Uint8ClampedArray,
  width: number,
  x: number,
  y: number,
  step: number,
): boolean =>
  data[((y - step) * width + x) * 4 + 3] === 0 ||
  data[((y + step) * width + x) * 4 + 3] === 0 ||
  data[(y * width + x - step) * 4 + 3] === 0 ||
  data[(y * width + x + step) * 4 + 3] === 0;

const drawMaskBorder = (
  context: CanvasRenderingContext2D,
  maskCanvas: HTMLCanvasElement,
): void => {
  const maskContext = maskCanvas.getContext("2d");
  if (!maskContext) return;
  const { width, height } = maskCanvas;
  const data = maskContext.getImageData(0, 0, width, height).data;
  const step = Math.max(1, Math.round(Math.max(width, height) / 1_200));
  const dash = step * 8;
  const period = dash + step * 5;
  context.save();
  context.fillStyle = "rgba(255, 255, 255, 0.72)";
  context.shadowColor = "rgba(0, 0, 0, 0.24)";
  context.shadowBlur = step * 1.5;
  for (let y = step; y < height - step; y += step) {
    for (let x = step; x < width - step; x += step) {
      const alpha = data[(y * width + x) * 4 + 3];
      if (!alpha || !maskEdge(data, width, x, y, step)) continue;
      if ((x + y) % period > dash) continue;
      context.fillRect(
        x - step / 2,
        y - step / 2,
        Math.max(1.5, step),
        Math.max(1.5, step),
      );
    }
  }
  context.restore();
};

const renderMaskPreview = (
  maskCanvas: HTMLCanvasElement,
  previewCanvas: HTMLCanvasElement | null,
  withBorder = false,
): void => {
  const context = previewCanvas?.getContext("2d");
  if (!previewCanvas || !context) return;
  context.clearRect(0, 0, previewCanvas.width, previewCanvas.height);
  context.fillStyle = CREATIVE_IMAGE_MASK_FILL;
  context.fillRect(0, 0, previewCanvas.width, previewCanvas.height);
  context.globalCompositeOperation = "destination-in";
  context.drawImage(maskCanvas, 0, 0);
  context.globalCompositeOperation = "source-over";
  if (withBorder) drawMaskBorder(context, maskCanvas);
};

const drawMaskStroke = (
  context: CanvasRenderingContext2D,
  from: CreativeImageMaskPoint,
  to: CreativeImageMaskPoint,
  brushSize: number,
): void => {
  if (from.x === to.x && from.y === to.y) {
    context.beginPath();
    context.arc(to.x, to.y, brushSize / 2, 0, Math.PI * 2);
    context.fill();
    return;
  }
  context.beginPath();
  context.moveTo(from.x, from.y);
  context.lineTo(to.x, to.y);
  context.stroke();
};

export const CreativeImageMaskEditDialogContent: React.FC<
  CreativeImageMaskEditDialogProps
> = ({
  visible,
  asset,
  catalog,
  model,
  busy = false,
  retryLocked = false,
  progress = null,
  error = null,
  onModelChange,
  onOpenModelSettings,
  onAbandon,
  onClose,
  onConfirm,
}) => {
  const { t } = useTranslation();
  const maskCanvasRef = useRef<HTMLCanvasElement>(null);
  const previewCanvasRef = useRef<HTMLCanvasElement>(null);
  const pointerRef = useRef<MaskPointerSession | null>(null);
  const [dimensions, setDimensions] = useState<MaskDimensions | null>(() =>
    finiteDimensions(asset),
  );
  const [imageFailed, setImageFailed] = useState(false);
  const [prompt, setPrompt] = useState("");
  const [brushSize, setBrushSize] = useState(CREATIVE_IMAGE_MASK_BRUSH_DEFAULT);
  const [mode, setMode] = useState<CreativeImageMaskDrawMode>("paint");
  const [validationError, setValidationError] =
    useState<CreativeImageMaskValidationError | null>(null);
  const draftLocked = busy || retryLocked;

  useEffect(() => {
    if (!visible) return;
    setDimensions(finiteDimensions(asset));
    setImageFailed(false);
    setPrompt("");
    setBrushSize(CREATIVE_IMAGE_MASK_BRUSH_DEFAULT);
    setMode("paint");
    setValidationError(null);
    pointerRef.current = null;
  }, [asset?.id, visible]);

  useEffect(() => {
    clearCanvas(maskCanvasRef.current);
    clearCanvas(previewCanvasRef.current);
  }, [dimensions?.height, dimensions?.width]);

  const draw = (event: React.PointerEvent<HTMLCanvasElement>): void => {
    const maskCanvas = maskCanvasRef.current;
    const context = maskCanvas?.getContext("2d");
    if (!maskCanvas || !context || draftLocked) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    const point = creativeImageMaskPoint(bounds, maskCanvas, {
      x: event.clientX,
      y: event.clientY,
    });
    context.lineCap = "round";
    context.lineJoin = "round";
    context.lineWidth = brushSize;
    context.globalCompositeOperation =
      mode === "paint" ? "source-over" : "destination-out";
    context.strokeStyle = "#000";
    context.fillStyle = "#000";
    drawMaskStroke(
      context,
      pointerRef.current?.last ?? point,
      point,
      brushSize,
    );
    renderMaskPreview(maskCanvas, previewCanvasRef.current);
    if (pointerRef.current) pointerRef.current.last = point;
    if (mode === "paint") setValidationError(null);
  };

  const startDraw = (event: React.PointerEvent<HTMLCanvasElement>): void => {
    if (draftLocked || !dimensions) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    pointerRef.current = { pointerId: event.pointerId, last: null };
    draw(event);
  };

  const moveDraw = (event: React.PointerEvent<HTMLCanvasElement>): void => {
    if (pointerRef.current?.pointerId !== event.pointerId) return;
    event.preventDefault();
    draw(event);
  };

  const stopDraw = (event: React.PointerEvent<HTMLCanvasElement>): void => {
    if (pointerRef.current?.pointerId !== event.pointerId) return;
    pointerRef.current = null;
    const maskCanvas = maskCanvasRef.current;
    if (maskCanvas) {
      renderMaskPreview(
        maskCanvas,
        previewCanvasRef.current,
        maskHasPaint(maskCanvas),
      );
    }
  };

  const reset = (): void => {
    clearCanvas(maskCanvasRef.current);
    clearCanvas(previewCanvasRef.current);
    setValidationError(null);
  };

  const submit = (): void => {
    const maskCanvas = maskCanvasRef.current;
    if (!maskCanvas) return;
    const nextError = validateCreativeImageMaskEdit({
      prompt,
      hasMask: maskHasPaint(maskCanvas),
      hasModel: model !== null,
    });
    if (nextError || !model) {
      setValidationError(nextError);
      return;
    }
    setValidationError(null);
    onConfirm({
      prompt: prompt.trim(),
      model,
      selection: {
        width: maskCanvas.width,
        height: maskCanvas.height,
        source: maskCanvas,
      },
    });
  };

  const shownError = validationError
    ? t(VALIDATION_ERROR_KEYS[validationError])
    : error;

  return (
    <div className={styles.maskDialog} data-creative-image-mask-edit-dialog>
      <div className={styles.maskPreviewPane}>
        {asset ? (
          <div className={styles.maskStage}>
            {!imageFailed ? (
              <img
                src={asset.originalUrl}
                alt={t(
                  "creativeStudio.canvas.imageTools.mask.previewAlt",
                  { title: asset.title },
                )}
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
                {t("creativeStudio.canvas.imageTools.mask.loadFailed")}
              </div>
            )}
            {dimensions && !imageFailed ? (
              <>
                <canvas
                  ref={maskCanvasRef}
                  width={dimensions.width}
                  height={dimensions.height}
                  className={styles.maskSourceCanvas}
                  aria-hidden="true"
                />
                <canvas
                  ref={previewCanvasRef}
                  width={dimensions.width}
                  height={dimensions.height}
                  className={styles.maskPreviewCanvas}
                  aria-label={t(
                    "creativeStudio.canvas.imageTools.mask.canvasLabel",
                  )}
                  onPointerDown={startDraw}
                  onPointerMove={moveDraw}
                  onPointerUp={stopDraw}
                  onPointerCancel={stopDraw}
                />
              </>
            ) : null}
          </div>
        ) : null}
      </div>

      <section className={styles.maskControls}>
        <header>
          <h2>{t("creativeStudio.canvas.imageTools.mask.heading")}</h2>
          <p>
            {dimensions
              ? `${dimensions.width} × ${dimensions.height}px`
              : t("creativeStudio.canvas.imageTools.mask.reading")}
          </p>
        </header>

        <div className={styles.maskModeGrid}>
          <Button
            type={mode === "paint" ? "primary" : "default"}
            icon={<Paint {...iconProps} />}
            disabled={draftLocked}
            onClick={() => setMode("paint")}
          >
            {t("creativeStudio.canvas.imageTools.mask.paint")}
          </Button>
          <Button
            type={mode === "erase" ? "primary" : "default"}
            icon={<Erase {...iconProps} />}
            disabled={draftLocked}
            onClick={() => setMode("erase")}
          >
            {t("creativeStudio.canvas.imageTools.mask.erase")}
          </Button>
        </div>

        <div className={styles.maskField}>
          <div className={styles.maskFieldLabel}>
            <span>
              {t("creativeStudio.canvas.imageTools.mask.brushSize")}
            </span>
            <strong>{brushSize}px</strong>
          </div>
          <Slider
            min={CREATIVE_IMAGE_MASK_BRUSH_MIN}
            max={CREATIVE_IMAGE_MASK_BRUSH_MAX}
            step={CREATIVE_IMAGE_MASK_BRUSH_STEP}
            value={brushSize}
            disabled={draftLocked}
            onChange={(value) =>
              setBrushSize(
                normalizeCreativeImageMaskBrush(
                  Array.isArray(value) ? (value[0] ?? brushSize) : value,
                ),
              )
            }
          />
        </div>

        <div className={styles.maskField}>
          <label
            className={styles.maskFieldTitle}
            htmlFor="creative-mask-edit-prompt"
          >
            {t("creativeStudio.canvas.imageTools.mask.promptLabel")}
          </label>
          <Input.TextArea
            id="creative-mask-edit-prompt"
            rows={6}
            value={prompt}
            disabled={draftLocked}
            status={validationError === "promptRequired" ? "error" : undefined}
            placeholder={t(
              "creativeStudio.canvas.imageTools.mask.promptPlaceholder",
            )}
            onChange={(value) => {
              setPrompt(value);
              setValidationError(null);
            }}
          />
          {shownError ? (
            <div className={styles.maskError} role="alert">
              {shownError}
            </div>
          ) : null}
        </div>

        <CreativeModelSelect
          catalog={catalog}
          filter={{ capability: "task", task: "image_edit" }}
          value={model}
          disabled={draftLocked}
          label={t("creativeStudio.canvas.imageTools.mask.modelLabel")}
          onChange={(selection) => {
            setValidationError(null);
            onModelChange(selection);
          }}
          onOpenModelSettings={onOpenModelSettings}
        />

        {busy && progress !== null ? (
          <Progress
            percent={Math.round(Math.min(100, Math.max(0, progress)))}
            size="small"
            aria-label={t(
              "creativeStudio.canvas.imageTools.mask.uploadProgress",
            )}
          />
        ) : null}

        <footer className={styles.maskFooter}>
          <Button
            icon={<Refresh {...iconProps} />}
            disabled={draftLocked}
            onClick={reset}
          >
            {t("creativeStudio.canvas.actions.reset")}
          </Button>
          <div>
            <Button
              icon={<CloseOne {...iconProps} />}
              disabled={busy || (retryLocked && !onAbandon)}
              onClick={retryLocked ? onAbandon : onClose}
            >
              {t(
                retryLocked
                  ? "creativeStudio.canvas.imageTools.mask.abandon"
                  : "creativeStudio.canvas.actions.cancel",
              )}
            </Button>
            <Button
              type="primary"
              icon={<MagicWand {...iconProps} />}
              loading={busy}
              disabled={busy || !asset || !dimensions || imageFailed}
              onClick={submit}
            >
              {t(
                retryLocked
                  ? "creativeStudio.canvas.imageTools.mask.safeRetry"
                  : "creativeStudio.canvas.imageTools.mask.submit",
              )}
            </Button>
          </div>
        </footer>
      </section>
    </div>
  );
};

const CreativeImageMaskEditDialog: React.FC<
  CreativeImageMaskEditDialogProps
> = (props) => (
  <Modal
    title={null}
    visible={props.visible && Boolean(props.asset)}
    className={styles.maskModal}
    style={{ width: 980, maxWidth: "calc(100vw - 32px)" }}
    footer={null}
    maskClosable={!props.busy && !props.retryLocked}
    escToExit={!props.busy && !props.retryLocked}
    closable={!props.busy && !props.retryLocked}
    unmountOnExit
    getPopupContainer={() =>
      document.getElementById("creative-studio-portal-root") ?? document.body
    }
    onCancel={props.onClose}
  >
    <CreativeImageMaskEditDialogContent {...props} />
  </Modal>
);

export default CreativeImageMaskEditDialog;
