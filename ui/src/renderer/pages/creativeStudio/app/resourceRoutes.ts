/** Destinations of the retained canvas and asset resources. */
export const CANVASES_PATH = '/nomi/canvases';
export const CANVAS_PATTERN = '/nomi/canvases/:canvasId';
export const ASSET_LIBRARY_PATH = '/asset-library';
export const MATERIALS_PATH = '/asset-library/materials';
export const PROMPTS_PATH = '/asset-library/prompts';
export const TEMPLATES_PATH = '/asset-library/templates';
export type ResourceSection = 'canvases' | 'canvas' | 'assets' | 'prompts' | 'templates';
const pathnameOf = (path: string): string =>
  (path.split(/[?#]/, 1)[0] || '/').replace(/\/+$/, '') || '/';
export const canvasPath = (canvasId: string): string => {
  if (!canvasId.trim()) throw new Error('Canvas id is required');
  return `${CANVASES_PATH}/${encodeURIComponent(canvasId.trim())}`;
};
export function matchCanvasPath(path: string): { canvasId: string } | null {
  const match = /^\/nomi\/canvases\/([^/]+)$/.exec(pathnameOf(path));
  if (!match) return null;
  try {
    const canvasId = decodeURIComponent(match[1]).trim();
    return canvasId ? { canvasId } : null;
  } catch { return null; }
}
export function resourceSectionForPath(path: string): ResourceSection | null {
  const pathname = pathnameOf(path);
  if (pathname === CANVASES_PATH) return 'canvases';
  if (matchCanvasPath(pathname)) return 'canvas';
  if (pathname === ASSET_LIBRARY_PATH || pathname === MATERIALS_PATH) return 'assets';
  if (pathname === PROMPTS_PATH) return 'prompts';
  if (pathname === TEMPLATES_PATH) return 'templates';
  return null;
}
/** One-way canvas resume import; this function is never mounted as a route. */
export function migratedCreativeRoute(path: string): string | null {
  if (!path.startsWith('/') || path.startsWith('//') || path.length > 4096) return null;
  const parsed = new URL(path, 'https://nomifun.invalid');
  if (parsed.origin !== 'https://nomifun.invalid') return null;
  const pathname = pathnameOf(parsed.pathname);
  let destination: string | null = null;
  if (['/workshop', '/workshop/canvases', '/workshop/projects'].includes(pathname)) destination = CANVASES_PATH;
  const canvas = /^\/workshop\/canvas\/([^/]+)$/.exec(pathname);
  if (canvas) {
    try { destination = canvasPath(decodeURIComponent(canvas[1])); } catch { return null; }
  }
  return destination ? `${destination}${parsed.search}${parsed.hash}` : null;
}
