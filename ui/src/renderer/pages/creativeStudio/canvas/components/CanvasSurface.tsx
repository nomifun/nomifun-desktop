/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React from 'react';
import { useTranslation } from 'react-i18next';

import CanvasMiniMapFrame from './CanvasMiniMapFrame';
import CanvasZoomControls, { type CanvasZoomControlsProps } from './CanvasZoomControls';
import styles from './CanvasSurface.module.css';

export type CanvasBackgroundMode = 'dots' | 'lines' | 'blank';
export type CanvasInteractionTool = 'select' | 'pan';

export interface CanvasSurfaceViewport {
  x: number;
  y: number;
  zoom: number;
}

export interface CanvasSelectionRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface CanvasSurfaceProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, 'children' | 'onDoubleClick'> {
  viewport: CanvasSurfaceViewport;
  backgroundMode?: CanvasBackgroundMode;
  tool?: CanvasInteractionTool;
  isPanning?: boolean;
  gridStep?: number;
  ariaLabel?: string;
  edgeLayer?: React.ReactNode;
  nodeLayer?: React.ReactNode;
  worldOverlay?: React.ReactNode;
  screenOverlay?: React.ReactNode;
  selectionRect?: CanvasSelectionRect | null;
  topDock?: React.ReactNode;
  leftDock?: React.ReactNode;
  rightDock?: React.ReactNode;
  bottomDock?: React.ReactNode;
  miniMap?: React.ReactNode;
  miniMapLabel?: string;
  isMiniMapOpen?: boolean;
  zoomControls?: Omit<CanvasZoomControlsProps, 'zoom' | 'isMiniMapOpen'> | false;
  onDoubleClick?: React.MouseEventHandler<HTMLDivElement>;
}

type CanvasSurfaceStyle = React.CSSProperties & {
  '--creative-canvas-grid-size': string;
  '--creative-canvas-grid-x': string;
  '--creative-canvas-grid-y': string;
  '--creative-canvas-inverse-zoom': number;
};

const finiteOr = (value: number, fallback: number) => (Number.isFinite(value) ? value : fallback);

const normaliseSelectionRect = (selection: CanvasSelectionRect) => ({
  left: selection.width >= 0 ? selection.x : selection.x + selection.width,
  top: selection.height >= 0 ? selection.y : selection.y + selection.height,
  width: Math.abs(selection.width),
  height: Math.abs(selection.height),
});

const stopChromeEvent = (event: React.SyntheticEvent) => event.stopPropagation();

/**
 * Product-level infinite-canvas surface.
 *
 * This component is intentionally store-agnostic. The owning controller
 * supplies viewport state, layers, chrome and every state-changing callback;
 * the surface only establishes screen/world coordinate spaces and layering.
 */
const CanvasSurface = React.forwardRef<HTMLDivElement, CanvasSurfaceProps>(
  (
    {
      viewport,
      backgroundMode = 'dots',
      tool = 'select',
      isPanning = false,
      gridStep = 48,
      ariaLabel,
      edgeLayer,
      nodeLayer,
      worldOverlay,
      screenOverlay,
      selectionRect,
      topDock,
      leftDock,
      rightDock,
      bottomDock,
      miniMap,
      miniMapLabel,
      isMiniMapOpen = false,
      zoomControls,
      className,
      style,
      ...interactionProps
    },
    ref
  ) => {
    const { t } = useTranslation();
    const safeZoom = Math.max(0.001, finiteOr(viewport.zoom, 1));
    const safeX = finiteOr(viewport.x, 0);
    const safeY = finiteOr(viewport.y, 0);
    const safeGridStep = Math.max(4, finiteOr(gridStep, 48));
    const screenGridSize = safeGridStep * safeZoom;
    const gridStyle: CanvasSurfaceStyle = {
      '--creative-canvas-grid-size': `${screenGridSize}px`,
      '--creative-canvas-grid-x': `${safeX % screenGridSize}px`,
      '--creative-canvas-grid-y': `${safeY % screenGridSize}px`,
      '--creative-canvas-inverse-zoom': 1 / safeZoom,
    };
    const worldStyle: React.CSSProperties = {
      transform: `translate3d(${safeX}px, ${safeY}px, 0) scale(${safeZoom})`,
    };
    const selectionStyle = selectionRect ? normaliseSelectionRect(selectionRect) : undefined;
    const shouldRenderZoomControls = zoomControls !== false;

    return (
      <div
        {...interactionProps}
        ref={ref}
        className={classNames(styles.surface, className)}
        style={{ ...style, ...gridStyle }}
        role='region'
        aria-label={ariaLabel ?? t('creativeStudio.canvas.surface.label')}
        data-canvas-surface
        data-canvas-background={backgroundMode}
        data-canvas-tool={tool}
        data-canvas-panning={isPanning || undefined}
      >
        <div className={classNames(styles.grid, styles[`grid_${backgroundMode}`])} aria-hidden='true' />

        <div className={styles.world} style={worldStyle} data-canvas-world>
          <div className={styles.edgeLayer} data-canvas-layer='edges'>
            {edgeLayer}
          </div>
          <div className={styles.nodeLayer} data-canvas-layer='nodes'>
            {nodeLayer}
          </div>
          <div className={styles.worldOverlay} data-canvas-layer='world-overlay'>
            {worldOverlay}
          </div>
          {selectionStyle ? (
            <div
              className={styles.selection}
              style={selectionStyle}
              data-canvas-layer='selection'
              aria-hidden='true'
            />
          ) : null}
        </div>

        {screenOverlay ? (
          <div className={styles.screenOverlay} data-canvas-layer='screen-overlay'>
            {screenOverlay}
          </div>
        ) : null}

        <div
          className={styles.chrome}
          data-canvas-chrome
          onPointerDown={stopChromeEvent}
          onPointerMove={stopChromeEvent}
          onPointerUp={stopChromeEvent}
          onDoubleClick={stopChromeEvent}
          onWheel={stopChromeEvent}
          onContextMenu={stopChromeEvent}
        >
          {topDock ? (
            <div className={classNames(styles.dock, styles.topDock)} data-canvas-no-zoom data-canvas-dock='top'>
              {topDock}
            </div>
          ) : null}
          {leftDock ? (
            <div className={classNames(styles.dock, styles.leftDock)} data-canvas-no-zoom data-canvas-dock='left'>
              {leftDock}
            </div>
          ) : null}
          {rightDock ? (
            <div className={classNames(styles.dock, styles.rightDock)} data-canvas-no-zoom data-canvas-dock='right'>
              {rightDock}
            </div>
          ) : null}
          {bottomDock ? (
            <div className={classNames(styles.dock, styles.bottomDock)} data-canvas-no-zoom data-canvas-dock='bottom'>
              {bottomDock}
            </div>
          ) : null}

          {shouldRenderZoomControls ? (
            <div className={styles.zoomDock}>
              <CanvasZoomControls
                {...(zoomControls ?? {})}
                zoom={safeZoom}
                isMiniMapOpen={isMiniMapOpen}
              />
            </div>
          ) : null}

          {isMiniMapOpen && miniMap ? (
            <div className={styles.miniMapDock}>
              <CanvasMiniMapFrame label={miniMapLabel}>{miniMap}</CanvasMiniMapFrame>
            </div>
          ) : null}
        </div>
      </div>
    );
  }
);

CanvasSurface.displayName = 'CanvasSurface';

export default CanvasSurface;
