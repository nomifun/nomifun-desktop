/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { GeomRect, GeomSize } from './windowGeometry';

export type MemoryPanelPlacement = 'above' | 'left' | 'right';

export interface MemoryPanelLayoutInput {
  anchor: GeomRect;
  monitor: GeomRect;
  scaleFactor: number;
  desiredPanel: GeomSize;
  bottomChrome?: number;
}

export interface MemoryPanelLayout {
  placement: MemoryPanelPlacement;
  windowRect: GeomRect;
  panelMaxWidth: number;
  panelMaxHeight: number;
  gap: number;
  anchorOffset: { x: number; y: number };
}

export interface MonitorLayout {
  id: string;
  bounds: GeomRect;
  workArea: GeomRect;
  scaleFactor: number;
}

export interface DeskRestoreLayoutInput {
  anchor: GeomRect;
  originalMonitorId: string | null;
  monitors: MonitorLayout[];
  logicalDesk: GeomSize;
}

export interface DeskRestoreLayout {
  rect: GeomRect;
  monitorId: string | null;
  scaleFactor: number;
}

export interface AchievedMemoryPanelInput {
  achieved: GeomSize;
  anchor: GeomRect;
  monitor: GeomRect;
  gap: number;
  desiredWidth: number;
  desiredHeight: number;
  bottomChrome?: number;
  preferredPlacement?: MemoryPanelPlacement;
  minWidth?: number;
  minHeight?: number;
}

const overlapArea = (a: GeomRect, b: GeomRect): number => {
  const width = Math.max(0, Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x));
  const height = Math.max(0, Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y));
  return width * height;
};

const clamp = (value: number, min: number, max: number): number => Math.min(Math.max(value, min), Math.max(min, max));

export function pickHostMonitor(anchor: GeomRect, monitors: GeomRect[]): GeomRect | null {
  if (monitors.length === 0) return null;
  let best = monitors[0];
  let bestArea = overlapArea(anchor, best);
  for (const monitor of monitors.slice(1)) {
    const area = overlapArea(anchor, monitor);
    if (area > bestArea) {
      best = monitor;
      bestArea = area;
    }
  }
  return best;
}

export function memoryPanelStageShiftX(layout: MemoryPanelLayout, anchorWidth: number): number {
  const sideSpan = layout.panelMaxWidth + layout.gap;
  const naturalCenter =
    layout.windowRect.width / 2 +
    (layout.placement === 'left' ? sideSpan / 2 : layout.placement === 'right' ? -sideSpan / 2 : 0);
  const anchoredCenter = layout.anchorOffset.x + anchorWidth / 2;
  return anchoredCenter - naturalCenter;
}

export function resolveDeskRestoreLayout(input: DeskRestoreLayoutInput): DeskRestoreLayout {
  const original = input.originalMonitorId
    ? input.monitors.find((monitor) => monitor.id === input.originalMonitorId)
    : null;
  if (original) {
    return { rect: input.anchor, monitorId: original.id, scaleFactor: original.scaleFactor };
  }

  const hostBounds = pickHostMonitor(
    input.anchor,
    input.monitors.map((monitor) => monitor.bounds)
  );
  const host = hostBounds
    ? input.monitors.find(
        (monitor) =>
          monitor.bounds.x === hostBounds.x &&
          monitor.bounds.y === hostBounds.y &&
          monitor.bounds.width === hostBounds.width &&
          monitor.bounds.height === hostBounds.height
      )
    : null;
  if (!host) {
    return { rect: input.anchor, monitorId: null, scaleFactor: 1 };
  }

  const scale = Number.isFinite(host.scaleFactor) && host.scaleFactor > 0 ? host.scaleFactor : 1;
  const width = Math.min(host.workArea.width, Math.max(1, Math.round(input.logicalDesk.width * scale)));
  const height = Math.min(host.workArea.height, Math.max(1, Math.round(input.logicalDesk.height * scale)));
  const rawX = input.anchor.x + Math.round((input.anchor.width - width) / 2);
  const rawY = input.anchor.y + input.anchor.height - height;
  const x = clamp(rawX, host.workArea.x, host.workArea.x + host.workArea.width - width);
  const y = clamp(rawY, host.workArea.y, host.workArea.y + host.workArea.height - height);
  return {
    rect: { x, y, width, height },
    monitorId: host.id,
    scaleFactor: scale,
  };
}

