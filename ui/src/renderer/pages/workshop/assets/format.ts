/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Small display formatters shared across the asset-library surfaces (M4).
 * Pure, dependency-free — safe to import from any asset component.
 */

/** Human-readable byte size (e.g. `1.4 MB`). Returns `—` for null/0. */
export { formatBytes } from '@/renderer/utils/file/formatBytes';

/** `1024 × 768`, or null when dimensions are unknown. */
export function formatDimensions(
  width: number | null | undefined,
  height: number | null | undefined
): string | null {
  if (!width || !height) return null;
  return `${width} × ${height}`;
}

/** `MM:SS` (or `H:MM:SS`) from a seconds count; null when not a positive number. */
export function formatDuration(seconds: number | null | undefined): string | null {
  if (typeof seconds !== 'number' || !Number.isFinite(seconds) || seconds <= 0) return null;
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(h > 0 ? 2 : 1, '0');
  const ss = String(s).padStart(2, '0');
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/**
 * Best-effort duration (in seconds) recorded on a generated asset's origin
 * params (`seconds` / `duration`). Used only to render a corner badge on video
 * cards; returns null when nothing usable is present.
 */
export function originDurationSeconds(params: Record<string, unknown> | undefined): number | null {
  if (!params) return null;
  const candidate = params.seconds ?? params.duration;
  return typeof candidate === 'number' && Number.isFinite(candidate) ? candidate : null;
}
