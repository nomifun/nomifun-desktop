import { describe, expect, test } from 'bun:test';
import { COMPANION_TOOLTIP_GUTTER, companionContentPosition, companionNativePosition, withCompanionTooltipGutters } from './companionWindowSize';
import { placeResizedWindow } from './windowGeometry';
import { resolveDeskRestoreLayout } from './deskRestoreGeometry';
import { CompanionClickThroughController } from './companionClickThroughController';

describe('stable companion tooltip space', () => {
  test('restoring and switching preserves saved content positions without accumulating gutter offsets', () => {
    for (const scale of [1, 1.25, 1.5, 2]) {
      const saved = { x: -230, y: 420 };
      const native = companionNativePosition(saved, scale);
      expect(companionContentPosition(native, scale)).toEqual(saved);
      const moved = { x: native.x + 30, y: native.y - 10 };
      const nextSaved = companionContentPosition(moved, scale);
      expect(nextSaved).toEqual({ x: saved.x + 30, y: saved.y - 10 });
      expect(companionNativePosition(nextSaved, scale)).toEqual(moved);
    }
  });
  test('keeps visible content centered and unchanged through desk/chat/restore at multiple DPI scales', () => {
    for (const scale of [1, 1.25, 1.5, 2]) {
      const desk = withCompanionTooltipGutters({ width: 200, height: 180 });
      const anchor = { x: 400 * scale, y: 600 * scale, width: desk.width * scale, height: desk.height * scale };
      const chat = withCompanionTooltipGutters({ width: 500, height: 640 });
      const size = { width: chat.width * scale, height: chat.height * scale };
      const position = placeResizedWindow(anchor, size, []);
      // Native positions are integral physical pixels, including at fractional DPI.
      expect(Math.abs(position.x + size.width / 2 - (anchor.x + anchor.width / 2))).toBeLessThanOrEqual(0.5);
      expect(position.y + size.height).toBe(anchor.y + anchor.height);
      expect(desk.width - COMPANION_TOOLTIP_GUTTER * 2).toBe(200);
      expect(chat.width - COMPANION_TOOLTIP_GUTTER * 2).toBe(500);
      const restored = resolveDeskRestoreLayout({ anchor, originalMonitorId: 'screen', monitors: [{
        id: 'screen', scaleFactor: scale,
        bounds: { x: 0, y: 0, width: 1920 * scale, height: 1080 * scale },
        workArea: { x: 0, y: 0, width: 1920 * scale, height: 1040 * scale },
      }], logicalDesk: desk });
      expect(restored.rect).toEqual(anchor);
    }
  });

  test('transparent tooltip gutters remain click-through during an open switcher', async () => {
    const viewport = withCompanionTooltipGutters({ width: 200, height: 180 });
    const ignored: boolean[] = [];
    let x = COMPANION_TOOLTIP_GUTTER + 12;
    const controller = new CompanionClickThroughController({
      sample: async () => ({ kind: 'point', backend: 'win32', xRatio: x / viewport.width, yRatio: 0.5 }),
      viewport: () => viewport,
      // A visible avatar column, not the entire native transparent rectangle.
      hitTest: (clientX) => clientX >= COMPANION_TOOLTIP_GUTTER && clientX <= COMPANION_TOOLTIP_GUTTER + 38,
      setIgnore: async (value) => { ignored.push(value); },
    });
    await controller.initialize();
    for (let index = 0; index < 20; index++) await controller.tick({ captureAll: false, dragging: false });
    expect(ignored).toEqual([false]);
    x = 20;
    await controller.tick({ captureAll: false, dragging: false });
    expect(ignored).toEqual([false, true]);
    x = COMPANION_TOOLTIP_GUTTER + 12;
    await controller.tick({ captureAll: false, dragging: false });
    expect(ignored).toEqual([false, true, false]);
    await controller.dispose();
  });
});