export function fitMemoryPanelInAchievedWindow(input: AchievedMemoryPanelInput): MemoryPanelLayout | null {
  const { achieved, anchor, monitor } = input;
  const minWidth = input.minWidth ?? 220;
  const minHeight = input.minHeight ?? 96;
  const bottomChrome = Math.max(0, input.bottomChrome ?? 64);
  const rawY = anchor.y + anchor.height - achieved.height;

  // A side layout has no vertical stage correction. Reject it if bottom
  // anchoring the partially resized window would move the companion.
  const sideCanPreserveAnchor =
    achieved.height <= monitor.height && rawY >= monitor.y && rawY + achieved.height <= monitor.y + monitor.height;
  const sideWidth = Math.max(0, achieved.width - anchor.width - input.gap);
  const sideHeight = Math.max(0, Math.min(input.desiredHeight, achieved.height - bottomChrome));
  const availableLeft = Math.max(0, anchor.x - monitor.x - input.gap);
  const availableRight = Math.max(0, monitor.x + monitor.width - (anchor.x + anchor.width) - input.gap);

  type Candidate = MemoryPanelLayout & { area: number };
  const candidates: Candidate[] = [];
  const addCandidate = (
    placement: MemoryPanelPlacement,
    panelMaxWidth: number,
    panelMaxHeight: number,
    rawX: number,
    candidateRawY: number
  ) => {
    if (panelMaxWidth < minWidth || panelMaxHeight < minHeight) return;
    if (achieved.width > monitor.width || achieved.height > monitor.height) return;
    const x = clamp(rawX, monitor.x, monitor.x + monitor.width - achieved.width);
    const y = clamp(candidateRawY, monitor.y, monitor.y + monitor.height - achieved.height);
    candidates.push({
      placement,
      windowRect: { x, y, width: achieved.width, height: achieved.height },
      panelMaxWidth,
      panelMaxHeight,
      gap: input.gap,
      anchorOffset: { x: anchor.x - x, y: anchor.y - y },
      area: panelMaxWidth * panelMaxHeight,
    });
  };

  const aboveWidth = Math.max(0, Math.min(input.desiredWidth, achieved.width, monitor.width));
  const aboveHeight = Math.max(0, Math.min(input.desiredHeight, achieved.height - anchor.height - input.gap));
  if (rawY >= monitor.y) {
    addCandidate(
      'above',
      aboveWidth,
      aboveHeight,
      anchor.x + Math.round((anchor.width - achieved.width) / 2),
      rawY
    );
  }

  if (sideCanPreserveAnchor) {
    const rightWidth = Math.min(input.desiredWidth, sideWidth, availableRight);
    if (anchor.x + achieved.width <= monitor.x + monitor.width) {
      addCandidate('right', rightWidth, sideHeight, anchor.x, rawY);
    }

    const leftWidth = Math.min(input.desiredWidth, sideWidth, availableLeft);
    const leftX = anchor.x + anchor.width - achieved.width;
    if (leftX >= monitor.x) {
      addCandidate('left', leftWidth, sideHeight, leftX, rawY);
    }
  }

  if (candidates.length === 0) return null;
  const preferred = input.preferredPlacement
    ? candidates.find((candidate) => candidate.placement === input.preferredPlacement)
    : null;
  const selected = preferred ?? candidates.sort((a, b) => b.area - a.area)[0];
  const { area: _area, ...layout } = selected;
  return layout;
}

export function chooseMemoryPanelLayout(input: MemoryPanelLayoutInput): MemoryPanelLayout {
  const { anchor, monitor } = input;
  const scale = Number.isFinite(input.scaleFactor) && input.scaleFactor > 0 ? input.scaleFactor : 1;
  const gap = Math.round(12 * scale);
  const desiredWidth = Math.max(1, Math.round(input.desiredPanel.width * scale));
  const desiredHeight = Math.max(1, Math.round(input.desiredPanel.height * scale));
  const bottomChrome = Math.max(0, Math.round((input.bottomChrome ?? 64) * scale));

  const availableAbove = Math.max(0, anchor.y - monitor.y - gap);
  const availableLeft = Math.max(0, anchor.x - monitor.x - gap);
  const availableRight = Math.max(0, monitor.x + monitor.width - (anchor.x + anchor.width) - gap);
  const availableSideHeight = Math.max(1, anchor.y + anchor.height - monitor.y - bottomChrome);

  let placement: MemoryPanelPlacement;
  if (availableAbove >= desiredHeight) {
    placement = 'above';
  } else if (Math.max(availableLeft, availableRight) >= desiredWidth) {
    placement = availableRight >= availableLeft ? 'right' : 'left';
  } else {
    const aboveArea = availableAbove * Math.min(desiredWidth, monitor.width);
    const sideHeight = Math.min(desiredHeight, availableSideHeight);
    const leftArea = availableLeft * sideHeight;
    const rightArea = availableRight * sideHeight;
    placement = aboveArea >= Math.max(leftArea, rightArea) ? 'above' : availableRight >= availableLeft ? 'right' : 'left';
  }

  const availableWidth =
    placement === 'above' ? monitor.width : placement === 'left' ? availableLeft : availableRight;
  const panelMaxWidth = Math.max(1, Math.min(desiredWidth, availableWidth));
  const panelMaxHeight = Math.max(
    1,
    Math.min(desiredHeight, placement === 'above' ? availableAbove : availableSideHeight)
  );

  const width =
    placement === 'above'
      ? Math.min(monitor.width, Math.max(anchor.width, panelMaxWidth))
      : Math.min(monitor.width, anchor.width + gap + panelMaxWidth);
  const height =
    placement === 'above'
      ? Math.min(monitor.height, anchor.height + gap + panelMaxHeight)
      : Math.min(monitor.height, Math.max(anchor.height, panelMaxHeight + bottomChrome));

  const rawX =
    placement === 'above'
      ? anchor.x + Math.round((anchor.width - width) / 2)
      : placement === 'left'
        ? anchor.x - gap - panelMaxWidth
        : anchor.x;
  const rawY = placement === 'above' ? anchor.y - gap - panelMaxHeight : anchor.y + anchor.height - height;
  const x = clamp(rawX, monitor.x, monitor.x + monitor.width - width);
  const y = clamp(rawY, monitor.y, monitor.y + monitor.height - height);

  return {
    placement,
    windowRect: { x, y, width, height },
    panelMaxWidth,
    panelMaxHeight,
    gap,
    anchorOffset: { x: anchor.x - x, y: anchor.y - y },
  };
}
