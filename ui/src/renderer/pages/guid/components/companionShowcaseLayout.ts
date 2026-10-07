/** Layout uses pane dimensions, never device or touch detection. */
export function showcaseCapacity(width: number): number {
  return width >= 640 ? 4 : width >= 460 ? 3 : 2;
}

export function visibleCompanions<T extends { companion_id: string }>(
  companions: T[], capacity: number, selectedId: string | null,
): T[] {
  const visible = companions.slice(0, capacity);
  const selected = companions.find((companion) => companion.companion_id === selectedId);
  if (selected && !visible.includes(selected)) visible[visible.length - 1] = selected;
  return visible;
}

/** Bound both axes without distorting landscape cutouts or cropping tall figures. */
export function fitShowcaseFigure(aspect: number, width: number, height: number): number {
  const safeAspect = Number.isFinite(aspect) && aspect > 0 ? aspect : 1;
  return Math.max(1, Math.min(height, Math.floor(Math.max(1, width) / safeAspect)));
}
