/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export const CREATIVE_STUDIO_ROOT_PATH = '/workshop';
export const CREATIVE_STUDIO_CANVASES_PATH =
  `${CREATIVE_STUDIO_ROOT_PATH}/canvases`;
/** @deprecated Redirect-only compatibility path. */
export const CREATIVE_STUDIO_LEGACY_PROJECTS_PATH =
  `${CREATIVE_STUDIO_ROOT_PATH}/projects`;
export const CREATIVE_STUDIO_CANVAS_PATTERN =
  '/workshop/canvas/:canvasId';
export const CREATIVE_STUDIO_DIRECTOR_PATTERN =
  '/workshop/director/:canvasId';
export const CREATIVE_STUDIO_IMAGE_PATH = '/workshop/image';
export const CREATIVE_STUDIO_VIDEO_PATH = '/workshop/video';
export const CREATIVE_STUDIO_PROMPTS_PATH = '/workshop/prompts';
export const CREATIVE_STUDIO_ASSETS_PATH = '/workshop/assets';
export const CREATIVE_STUDIO_TEMPLATES_PATH = '/workshop/templates';
/** @deprecated Redirect-only compatibility path. */
export const CREATIVE_STUDIO_LEGACY_WORKFLOWS_PATH = '/workshop/workflows';
/** @deprecated Use CREATIVE_STUDIO_TEMPLATES_PATH. */
export const CREATIVE_STUDIO_WORKFLOWS_PATH = CREATIVE_STUDIO_TEMPLATES_PATH;
export const WORKBENCH_HOME_PATH = '/guid';

export type CreativeStudioSection =
  | 'canvases'
  | 'canvas'
  | 'director'
  | 'image'
  | 'video'
  | 'prompts'
  | 'assets'
  | 'templates';

export interface CreativeStudioCanvasRouteMatch {
  canvasId: string;
}

export interface CreativeStudioDirectorRouteMatch {
  canvasId: string;
}

const stripSearchAndHash = (path: string): string =>
  path.split(/[?#]/, 1)[0] || '/';

const normalizePathname = (path: string): string => {
  const pathname = stripSearchAndHash(path.trim());
  if (pathname.length <= 1) return pathname || '/';
  return pathname.replace(/\/+$/, '');
};

const requiredCanvasId = (canvasId: string): string => {
  const normalized = canvasId.trim();
  if (!normalized) throw new Error('Creative Studio canvas id is required');
  return normalized;
};

export const creativeStudioCanvasPath = (canvasId: string): string =>
  `${CREATIVE_STUDIO_ROOT_PATH}/canvas/${encodeURIComponent(
    requiredCanvasId(canvasId)
  )}`;

export const creativeStudioDirectorPath = (canvasId: string): string =>
  `${CREATIVE_STUDIO_ROOT_PATH}/director/${encodeURIComponent(
    requiredCanvasId(canvasId)
  )}`;

const matchCanvasId = (
  path: string,
  pattern: RegExp
): string | null => {
  const match = pattern.exec(normalizePathname(path));
  if (!match) return null;
  try {
    const canvasId = decodeURIComponent(match[1]).trim();
    return canvasId || null;
  } catch {
    return null;
  }
};

export const matchCreativeStudioCanvasPath = (
  path: string
): CreativeStudioCanvasRouteMatch | null => {
  const canvasId = matchCanvasId(path, /^\/workshop\/canvas\/([^/]+)$/);
  return canvasId ? { canvasId } : null;
};

export const matchCreativeStudioDirectorPath = (
  path: string
): CreativeStudioDirectorRouteMatch | null => {
  const canvasId = matchCanvasId(path, /^\/workshop\/director\/([^/]+)$/);
  return canvasId ? { canvasId } : null;
};

/** Exact section matching keeps `/workshop-other` outside the product shell. */
export const creativeStudioSectionForPath = (
  path: string
): CreativeStudioSection | null => {
  const pathname = normalizePathname(path);
  if (pathname === CREATIVE_STUDIO_ROOT_PATH) return 'canvases';
  if (
    pathname === CREATIVE_STUDIO_CANVASES_PATH ||
    pathname === CREATIVE_STUDIO_LEGACY_PROJECTS_PATH
  ) {
    return 'canvases';
  }
  if (matchCreativeStudioCanvasPath(pathname)) return 'canvas';
  if (matchCreativeStudioDirectorPath(pathname)) return 'director';
  if (pathname === CREATIVE_STUDIO_IMAGE_PATH) return 'image';
  if (pathname === CREATIVE_STUDIO_VIDEO_PATH) return 'video';
  if (pathname === CREATIVE_STUDIO_PROMPTS_PATH) return 'prompts';
  if (pathname === CREATIVE_STUDIO_ASSETS_PATH) return 'assets';
  if (
    pathname === CREATIVE_STUDIO_TEMPLATES_PATH ||
    pathname === CREATIVE_STUDIO_LEGACY_WORKFLOWS_PATH
  ) {
    return 'templates';
  }
  return null;
};

export const isCreativeStudioPath = (path: string): boolean =>
  creativeStudioSectionForPath(path) !== null;

/**
 * @deprecated Compatibility exports for canvas/editor modules that have not
 * migrated their local variable names. All paths resolve to Canvas routes.
 */
export const CREATIVE_STUDIO_PROJECTS_PATH = CREATIVE_STUDIO_CANVASES_PATH;
/** @deprecated Use CREATIVE_STUDIO_CANVAS_PATTERN. */
export const CREATIVE_STUDIO_CANVAS_PROJECT_PATTERN =
  CREATIVE_STUDIO_CANVAS_PATTERN;
/** @deprecated Use CREATIVE_STUDIO_DIRECTOR_PATTERN. */
export const CREATIVE_STUDIO_DIRECTOR_PROJECT_PATTERN =
  CREATIVE_STUDIO_DIRECTOR_PATTERN;
/** @deprecated Use creativeStudioCanvasPath. */
export const creativeStudioCanvasProjectPath = creativeStudioCanvasPath;
/** @deprecated Use creativeStudioDirectorPath. */
export const creativeStudioDirectorProjectPath = creativeStudioDirectorPath;

/** @deprecated Use CreativeStudioCanvasRouteMatch. */
export interface LegacyCreativeStudioCanvasProjectRouteMatch {
  projectId: string;
}

/** @deprecated Use CreativeStudioDirectorRouteMatch. */
export interface LegacyCreativeStudioDirectorProjectRouteMatch {
  projectId: string;
}

/** @deprecated Use matchCreativeStudioCanvasPath. */
export const matchCreativeStudioCanvasProjectPath = (
  path: string
): LegacyCreativeStudioCanvasProjectRouteMatch | null => {
  const match = matchCreativeStudioCanvasPath(path);
  return match ? { projectId: match.canvasId } : null;
};

/** @deprecated Use matchCreativeStudioDirectorPath. */
export const matchCreativeStudioDirectorProjectPath = (
  path: string
): LegacyCreativeStudioDirectorProjectRouteMatch | null => {
  const match = matchCreativeStudioDirectorPath(path);
  return match ? { projectId: match.canvasId } : null;
};
