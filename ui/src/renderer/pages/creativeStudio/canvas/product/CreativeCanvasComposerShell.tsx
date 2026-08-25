/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import styles from './CreativeCanvasComposerShell.module.css';

export type CreativeCanvasComposerKind = 'image' | 'video' | 'audio';

export interface CreativeCanvasComposerShellProps {
  kind: CreativeCanvasComposerKind;
  nodeId: string;
  children: React.ReactNode;
  mode?: string;
  voiceProfile?: 'unsupported' | 'required' | 'optional';
}

interface CreativeCanvasComposerOverlayLayout {
  left: number;
  top: number;
  width: number;
}

const DEFAULT_OVERLAY_LAYOUT: CreativeCanvasComposerOverlayLayout = {
  left: 16,
  top: 16,
  width: 358,
};

const CreativeCanvasComposerShell: React.FC<
  CreativeCanvasComposerShellProps
> = ({
  kind,
  nodeId,
  children,
  mode,
  voiceProfile,
}) => {
  const positionerRef = useRef<HTMLDivElement>(null);
  const anchorRef = useRef<HTMLSpanElement>(null);
  const horizontalOffsetRef = useRef(0);
  const [placement, setPlacement] = useState<'above' | 'below'>('below');
  const [horizontalOffset, setHorizontalOffset] = useState(0);
  const [overlay, setOverlay] = useState(false);
  const [overlayLayout, setOverlayLayout] = useState(
    DEFAULT_OVERLAY_LAYOUT
  );

  useLayoutEffect(() => {
    const positioner = positionerRef.current;
    const anchor = anchorRef.current;
    const surface = anchor?.closest<HTMLElement>('[data-canvas-surface]');
    const node = anchor?.closest<HTMLElement>('[data-canvas-node-id]');
    if (!positioner || !surface || !node) return;

    const updatePlacement = (): void => {
      const surfaceRect = surface.getBoundingClientRect();
      const nodeRect = node.getBoundingClientRect();
      const panelRect = positioner.getBoundingClientRect();
      const panelHeight = panelRect.height;
      const inset = 12;
      const gap = 16;
      const compactWidth = Math.min(580, Math.max(0, window.innerWidth - 32));
      const shouldOverlay = surfaceRect.width < compactWidth + inset * 2;
      const spaceBelow = surfaceRect.bottom - nodeRect.bottom - gap - inset;
      const spaceAbove = nodeRect.top - surfaceRect.top - gap - inset;
      const nextPlacement =
        panelHeight <= spaceBelow || spaceBelow >= spaceAbove ? 'below' : 'above';
      setPlacement((current) =>
        current === nextPlacement ? current : nextPlacement
      );

      setOverlay((current) =>
        current === shouldOverlay ? current : shouldOverlay
      );
      if (shouldOverlay) {
        const belowTop = nodeRect.bottom + gap;
        const aboveTop = nodeRect.top - panelHeight - gap;
        const preferredTop =
          belowTop + panelHeight <= window.innerHeight - inset
            ? belowTop
            : aboveTop >= inset
              ? aboveTop
              : Math.max(
                  inset,
                  Math.min(
                    window.innerHeight - panelHeight - inset,
                    nodeRect.top
                  )
                );
        const nextLayout = {
          left: Math.max(16, (window.innerWidth - compactWidth) / 2),
          top: preferredTop,
          width: compactWidth,
        };
        setOverlayLayout((current) =>
          Math.abs(current.left - nextLayout.left) < 0.5 &&
          Math.abs(current.top - nextLayout.top) < 0.5 &&
          Math.abs(current.width - nextLayout.width) < 0.5
            ? current
            : nextLayout
        );
        if (horizontalOffsetRef.current !== 0) {
          horizontalOffsetRef.current = 0;
          setHorizontalOffset(0);
        }
        return;
      }

      const naturalLeft = panelRect.left - horizontalOffsetRef.current;
      const naturalRight = panelRect.right - horizontalOffsetRef.current;
      const surfaceCanContainPanel =
        surfaceRect.width >= panelRect.width + inset * 2;
      const minimumLeft = (surfaceCanContainPanel ? surfaceRect.left : 0) + inset;
      const maximumRight =
        (surfaceCanContainPanel ? surfaceRect.right : window.innerWidth) - inset;
      const desiredOffset =
        panelRect.width > maximumRight - minimumLeft
          ? (minimumLeft + maximumRight) / 2 -
            (naturalLeft + naturalRight) / 2
          : naturalLeft < minimumLeft
            ? minimumLeft - naturalLeft
            : naturalRight > maximumRight
              ? maximumRight - naturalRight
              : 0;
      if (Math.abs(horizontalOffsetRef.current - desiredOffset) >= 0.5) {
        horizontalOffsetRef.current = desiredOffset;
        setHorizontalOffset(desiredOffset);
      }
    };

    updatePlacement();
    const observer =
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver(updatePlacement);
    observer?.observe(surface);
    observer?.observe(positioner);
    window.addEventListener('resize', updatePlacement);
    return () => {
      observer?.disconnect();
      window.removeEventListener('resize', updatePlacement);
    };
  }, [nodeId, overlay]);

  const content = (
    <div
      ref={positionerRef}
      className={styles.positioner}
      data-canvas-composer
      data-canvas-composer-shell
      data-canvas-composer-kind={kind}
      data-canvas-image-composer={kind === 'image' || undefined}
      data-canvas-video-composer={kind === 'video' || undefined}
      data-canvas-audio-composer={kind === 'audio' || undefined}
      data-overlay={overlay || undefined}
      data-placement={placement}
      data-mode={mode}
      data-node-id={nodeId}
      data-voice-profile={voiceProfile}
      style={
        {
          '--creative-canvas-composer-offset-x': `${horizontalOffset}px`,
          '--creative-canvas-composer-overlay-left': `${overlayLayout.left}px`,
          '--creative-canvas-composer-overlay-top': `${overlayLayout.top}px`,
          '--creative-canvas-composer-overlay-width': `${overlayLayout.width}px`,
        } as React.CSSProperties
      }
      onMouseDown={(event) => event.stopPropagation()}
      onPointerDown={(event) => event.stopPropagation()}
      onDoubleClick={(event) => event.stopPropagation()}
      onWheel={(event) => event.stopPropagation()}
    >
      <div className={styles.panel}>{children}</div>
    </div>
  );

  return (
    <>
      <span
        ref={anchorRef}
        hidden
        aria-hidden='true'
        data-canvas-composer-anchor
        data-canvas-image-composer-anchor={kind === 'image' || undefined}
        data-canvas-video-composer-anchor={kind === 'video' || undefined}
        data-canvas-audio-composer-anchor={kind === 'audio' || undefined}
        data-placement={placement}
      />
      {overlay && typeof document !== 'undefined'
        ? createPortal(content, document.body)
        : content}
    </>
  );
};

export default CreativeCanvasComposerShell;
