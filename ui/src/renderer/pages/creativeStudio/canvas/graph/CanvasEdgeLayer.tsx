/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';

import type { CreativeCanvasConnection, CreativeCanvasNode } from '../../domain/schema';
import {
  buildCanvasConnectionBezier,
  type CanvasHandleGeometryByNode,
} from './geometry';
import styles from './CanvasEdgeLayer.module.css';

export interface CanvasEdgeVisualState {
  selected?: boolean;
  upstream?: boolean;
  downstream?: boolean;
  error?: boolean;
  highlighted?: boolean;
  dimmed?: boolean;
}

export interface CanvasEdgeLayerProps {
  nodes: readonly CreativeCanvasNode[];
  connections: readonly CreativeCanvasConnection[];
  handleGeometry?: CanvasHandleGeometryByNode;
  stateByConnectionId?: Readonly<Record<string, CanvasEdgeVisualState | undefined>>;
  ariaLabel?: string;
  getConnectionLabel?: (connection: CreativeCanvasConnection) => string;
  onSelectConnection?: (
    connection: CreativeCanvasConnection,
    event: React.MouseEvent<SVGPathElement> | React.KeyboardEvent<SVGPathElement>
  ) => void;
  onConnectionContextMenu?: (
    connection: CreativeCanvasConnection,
    event: React.MouseEvent<SVGPathElement>
  ) => void;
  onConnectionHoverChange?: (connection: CreativeCanvasConnection | null) => void;
}

/**
 * Controlled world-space edge renderer. Missing endpoints are omitted rather
 * than guessed, while handle IDs resolve through caller-supplied node-local
 * geometry and fall back to right-to-left node midpoints.
 */
const CanvasEdgeLayer: React.FC<CanvasEdgeLayerProps> = ({
  nodes,
  connections,
  handleGeometry,
  stateByConnectionId,
  ariaLabel = '画布连接',
  getConnectionLabel,
  onSelectConnection,
  onConnectionContextMenu,
  onConnectionHoverChange,
}) => {
  const nodeById = new Map(nodes.map((node) => [node.id, node]));

  return (
    <svg className={styles.layer} role='group' aria-label={ariaLabel} data-canvas-edge-layer>
      {connections.map((connection) => {
        const sourceNode = nodeById.get(connection.sourceNodeId);
        const targetNode = nodeById.get(connection.targetNodeId);
        if (!sourceNode || !targetNode) return null;
        const geometry = buildCanvasConnectionBezier(connection, sourceNode, targetNode, handleGeometry);
        const state = stateByConnectionId?.[connection.id] ?? {};
        const label =
          getConnectionLabel?.(connection) ??
          `${connection.sourceNodeId} → ${connection.targetNodeId}`;

        return (
          <g
            key={connection.id}
            className={styles.edge}
            data-connection-id={connection.id}
            data-edge-selected={state.selected || undefined}
            data-edge-upstream={state.upstream || undefined}
            data-edge-downstream={state.downstream || undefined}
            data-edge-error={state.error || undefined}
            data-edge-highlighted={state.highlighted || undefined}
            data-edge-dimmed={state.dimmed || undefined}
            data-edge-interactive={Boolean(
              onSelectConnection || onConnectionContextMenu || onConnectionHoverChange
            ) || undefined}
          >
            <path
              className={styles.visiblePath}
              d={geometry.path}
              fill='none'
              vectorEffect='non-scaling-stroke'
              aria-hidden='true'
            />
            <path
              className={styles.hitPath}
              d={geometry.path}
              fill='none'
              vectorEffect='non-scaling-stroke'
              role={onSelectConnection ? 'button' : undefined}
              tabIndex={onSelectConnection ? 0 : undefined}
              aria-label={label}
              onClick={(event) => {
                if (!onSelectConnection) return;
                event.stopPropagation();
                onSelectConnection(connection, event);
              }}
              onKeyDown={(event) => {
                if (!onSelectConnection || (event.key !== 'Enter' && event.key !== ' ')) return;
                event.preventDefault();
                event.stopPropagation();
                onSelectConnection(connection, event);
              }}
              onContextMenu={(event) => {
                if (!onConnectionContextMenu) return;
                event.preventDefault();
                event.stopPropagation();
                onConnectionContextMenu(connection, event);
              }}
              onPointerEnter={() => onConnectionHoverChange?.(connection)}
              onPointerLeave={() => onConnectionHoverChange?.(null)}
            />
          </g>
        );
      })}
    </svg>
  );
};

export default CanvasEdgeLayer;
