/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';

import type {
  CreativeCanvasConnection,
  CreativeCanvasNode,
} from '../../domain/schema';
import { buildCanvasConnectionBezier } from '../graph';
import styles from './CreativeCanvasConnectionEdge.module.css';

export interface CreativeCanvasConnectionEdgeProps {
  connection: CreativeCanvasConnection;
  source: CreativeCanvasNode;
  target: CreativeCanvasNode;
  selected: boolean;
  highlighted?: boolean;
  dimmed?: boolean;
  onActivate(): void;
  onContextMenu?: React.MouseEventHandler<SVGElement>;
  ariaLabel?: string;
}

/**
 * One world-space connection for CreativeCanvasEditor's per-edge render slot.
 * Endpoint geometry remains canonical; the transparent path only expands the
 * pointer target and never changes the visible connection.
 */
const CreativeCanvasConnectionEdge: React.FC<CreativeCanvasConnectionEdgeProps> = ({
  connection,
  source,
  target,
  selected,
  highlighted = false,
  dimmed = false,
  onActivate,
  onContextMenu,
  ariaLabel = `连接 ${source.id} 至 ${target.id}`,
}) => {
  const geometry = buildCanvasConnectionBezier(connection, source, target);

  const activate = (
    event: React.MouseEvent<SVGPathElement> | React.KeyboardEvent<SVGPathElement>
  ) => {
    event.stopPropagation();
    onActivate();
  };

  return (
    <svg
      className={styles.layer}
      role='group'
      aria-label={ariaLabel}
      data-canvas-product-edge
      data-connection-id={connection.id}
      data-edge-selected={selected || undefined}
      data-edge-highlighted={highlighted || undefined}
      data-edge-dimmed={dimmed || undefined}
      onContextMenu={onContextMenu}
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
        role='button'
        tabIndex={0}
        aria-label={ariaLabel}
        aria-pressed={selected}
        onPointerDown={(event) => event.stopPropagation()}
        onClick={activate}
        onKeyDown={(event) => {
          if (event.key !== 'Enter' && event.key !== ' ') return;
          event.preventDefault();
          activate(event);
        }}
      />
    </svg>
  );
};

export default CreativeCanvasConnectionEdge;
