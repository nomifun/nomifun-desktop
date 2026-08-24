/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  CheckOne,
  Compass,
  Down,
  FullScreen,
  Minus,
  Plus,
  Up,
} from '@icon-park/react';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import styles from './CanvasZoomControls.module.css';

export type CanvasZoomBackground = 'dots' | 'lines' | 'blank';

export interface CanvasZoomControlLabels {
  zoomOut: string;
  zoomIn: string;
  zoomSlider: string;
  resetView: string;
  fitView: string;
  openMiniMap: string;
  closeMiniMap: string;
  zoomMenu: string;
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
  background?: CanvasZoomBackground;
  onBackgroundChange?: (background: CanvasZoomBackground) => void;
  /** Show +/- beside the percentage trigger when embedded in the minimap footer. */
  showInlineStepper?: boolean;
}

const DEFAULT_MIN_ZOOM = 0.05;
const DEFAULT_MAX_ZOOM = 4;
const ZOOM_FACTOR = 1.2;
const BACKGROUND_OPTIONS: readonly CanvasZoomBackground[] = ['blank', 'lines', 'dots'];

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

const iconProps = {
  theme: 'outline' as const,
  size: 16,
  fill: 'currentColor',
  strokeWidth: 3,
};

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
  background = 'lines',
  onBackgroundChange,
  showInlineStepper = false,
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
    zoomMenu: t('creativeStudio.canvas.zoom.slider'),
    ...labels,
  };
  const safeMinZoom = Math.max(0.001, Math.min(minZoom, maxZoom));
  const safeMaxZoom = Math.max(safeMinZoom, maxZoom);
  const safeZoom = Number.isFinite(zoom) ? clamp(zoom, safeMinZoom, safeMaxZoom) : 1;
  const percentageValue = useMemo(() => Math.round(safeZoom * 100), [safeZoom]);
  const percentage = `${percentageValue}%`;
  const zoomDisabled = disabled || !onZoomChange;
  const selectedBackground = background;
  const controlsRef = useRef<HTMLDivElement>(null);
  const [zoomMenuOpen, setZoomMenuOpen] = useState(false);
  const [zoomInput, setZoomInput] = useState(String(percentageValue));

  useEffect(() => {
    setZoomInput(String(percentageValue));
  }, [percentageValue]);

  useEffect(() => {
    if (!zoomMenuOpen) return;
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && !controlsRef.current?.contains(target)) {
        setZoomMenuOpen(false);
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setZoomMenuOpen(false);
      }
    };
    document.addEventListener('pointerdown', handlePointerDown);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [zoomMenuOpen]);

  const updateZoom = (nextZoom: number) => {
    const next = clamp(nextZoom, safeMinZoom, safeMaxZoom);
    setZoomInput(String(Math.round(next * 100)));
    onZoomChange?.(next);
  };

  const commitZoomInput = (rawValue = zoomInput) => {
    const value = Number(rawValue);
    if (!Number.isFinite(value)) {
      setZoomInput(String(percentageValue));
      return;
    }
    updateZoom(value / 100);
  };

  const zoomMenu = (
    <div className={styles.zoomMenu} role='menu' aria-label={controlLabels.zoomMenu}>
      <div className={styles.backgroundHeading}>
        {t('creativeStudio.canvas.chrome.canvasBackground')}
      </div>
      <div className={styles.backgroundOptions}>
        {BACKGROUND_OPTIONS.map((background) => (
          <button
            key={background}
            type='button'
            role='menuitemradio'
            className={styles.backgroundOption}
            aria-checked={background === selectedBackground}
            data-active={background === selectedBackground || undefined}
            disabled={disabled || !onBackgroundChange}
            onClick={() => {
              onBackgroundChange?.(background);
              setZoomMenuOpen(false);
            }}
          >
            <span className={styles.selectionIndicator} aria-hidden='true'>
              {background === selectedBackground ? <CheckOne {...iconProps} /> : null}
            </span>
            <span>{t(`creativeStudio.canvas.backgrounds.${background}`)}</span>
          </button>
        ))}
      </div>
      <div className={styles.zoomStepper} data-canvas-zoom-stepper>
        <button
          type='button'
          className={styles.stepButton}
          title={controlLabels.zoomOut}
          aria-label={controlLabels.zoomOut}
          disabled={zoomDisabled || safeZoom <= safeMinZoom}
          onClick={() => updateZoom(safeZoom / ZOOM_FACTOR)}
        >
          <Minus {...iconProps} />
        </button>
        <label className={styles.zoomInput}>
          <input
            type='text'
            inputMode='numeric'
            pattern='[0-9]*'
            value={zoomInput}
            aria-label={controlLabels.zoomSlider}
            disabled={zoomDisabled}
            onChange={(event) => setZoomInput(event.currentTarget.value)}
            onBlur={(event) => commitZoomInput(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault();
                commitZoomInput(event.currentTarget.value);
              }
            }}
          />
          <span aria-hidden='true'>%</span>
        </label>
        <button
          type='button'
          className={styles.stepButton}
          title={controlLabels.zoomIn}
          aria-label={controlLabels.zoomIn}
          disabled={zoomDisabled || safeZoom >= safeMaxZoom}
          onClick={() => updateZoom(safeZoom * ZOOM_FACTOR)}
        >
          <Plus {...iconProps} />
        </button>
      </div>
    </div>
  );

  return (
    <div
      ref={controlsRef}
      className={styles.controls}
      data-canvas-no-zoom
      data-canvas-zoom-controls
      data-embedded={showInlineStepper || undefined}
    >
      <div className={styles.menuAnchor}>
        {zoomMenuOpen ? (
          <div className={styles.zoomMenuPopover}>{zoomMenu}</div>
        ) : null}
        {showInlineStepper ? (
          <button
            type='button'
            className={styles.stepButton}
            title={controlLabels.zoomOut}
            aria-label={controlLabels.zoomOut}
            disabled={zoomDisabled || safeZoom <= safeMinZoom}
            onClick={() => updateZoom(safeZoom / ZOOM_FACTOR)}
          >
            <Minus {...iconProps} />
          </button>
        ) : null}
        <button
          type='button'
          className={styles.percentageButton}
          title={controlLabels.zoomMenu}
          aria-label={`${controlLabels.zoomMenu}, ${percentage}`}
          aria-expanded={zoomMenuOpen}
          aria-haspopup='menu'
          data-active={zoomMenuOpen || undefined}
          disabled={disabled}
          onClick={() => setZoomMenuOpen((open) => !open)}
          onDoubleClick={() => {
            setZoomMenuOpen(false);
            onResetView?.();
          }}
        >
          <span>{percentage}</span>
          {zoomMenuOpen ? <Up {...iconProps} size={13} /> : <Down {...iconProps} size={13} />}
        </button>
        {showInlineStepper ? (
          <button
            type='button'
            className={styles.stepButton}
            title={controlLabels.zoomIn}
            aria-label={controlLabels.zoomIn}
            disabled={zoomDisabled || safeZoom >= safeMaxZoom}
            onClick={() => updateZoom(safeZoom * ZOOM_FACTOR)}
          >
            <Plus {...iconProps} />
          </button>
        ) : null}
      </div>

      <span className={styles.divider} aria-hidden='true' />

      <button
        type='button'
        className={styles.iconButton}
        title={controlLabels.fitView}
        aria-label={controlLabels.fitView}
        disabled={disabled || !onFitView}
        onClick={onFitView}
      >
        <FullScreen {...iconProps} />
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
        <Compass {...iconProps} />
      </button>
    </div>
  );
};

export default CanvasZoomControls;
