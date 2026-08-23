/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';

import type {
  CreativeCanvasNode,
  CreativePoint,
  CreativeSize,
  CreativeViewport,
} from '../../domain/schema';
import {
  centerCanvasViewportAt,
  createCanvasMiniMapProjection,
  miniMapPointToWorld,
  visibleWorldRect,
  worldRectToMiniMap,
  type CanvasMiniMapProjection,
} from './geometry';
import styles from './CanvasMiniMap.module.css';

export type CanvasMiniMapNavigationPhase = 'start' | 'move' | 'end';

export interface CanvasMiniMapNavigationRequest {
  phase: CanvasMiniMapNavigationPhase;
  worldCenter: CreativePoint;
  viewport: CreativeViewport;
}

export interface CanvasMiniMapProps {
  nodes: readonly CreativeCanvasNode[];
  viewport: CreativeViewport;
  viewportSize: CreativeSize;
  width?: number;
  height?: number;
  padding?: number;
  worldPadding?: number;
  dragging?: boolean;
  selectedNodeIds?: ReadonlySet<string>;
  ariaLabel?: string;
  onNavigate?: (
    request: CanvasMiniMapNavigationRequest,
    event: React.PointerEvent<SVGSVGElement>
  ) => void;
}

const eventPointInMiniMap = (
  event: React.PointerEvent<SVGSVGElement>,
  projection: CanvasMiniMapProjection
): CreativePoint | null => {
  const bounds = event.currentTarget.getBoundingClientRect();
  if (bounds.width <= 0 || bounds.height <= 0) return null;
  return {
    x: ((event.clientX - bounds.left) / bounds.width) * projection.width,
    y: ((event.clientY - bounds.top) / bounds.height) * projection.height,
  };
};

/**
 * Real canonical-node minimap. Pointer capture is used only to keep receiving
 * a gesture; dragging state and viewport authority remain in the caller.
 */
const CanvasMiniMap: React.FC<CanvasMiniMapProps> = ({
  nodes,
  viewport,
  viewportSize,
  width = 240,
  height = 160,
  padding = 10,
  worldPadding = 120,
  dragging = false,
  selectedNodeIds,
  ariaLabel,
  onNavigate,
}) => {
  const { t } = useTranslation();
  const projection = createCanvasMiniMapProjection(nodes, viewport, viewportSize, {
    width,
    height,
    padding,
    worldPadding,
  });
  const viewportRect = worldRectToMiniMap(visibleWorldRect(viewport, viewportSize), projection);
  const orderedNodes = [...nodes].sort((left, right) => left.zIndex - right.zIndex);

  const emitNavigation = (
    phase: CanvasMiniMapNavigationPhase,
    event: React.PointerEvent<SVGSVGElement>
  ) => {
    if (!onNavigate) return;
    const miniMapPoint = eventPointInMiniMap(event, projection);
    if (!miniMapPoint) return;
    const worldCenter = miniMapPointToWorld(miniMapPoint, projection);
    onNavigate(
      {
        phase,
        worldCenter,
        viewport: centerCanvasViewportAt(worldCenter, viewport, viewportSize),
      },
      event
    );
  };

  return (
    <svg
      className={styles.miniMap}
      viewBox={`0 0 ${projection.width} ${projection.height}`}
      preserveAspectRatio='none'
      role={onNavigate ? 'application' : 'img'}
      aria-label={ariaLabel ?? t('creativeStudio.canvas.minimap.label')}
      data-canvas-minimap-renderer
      data-minimap-dragging={dragging || undefined}
      onPointerDown={(event) => {
        if (!onNavigate || event.button !== 0) return;
        event.preventDefault();
        event.stopPropagation();
        event.currentTarget.setPointerCapture(event.pointerId);
        emitNavigation('start', event);
      }}
      onPointerMove={(event) => {
        if (!onNavigate || (!dragging && !event.currentTarget.hasPointerCapture(event.pointerId))) return;
        emitNavigation('move', event);
      }}
      onPointerUp={(event) => {
        if (!onNavigate || (!dragging && !event.currentTarget.hasPointerCapture(event.pointerId))) return;
        emitNavigation('end', event);
        if (event.currentTarget.hasPointerCapture(event.pointerId)) {
          event.currentTarget.releasePointerCapture(event.pointerId);
        }
      }}
      onPointerCancel={(event) => {
        if (!onNavigate || (!dragging && !event.currentTarget.hasPointerCapture(event.pointerId))) return;
        emitNavigation('end', event);
        if (event.currentTarget.hasPointerCapture(event.pointerId)) {
          event.currentTarget.releasePointerCapture(event.pointerId);
        }
      }}
    >
      <rect className={styles.background} x={0} y={0} width={projection.width} height={projection.height} />
      {orderedNodes.map((node) => {
        const rect = worldRectToMiniMap(
          {
            x: node.position.x,
            y: node.position.y,
            width: node.size.width,
            height: node.size.height,
          },
          projection
        );
        return (
          <rect
            key={node.id}
            className={styles.node}
            x={rect.x}
            y={rect.y}
            width={rect.width}
            height={rect.height}
            rx={node.type === 'group' ? 2 : 1.5}
            data-minimap-node-id={node.id}
            data-minimap-node-type={node.type}
            data-minimap-node-selected={selectedNodeIds?.has(node.id) || undefined}
            vectorEffect='non-scaling-stroke'
            aria-hidden='true'
          />
        );
      })}
      <rect
        className={styles.viewport}
        x={viewportRect.x}
        y={viewportRect.y}
        width={viewportRect.width}
        height={viewportRect.height}
        rx={2}
        vectorEffect='non-scaling-stroke'
        aria-hidden='true'
        data-minimap-viewport
      />
    </svg>
  );
};

export default CanvasMiniMap;
