/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, it } from 'vitest';
import {
  chooseMemoryPanelLayout,
  fitMemoryPanelInAchievedWindow,
  memoryPanelStageShiftX,
  pickHostMonitor,
  resolveDeskRestoreLayout,
  type MonitorLayout,
} from './memoryPanelGeometry';

const MONITOR = { x: 0, y: 0, width: 1920, height: 1080 };

describe('chooseMemoryPanelLayout', () => {
  it('places the panel above with a 12px gap when headroom is available', () => {
    const layout = chooseMemoryPanelLayout({
      anchor: { x: 800, y: 700, width: 240, height: 214 },
      monitor: MONITOR,
      scaleFactor: 1,
      desiredPanel: { width: 340, height: 300 },
    });

    expect(layout.placement).toBe('above');
    expect(layout.gap).toBe(12);
    expect(layout.windowRect.y + layout.panelMaxHeight + layout.gap).toBe(700);
    expect(layout.anchorOffset.y).toBe(layout.panelMaxHeight + layout.gap);
  });

  it('flips right near the top when the right side has more room', () => {
    const layout = chooseMemoryPanelLayout({
      anchor: { x: 120, y: 10, width: 240, height: 214 },
      monitor: MONITOR,
      scaleFactor: 1,
      desiredPanel: { width: 340, height: 300 },
    });

    expect(layout.placement).toBe('right');
    expect(layout.windowRect.x).toBe(120);
    expect(layout.anchorOffset.x).toBe(0);
    expect(layout.panelMaxHeight).toBe(160);
  });

  it('flips left when the companion is against the right screen edge', () => {
    const layout = chooseMemoryPanelLayout({
      anchor: { x: 1660, y: 20, width: 240, height: 214 },
      monitor: MONITOR,
      scaleFactor: 1,
      desiredPanel: { width: 340, height: 300 },
    });

    expect(layout.placement).toBe('left');
    expect(layout.windowRect.x + layout.windowRect.width).toBe(1900);
    expect(layout.anchorOffset.x).toBe(layout.panelMaxWidth + layout.gap);
  });

  it('uses physical gap and dimensions at 150 percent scale', () => {
    const layout = chooseMemoryPanelLayout({
      anchor: { x: 900, y: 900, width: 360, height: 321 },
      monitor: { x: 0, y: 0, width: 2880, height: 1800 },
      scaleFactor: 1.5,
      desiredPanel: { width: 340, height: 300 },
    });

    expect(layout.gap).toBe(18);
    expect(layout.panelMaxWidth).toBe(510);
    expect(layout.panelMaxHeight).toBe(450);
  });

  for (const { scaleFactor, expectedGap, expectedWidth } of [
    { scaleFactor: 1.25, expectedGap: 15, expectedWidth: 425 },
    { scaleFactor: 2, expectedGap: 24, expectedWidth: 680 },
  ] as const) {
    it(`keeps logical spacing stable at ${scaleFactor} scale`, () => {
      const layout = chooseMemoryPanelLayout({
        anchor: { x: 1000, y: 1200, width: 240 * scaleFactor, height: 214 * scaleFactor },
        monitor: { x: 0, y: 0, width: 1920 * scaleFactor, height: 1080 * scaleFactor },
        scaleFactor,
        desiredPanel: { width: 340, height: 300 },
      });

      expect(layout.gap).toBe(expectedGap);
      expect(layout.panelMaxWidth).toBe(expectedWidth);
    });
  }

  it('keeps a large custom companion and panel disjoint', () => {
    const anchor = { x: 700, y: 500, width: 400, height: 464 };
    const layout = chooseMemoryPanelLayout({
      anchor,
      monitor: MONITOR,
      scaleFactor: 1,
      desiredPanel: { width: 360, height: 320 },
    });

    expect(layout.placement).toBe('above');
    const panelBottom = layout.windowRect.y + layout.panelMaxHeight;
    expect(panelBottom + layout.gap).toBe(anchor.y);
  });

  it('caps panel width to the available side and keeps the window on-screen', () => {
    const narrow = { x: 0, y: 0, width: 720, height: 600 };
    const layout = chooseMemoryPanelLayout({
      anchor: { x: 260, y: 10, width: 240, height: 214 },
      monitor: narrow,
      scaleFactor: 1,
      desiredPanel: { width: 340, height: 300 },
    });

    expect(layout.windowRect.x).toBeGreaterThanOrEqual(narrow.x);
    expect(layout.windowRect.x + layout.windowRect.width).toBeLessThanOrEqual(narrow.x + narrow.width);
    expect(layout.windowRect.y).toBeGreaterThanOrEqual(narrow.y);
    expect(layout.windowRect.y + layout.windowRect.height).toBeLessThanOrEqual(narrow.y + narrow.height);
  });

  it('keeps the stage screen center fixed when an above panel is clamped at the left edge', () => {
    const anchor = { x: 0, y: 700, width: 240, height: 214 };
    const layout = chooseMemoryPanelLayout({
      anchor,
      monitor: MONITOR,
      scaleFactor: 1,
      desiredPanel: { width: 340, height: 300 },
    });

    expect(layout.placement).toBe('above');
    expect(layout.windowRect.x).toBe(0);
    expect(memoryPanelStageShiftX(layout, anchor.width)).toBe(-50);
    const renderedStageCenter =
      layout.windowRect.x + layout.windowRect.width / 2 + memoryPanelStageShiftX(layout, anchor.width);
    expect(renderedStageCenter).toBe(anchor.x + anchor.width / 2);
  });

  it('clamps against an inset work area rather than the full monitor bounds', () => {
    const workArea = { x: 0, y: 48, width: 1920, height: 984 };
    const layout = chooseMemoryPanelLayout({
      anchor: { x: 820, y: 50, width: 240, height: 214 },
      monitor: workArea,
      scaleFactor: 1,
      desiredPanel: { width: 340, height: 300 },
    });

    expect(layout.windowRect.y).toBeGreaterThanOrEqual(workArea.y);
    expect(layout.windowRect.y + layout.windowRect.height).toBeLessThanOrEqual(workArea.y + workArea.height);
  });
});

