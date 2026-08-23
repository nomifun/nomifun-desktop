/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Compass, FullScreen, ZoomIn, ZoomOut } from '@icon-park/react';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

import styles from './CanvasZoomControls.module.css';

export interface CanvasZoomControlLabels {
  zoomOut: string;
  zoomIn: string;
  zoomSlider: string;
  resetView: string;
  fitView: string;
  openMiniMap: string;
  closeMiniMap: string;
}

export interface CanvasZoomControlsProps {
  zoom: number;
  minZoom?: number;
  maxZoom?: number;
  disabled?: boolean;
  isMiniMapOpen?: boolean;
  labels?: Partial<CanvasZoomControlLabels>;
  onZoomChange?: (zoom: number) => void;
  onResetView?: () => void;
  onFitView?: () => void;
  onToggleMiniMap?: () => void;
}

const DEFAULT_MIN_ZOOM = 0.05;
const DEFAULT_MAX_ZOOM = 4;
const ZOOM_FACTOR = 1.2;

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

/**
 * A fully controlled zoom dock. It calculates user intent, while the owning
 * canvas remains the sole authority for the viewport that is rendered.
 */
const CanvasZoomControls: React.FC<CanvasZoomControlsProps> = ({
  zoom,
  minZoom = DEFAULT_MIN_ZOOM,
  maxZoom = DEFAULT_MAX_ZOOM,
  disabled = false,
  isMiniMapOpen = false,
  labels,
  onZoomChange,
  onResetView,
  onFitView,
  onToggleMiniMap,
}) => {
  const { t } = useTranslation();
  const controlLabels: CanvasZoomControlLabels = {
    zoomOut: t('creativeStudio.canvas.zoom.zoomOut'),
    zoomIn: t('creativeStudio.canvas.zoom.zoomIn'),
    zoomSlider: t('creativeStudio.canvas.zoom.slider'),
    resetView: t('creativeStudio.canvas.zoom.resetView'),
    fitView: t('creativeStudio.canvas.actions.fitView'),
    openMiniMap: t('creativeStudio.canvas.actions.openMiniMap'),
    closeMiniMap: t('creativeStudio.canvas.actions.closeMiniMap'),
    ...labels,
  };
  const safeMinZoom = Math.max(0.001, Math.min(minZoom, maxZoom));
  const safeMaxZoom = Math.max(safeMinZoom, maxZoom);
  const safeZoom = Number.isFinite(zoom) ? clamp(zoom, safeMinZoom, safeMaxZoom) : 1;
  const logarithmicMin = Math.log(safeMinZoom);
  const logarithmicRange = Math.max(Math.log(safeMaxZoom) - logarithmicMin, Number.EPSILON);
  const sliderValue = (Math.log(safeZoom) - logarithmicMin) / logarithmicRange;
  const percentage = useMemo(() => `${Math.round(safeZoom * 100)}%`, [safeZoom]);
  const zoomDisabled = disabled || !onZoomChange;

  const emitSliderZoom = (value: number) => {
    onZoomChange?.(Math.exp(logarithmicMin + clamp(value, 0, 1) * logarithmicRange));
  };

  return (
    <div className={styles.controls} data-canvas-no-zoom data-canvas-zoom-controls>
      <button
        type='button'
        className={styles.iconButton}
        title={controlLabels.zoomOut}
        aria-label={controlLabels.zoomOut}
        disabled={zoomDisabled || safeZoom <= safeMinZoom}
        onClick={() => onZoomChange?.(clamp(safeZoom / ZOOM_FACTOR, safeMinZoom, safeMaxZoom))}
      >
        <ZoomOut theme='outline' size={16} fill='currentColor' strokeWidth={3} />
      </button>

      <input
        className={styles.slider}
        type='range'
        min={0}
        max={1}
        step={0.001}
        value={sliderValue}
        aria-label={controlLabels.zoomSlider}
        disabled={zoomDisabled}
        onChange={(event) => emitSliderZoom(Number(event.currentTarget.value))}
      />

      <button
        type='button'
        className={styles.iconButton}
        title={controlLabels.zoomIn}
        aria-label={controlLabels.zoomIn}
        disabled={zoomDisabled || safeZoom >= safeMaxZoom}
        onClick={() => onZoomChange?.(clamp(safeZoom * ZOOM_FACTOR, safeMinZoom, safeMaxZoom))}
      >
        <ZoomIn theme='outline' size={16} fill='currentColor' strokeWidth={3} />
      </button>

      <button
        type='button'
        className={styles.percentageButton}
        title={controlLabels.resetView}
        aria-label={t('creativeStudio.canvas.zoom.resetViewCurrent', {
          action: controlLabels.resetView,
          percentage,
        })}
        disabled={disabled || !onResetView}
        onClick={onResetView}
      >
        {percentage}
      </button>

      <span className={styles.divider} aria-hidden='true' />

      <button
        type='button'
        className={styles.iconButton}
        title={controlLabels.fitView}
        aria-label={controlLabels.fitView}
        disabled={disabled || !onFitView}
        onClick={onFitView}
      >
        <FullScreen theme='outline' size={16} fill='currentColor' strokeWidth={3} />
      </button>

      <button
        type='button'
        className={styles.iconButton}
        data-active={isMiniMapOpen || undefined}
        title={isMiniMapOpen ? controlLabels.closeMiniMap : controlLabels.openMiniMap}
        aria-label={isMiniMapOpen ? controlLabels.closeMiniMap : controlLabels.openMiniMap}
        aria-pressed={isMiniMapOpen}
        disabled={disabled || !onToggleMiniMap}
        onClick={onToggleMiniMap}
      >
        <Compass theme='outline' size={16} fill='currentColor' strokeWidth={3} />
      </button>
    </div>
  );
};

export default CanvasZoomControls;
