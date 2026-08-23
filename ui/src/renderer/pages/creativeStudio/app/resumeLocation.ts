/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  CREATIVE_STUDIO_CANVASES_PATH,
  CREATIVE_STUDIO_LEGACY_PROJECTS_PATH,
  CREATIVE_STUDIO_ROOT_PATH,
  creativeStudioSectionForPath,
  isCreativeStudioPath,
} from './routes';

export const CREATIVE_STUDIO_RESUME_LOCATION_KEY =
  'nomifun:creative-studio:resume-location';
export const CREATIVE_STUDIO_CANVASES_RESUME_LOCATION_KEY =
  'nomifun:creative-studio:canvases-resume-location';

type ResumeStorage = Pick<Storage, 'getItem' | 'setItem'>;

const RESUME_LOCATION_BASE = new URL('https://nomifun.invalid/');
const MAX_RESUME_LOCATION_LENGTH = 4096;

const browserSessionStorage = (): ResumeStorage | null => {
  if (typeof window === 'undefined') return null;
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
};

const parseResumeLocation = (value: unknown): URL | null => {
  if (
    typeof value !== 'string' ||
    value.length === 0 ||
    value.length > MAX_RESUME_LOCATION_LENGTH ||
    !value.startsWith('/') ||
    value.startsWith('//')
  ) {
    return null;
  }

  try {
    const parsed = new URL(value, RESUME_LOCATION_BASE);
    return parsed.origin === RESUME_LOCATION_BASE.origin ? parsed : null;
  } catch {
    return null;
  }
};

const serializeResumeLocation = (
  parsed: URL,
  pathname = parsed.pathname
): string => `${pathname}${parsed.search}${parsed.hash}`;

/**
 * Accept only same-app Creative Studio paths. A corrupt, absolute, stale, or
 * near-prefix value must never turn the primary rail entry into an open redirect.
 */
export function normalizeCreativeStudioResumeLocation(value: unknown): string {
  const parsed = parseResumeLocation(value);
  if (!parsed || !isCreativeStudioPath(parsed.pathname)) {
    return CREATIVE_STUDIO_CANVASES_PATH;
  }
  const pathname = parsed.pathname.replace(/\/+$/, '') || '/';
  return serializeResumeLocation(
    parsed,
    pathname === CREATIVE_STUDIO_ROOT_PATH
      ? CREATIVE_STUDIO_CANVASES_PATH
      : pathname
  );
}

/**
 * Resolve only locations owned by the My Canvases section. The list, Canvas
 * editor, and Director are one resumable section; sibling workbenches must not
 * replace its last detail location.
 */
export function normalizeCreativeStudioCanvasesResumeLocation(
  value: unknown
): string | null {
  const parsed = parseResumeLocation(value);
  if (!parsed) return null;

  const section = creativeStudioSectionForPath(parsed.pathname);
  if (
    section !== 'canvases' &&
    section !== 'canvas' &&
    section !== 'director'
  ) {
    return null;
  }

  const pathname = parsed.pathname.replace(/\/+$/, '') || '/';
  return serializeResumeLocation(
    parsed,
    pathname === CREATIVE_STUDIO_ROOT_PATH ||
      pathname === CREATIVE_STUDIO_LEGACY_PROJECTS_PATH
      ? CREATIVE_STUDIO_CANVASES_PATH
      : pathname
  );
}

export function readCreativeStudioResumeLocation(
  storage: ResumeStorage | null = browserSessionStorage()
): string {
  if (!storage) return CREATIVE_STUDIO_CANVASES_PATH;
  try {
    return normalizeCreativeStudioResumeLocation(
      storage.getItem(CREATIVE_STUDIO_RESUME_LOCATION_KEY)
    );
  } catch {
    return CREATIVE_STUDIO_CANVASES_PATH;
  }
}

export function readCreativeStudioCanvasesResumeLocation(
  storage: ResumeStorage | null = browserSessionStorage()
): string {
  if (!storage) return CREATIVE_STUDIO_CANVASES_PATH;
  try {
    return (
      normalizeCreativeStudioCanvasesResumeLocation(
        storage.getItem(CREATIVE_STUDIO_CANVASES_RESUME_LOCATION_KEY)
      ) ??
      // Upgrade existing sessions that only have the original product-wide key.
      normalizeCreativeStudioCanvasesResumeLocation(
        storage.getItem(CREATIVE_STUDIO_RESUME_LOCATION_KEY)
      ) ??
      CREATIVE_STUDIO_CANVASES_PATH
    );
  } catch {
    return CREATIVE_STUDIO_CANVASES_PATH;
  }
}

export function rememberCreativeStudioResumeLocation(
  value: string,
  storage: ResumeStorage | null = browserSessionStorage()
): string {
  const normalized = normalizeCreativeStudioResumeLocation(value);
  const canvasesLocation = normalizeCreativeStudioCanvasesResumeLocation(value);
  if (!storage) return normalized;
  try {
    storage.setItem(CREATIVE_STUDIO_RESUME_LOCATION_KEY, normalized);
  } catch {
    // Session storage may be unavailable in embedded or privacy-restricted hosts.
  }
  if (canvasesLocation) {
    try {
      storage.setItem(
        CREATIVE_STUDIO_CANVASES_RESUME_LOCATION_KEY,
        canvasesLocation
      );
    } catch {
      // Keep the in-memory caller usable even when persistence is unavailable.
    }
  }
  return normalized;
}