describe('pickHostMonitor', () => {
  it('picks the monitor with the largest overlap, including negative coordinates', () => {
    const left = { x: -1920, y: 0, width: 1920, height: 1080 };

    expect(pickHostMonitor({ x: -400, y: 600, width: 240, height: 214 }, [MONITOR, left])).toEqual(left);
  });

  it('falls back to the first monitor when the anchor overlaps none', () => {
    expect(pickHostMonitor({ x: 5000, y: 5000, width: 240, height: 214 }, [MONITOR])).toEqual(MONITOR);
  });

  it('returns null when no monitor is available', () => {
    expect(pickHostMonitor({ x: 0, y: 0, width: 240, height: 214 }, [])).toBeNull();
  });
});

describe('resolveDeskRestoreLayout', () => {
  const original: MonitorLayout = {
    id: 'external',
    bounds: { x: 1920, y: 0, width: 1920, height: 1080 },
    workArea: { x: 1920, y: 24, width: 1920, height: 1016 },
    scaleFactor: 1,
  };

  it('restores the exact captured rectangle while the original monitor still exists', () => {
    const anchor = { x: 2500, y: 700, width: 240, height: 214 };
    const restored = resolveDeskRestoreLayout({
      anchor,
      originalMonitorId: original.id,
      monitors: [original],
      logicalDesk: { width: 240, height: 214 },
    });

    expect(restored.rect).toEqual(anchor);
    expect(restored.scaleFactor).toBe(1);
  });

  it('moves into a remaining monitor work area and adopts its scale when the original monitor is removed', () => {
    const remaining: MonitorLayout = {
      id: 'retina-main',
      bounds: { x: 0, y: 0, width: 1800, height: 1168 },
      workArea: { x: 0, y: 48, width: 1800, height: 1080 },
      scaleFactor: 2,
    };
    const restored = resolveDeskRestoreLayout({
      anchor: { x: 2500, y: 700, width: 240, height: 214 },
      originalMonitorId: original.id,
      monitors: [remaining],
      logicalDesk: { width: 240, height: 214 },
    });

    expect(restored.monitorId).toBe(remaining.id);
    expect(restored.scaleFactor).toBe(2);
    expect(restored.rect.width).toBe(480);
    expect(restored.rect.height).toBe(428);
    expect(restored.rect.x).toBeGreaterThanOrEqual(remaining.workArea.x);
    expect(restored.rect.x + restored.rect.width).toBeLessThanOrEqual(
      remaining.workArea.x + remaining.workArea.width
    );
  });
});

describe('fitMemoryPanelInAchievedWindow', () => {
  it('uses a side viewport when the native window only grants horizontal expansion', () => {
    expect(
      fitMemoryPanelInAchievedWindow({
        achieved: { width: 500, height: 214 },
        anchor: { x: 120, y: 10, width: 240, height: 214 },
        monitor: MONITOR,
        gap: 12,
        desiredWidth: 340,
        desiredHeight: 300,
      })
    ).toMatchObject({
      placement: 'right',
      panelMaxWidth: 248,
      panelMaxHeight: 150,
      windowRect: { x: 120, y: 10, width: 500, height: 214 },
      anchorOffset: { x: 0, y: 0 },
    });
  });

  it('mirrors the horizontal-only fallback to the left at the right screen edge', () => {
    expect(
      fitMemoryPanelInAchievedWindow({
        achieved: { width: 500, height: 214 },
        anchor: { x: 1660, y: 20, width: 240, height: 214 },
        monitor: MONITOR,
        gap: 12,
        desiredWidth: 340,
        desiredHeight: 300,
      })
    ).toMatchObject({
      placement: 'left',
      panelMaxWidth: 248,
      panelMaxHeight: 150,
      windowRect: { x: 1400, y: 20, width: 500, height: 214 },
      anchorOffset: { x: 260, y: 0 },
    });
  });

  it('keeps a reduced scroll viewport when a partial native resize leaves readable space', () => {
    expect(
      fitMemoryPanelInAchievedWindow({
        achieved: { width: 360, height: 400 },
        anchor: { x: 800, y: 700, width: 240, height: 214 },
        monitor: MONITOR,
        gap: 12,
        desiredWidth: 340,
        desiredHeight: 300,
      })
    ).toMatchObject({ placement: 'above', panelMaxWidth: 340, panelMaxHeight: 174 });
  });

  it('returns null when the achieved desk window cannot fit a safe panel', () => {
    expect(
      fitMemoryPanelInAchievedWindow({
        achieved: { width: 240, height: 214 },
        anchor: { x: 800, y: 700, width: 240, height: 214 },
        monitor: MONITOR,
        gap: 12,
        desiredWidth: 340,
        desiredHeight: 300,
      })
    ).toBeNull();
  });
});
