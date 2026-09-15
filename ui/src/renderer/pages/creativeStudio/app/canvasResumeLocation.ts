import { CANVASES_PATH, migratedCreativeRoute, resourceSectionForPath } from './resourceRoutes';
const KEY = 'nomifun:canvas:resume-location';
const LEGACY_KEYS = ['nomifun:creative-studio:canvases-resume-location', 'nomifun:creative-studio:resume-location'];
type ResumeStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
const browserStorage = (): ResumeStorage | null => {
  try { return typeof window === 'undefined' ? null : window.sessionStorage; } catch { return null; }
};
export function normalizeCanvasResumeLocation(value: unknown): string | null {
  if (typeof value !== 'string' || !value.startsWith('/') || value.startsWith('//') || value.length > 4096) return null;
  const parsed = new URL(value, 'https://nomifun.invalid');
  if (parsed.origin !== 'https://nomifun.invalid') return null;
  const section = resourceSectionForPath(parsed.pathname);
  if (section !== 'canvas' && section !== 'canvases') return null;
  return `${parsed.pathname.replace(/\/+$/, '')}${parsed.search}${parsed.hash}`;
}
export function readCanvasResumeLocation(storage: ResumeStorage | null = browserStorage()): string {
  try {
    const current = normalizeCanvasResumeLocation(storage?.getItem(KEY));
    if (current) return current;
    for (const key of LEGACY_KEYS) {
      const legacy = storage?.getItem(key);
      const migrated = legacy ? normalizeCanvasResumeLocation(migratedCreativeRoute(legacy)) : null;
      if (migrated && storage) {
        storage.setItem(KEY, migrated);
        for (const oldKey of LEGACY_KEYS) storage.removeItem(oldKey);
        return migrated;
      }
    }
  } catch { /* Storage can be unavailable in embedded hosts. */ }
  return CANVASES_PATH;
}
export function rememberCanvasResumeLocation(path: string, storage: ResumeStorage | null = browserStorage()): string | null {
  const normalized = normalizeCanvasResumeLocation(path);
  if (normalized) {
    try { storage?.setItem(KEY, normalized); } catch { /* Keep in-memory navigation usable. */ }
  }
  return normalized;
}
