import type { GeomPoint, GeomSize } from './windowGeometry';

/** Permanent transparent space: 160px name + 8px gap on either side.
 * Keep this independent of hover/focus; resizing under the pointer feeds native
 * enter/leave events back into tooltip visibility and can oscillate forever.
 * Only visible chrome participates in the native click-through hit test.
 */
export const COMPANION_TOOLTIP_GUTTER = 168;

export function withCompanionTooltipGutters(content: GeomSize): GeomSize {
  return { width: content.width + COMPANION_TOOLTIP_GUTTER * 2, height: content.height };
}

/** Persist the original content origin, not the new transparent window margin. */
export function companionContentPosition(native: GeomPoint, scale: number): GeomPoint {
  return { x: native.x + Math.round(COMPANION_TOOLTIP_GUTTER * scale), y: native.y };
}

export function companionNativePosition(content: GeomPoint, scale: number): GeomPoint {
  return { x: content.x - Math.round(COMPANION_TOOLTIP_GUTTER * scale), y: content.y };
}
