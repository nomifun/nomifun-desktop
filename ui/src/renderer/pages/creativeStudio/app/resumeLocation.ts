/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CREATIVE_STUDIO_ROOT_PATH, isCreativeStudioPath } from './routes';

export const CREATIVE_STUDIO_RESUME_LOCATION_KEY =
  'nomifun:creative-studio:resume-location';

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

/**
 * Accept only same-app Creative Studio paths. A corrupt, absolute, stale, or
 * near-prefix value must never turn the primary rail entry into an open redirect.
 */
export function normalizeCreativeStudioResumeLocation(value: unknown): string {
  if (
    typeof value !== 'string' ||
    value.length === 0 ||
    value.length > MAX_RESUME_LOCATION_LENGTH ||
    !value.startsWith('/') ||
    value.startsWith('//')
  ) {
    return CREATIVE_STUDIO_ROOT_PATH;
  }

  try {
    const parsed = new URL(value, RESUME_LOCATION_BASE);
    if (
      parsed.origin !== RESUME_LOCATION_BASE.origin ||
      !isCreativeStudioPath(parsed.pathname)
    ) {
      return CREATIVE_STUDIO_ROOT_PATH;
    }
    return `${parsed.pathname}${parsed.search}${parsed.hash}`;
  } catch {
    return CREATIVE_STUDIO_ROOT_PATH;
  }
}

export function readCreativeStudioResumeLocation(
  storage: ResumeStorage | null = browserSessionStorage()
): string {
  if (!storage) return CREATIVE_STUDIO_ROOT_PATH;
  try {
    return normalizeCreativeStudioResumeLocation(
      storage.getItem(CREATIVE_STUDIO_RESUME_LOCATION_KEY)
    );
  } catch {
    return CREATIVE_STUDIO_ROOT_PATH;
  }
}

export function rememberCreativeStudioResumeLocation(
  value: string,
  storage: ResumeStorage | null = browserSessionStorage()
): string {
  const normalized = normalizeCreativeStudioResumeLocation(value);
  if (!storage) return normalized;
  try {
    storage.setItem(CREATIVE_STUDIO_RESUME_LOCATION_KEY, normalized);
  } catch {
    // Session storage may be unavailable in embedded or privacy-restricted hosts.
  }
  return normalized;
}
