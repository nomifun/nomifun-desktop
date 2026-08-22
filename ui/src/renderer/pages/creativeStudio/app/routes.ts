/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export const CREATIVE_STUDIO_ROOT_PATH = '/workshop';
export const CREATIVE_STUDIO_PROJECTS_PATH = `${CREATIVE_STUDIO_ROOT_PATH}/projects`;
export const CREATIVE_STUDIO_CANVAS_PROJECT_PATTERN = '/workshop/canvas/:projectId';
export const CREATIVE_STUDIO_DIRECTOR_PROJECT_PATTERN = '/workshop/director/:projectId';
export const CREATIVE_STUDIO_IMAGE_PATH = '/workshop/image';
export const CREATIVE_STUDIO_VIDEO_PATH = '/workshop/video';
export const CREATIVE_STUDIO_PROMPTS_PATH = '/workshop/prompts';
export const CREATIVE_STUDIO_ASSETS_PATH = '/workshop/assets';
export const CREATIVE_STUDIO_WORKFLOWS_PATH = '/workshop/workflows';
export const WORKBENCH_HOME_PATH = '/guid';

export type CreativeStudioSection =
  | 'home'
  | 'projects'
  | 'canvas'
  | 'director'
  | 'image'
  | 'video'
  | 'prompts'
  | 'assets'
  | 'workflows';

export interface CreativeStudioCanvasRouteMatch {
  projectId: string;
}

export interface CreativeStudioDirectorRouteMatch {
  projectId: string;
}

const stripSearchAndHash = (path: string): string => path.split(/[?#]/, 1)[0] || '/';

const normalizePathname = (path: string): string => {
  const pathname = stripSearchAndHash(path.trim());
  if (pathname.length <= 1) return pathname || '/';
  return pathname.replace(/\/+$/, '');
};

export const creativeStudioCanvasProjectPath = (projectId: string): string => {
  const normalized = projectId.trim();
  if (!normalized) throw new Error('Creative Studio project id is required');
  return `${CREATIVE_STUDIO_ROOT_PATH}/canvas/${encodeURIComponent(normalized)}`;
};

export const creativeStudioDirectorProjectPath = (projectId: string): string => {
  const normalized = projectId.trim();
  if (!normalized) throw new Error('Creative Studio project id is required');
  return `${CREATIVE_STUDIO_ROOT_PATH}/director/${encodeURIComponent(normalized)}`;
};

export const matchCreativeStudioCanvasProjectPath = (
  path: string
): CreativeStudioCanvasRouteMatch | null => {
  const pathname = normalizePathname(path);
  const match = /^\/workshop\/canvas\/([^/]+)$/.exec(pathname);
  if (!match) return null;
  try {
    const projectId = decodeURIComponent(match[1]).trim();
    return projectId ? { projectId } : null;
  } catch {
    return null;
  }
};

export const matchCreativeStudioDirectorProjectPath = (
  path: string
): CreativeStudioDirectorRouteMatch | null => {
  const pathname = normalizePathname(path);
  const match = /^\/workshop\/director\/([^/]+)$/.exec(pathname);
  if (!match) return null;
  try {
    const projectId = decodeURIComponent(match[1]).trim();
    return projectId ? { projectId } : null;
  } catch {
    return null;
  }
};

/** Exact section matching keeps `/workshop-other` outside the product shell. */
export const creativeStudioSectionForPath = (path: string): CreativeStudioSection | null => {
  const pathname = normalizePathname(path);
  if (pathname === CREATIVE_STUDIO_ROOT_PATH) return 'home';
  if (pathname === CREATIVE_STUDIO_PROJECTS_PATH) return 'projects';
  if (matchCreativeStudioCanvasProjectPath(pathname)) return 'canvas';
  if (matchCreativeStudioDirectorProjectPath(pathname)) return 'director';
  if (pathname === CREATIVE_STUDIO_IMAGE_PATH) return 'image';
  if (pathname === CREATIVE_STUDIO_VIDEO_PATH) return 'video';
  if (pathname === CREATIVE_STUDIO_PROMPTS_PATH) return 'prompts';
  if (pathname === CREATIVE_STUDIO_ASSETS_PATH) return 'assets';
  if (pathname === CREATIVE_STUDIO_WORKFLOWS_PATH) return 'workflows';
  return null;
};

export const isCreativeStudioPath = (path: string): boolean =>
  creativeStudioSectionForPath(path) !== null;
