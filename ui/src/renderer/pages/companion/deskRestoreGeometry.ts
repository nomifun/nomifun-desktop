import { clamp, pickHost, type GeomRect, type GeomSize } from './windowGeometry';

export interface MonitorLayout { id: string; bounds: GeomRect; workArea: GeomRect; scaleFactor: number }
export interface DeskRestoreLayoutInput { anchor: GeomRect; originalMonitorId: string | null; monitors: MonitorLayout[]; logicalDesk: GeomSize }
export interface DeskRestoreLayout { rect: GeomRect; monitorId: string | null; scaleFactor: number }

export function pickHostMonitor(anchor: GeomRect, monitors: GeomRect[]): GeomRect | null {
  return pickHost(anchor, monitors, (monitor) => monitor);
}

export function resolveDeskRestoreLayout(input: DeskRestoreLayoutInput): DeskRestoreLayout {
  const original = input.originalMonitorId ? input.monitors.find((monitor) => monitor.id === input.originalMonitorId) : null;
  if (original) return { rect: input.anchor, monitorId: original.id, scaleFactor: original.scaleFactor };
  const host = pickHost(input.anchor, input.monitors, (monitor) => monitor.bounds);
  if (!host) return { rect: input.anchor, monitorId: null, scaleFactor: 1 };
  const scale = Number.isFinite(host.scaleFactor) && host.scaleFactor > 0 ? host.scaleFactor : 1;
  const width = Math.min(host.workArea.width, Math.max(1, Math.round(input.logicalDesk.width * scale)));
  const height = Math.min(host.workArea.height, Math.max(1, Math.round(input.logicalDesk.height * scale)));
  const rawX = input.anchor.x + Math.round((input.anchor.width - width) / 2);
  const rawY = input.anchor.y + input.anchor.height - height;
  return { rect: { x: clamp(rawX, host.workArea.x, host.workArea.x + host.workArea.width - width), y: clamp(rawY, host.workArea.y, host.workArea.y + host.workArea.height - height), width, height }, monitorId: host.id, scaleFactor: scale };
}
