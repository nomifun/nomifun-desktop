/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  CREATIVE_STUDIO_ASSETS_PATH,
  CREATIVE_STUDIO_CANVASES_PATH,
  CREATIVE_STUDIO_CANVAS_PATTERN,
  CREATIVE_STUDIO_DIRECTOR_PATTERN,
  CREATIVE_STUDIO_IMAGE_PATH,
  CREATIVE_STUDIO_LEGACY_PROJECTS_PATH,
  CREATIVE_STUDIO_PROMPTS_PATH,
  CREATIVE_STUDIO_PROJECTS_PATH,
  CREATIVE_STUDIO_ROOT_PATH,
  CREATIVE_STUDIO_VIDEO_PATH,
  CREATIVE_STUDIO_WORKFLOWS_PATH,
  creativeStudioCanvasPath,
  creativeStudioCanvasProjectPath,
  creativeStudioDirectorPath,
  creativeStudioSectionForPath,
  isCreativeStudioPath,
  matchCreativeStudioCanvasPath,
  matchCreativeStudioCanvasProjectPath,
  matchCreativeStudioDirectorPath,
} from './routes';

describe('Creative Studio routes', () => {
  test('publishes Canvas-first canonical deep links and a legacy library redirect path', () => {
    expect(CREATIVE_STUDIO_ROOT_PATH).toBe('/workshop');
    expect(CREATIVE_STUDIO_CANVASES_PATH).toBe('/workshop/canvases');
    expect(CREATIVE_STUDIO_LEGACY_PROJECTS_PATH).toBe('/workshop/projects');
    expect(CREATIVE_STUDIO_CANVAS_PATTERN).toBe(
      '/workshop/canvas/:canvasId'
    );
    expect(CREATIVE_STUDIO_DIRECTOR_PATTERN).toBe(
      '/workshop/director/:canvasId'
    );
    expect(CREATIVE_STUDIO_IMAGE_PATH).toBe('/workshop/image');
    expect(CREATIVE_STUDIO_VIDEO_PATH).toBe('/workshop/video');
    expect(CREATIVE_STUDIO_PROMPTS_PATH).toBe('/workshop/prompts');
    expect(CREATIVE_STUDIO_ASSETS_PATH).toBe('/workshop/assets');
    expect(CREATIVE_STUDIO_WORKFLOWS_PATH).toBe('/workshop/workflows');
  });

  test('builds and matches encoded current-Canvas Director links', () => {
    const path = creativeStudioDirectorPath('  canvas/一  ');

    expect(path).toBe('/workshop/director/canvas%2F%E4%B8%80');
    expect(matchCreativeStudioDirectorPath(`${path}/?camera=primary#timeline`)).toEqual({
      canvasId: 'canvas/一',
    });
  });

  test('builds and matches encoded Canvas links', () => {
    const path = creativeStudioCanvasPath('  canvas/一  ');

    expect(path).toBe('/workshop/canvas/canvas%2F%E4%B8%80');
    expect(matchCreativeStudioCanvasPath(`${path}/?mode=focus#node-1`)).toEqual({
      canvasId: 'canvas/一',
    });
  });

  test('rejects missing and malformed Canvas links', () => {
    let blankError: Error | null = null;
    try {
      creativeStudioCanvasPath('   ');
    } catch (error) {
      blankError = error as Error;
    }

    expect(blankError?.message).toBe('Creative Studio canvas id is required');
    expect(matchCreativeStudioCanvasPath('/workshop/canvas')).toBe(null);
    expect(matchCreativeStudioCanvasPath('/workshop/canvas/a/extra')).toBe(null);
    expect(matchCreativeStudioCanvasPath('/workshop/canvas/%E0%A4%A')).toBe(null);
    expect(matchCreativeStudioDirectorPath('/workshop/director')).toBe(null);
    expect(matchCreativeStudioDirectorPath('/workshop/director/a/extra')).toBe(null);
    expect(matchCreativeStudioDirectorPath('/workshop/director/%E0%A4%A')).toBe(null);
  });

  test('keeps deprecated helpers as aliases to canonical Canvas destinations', () => {
    expect(CREATIVE_STUDIO_PROJECTS_PATH).toBe(CREATIVE_STUDIO_CANVASES_PATH);
    expect(creativeStudioCanvasProjectPath('canvas-1')).toBe(
      creativeStudioCanvasPath('canvas-1')
    );
    expect(
      matchCreativeStudioCanvasProjectPath('/workshop/canvas/canvas-1')
    ).toEqual({ projectId: 'canvas-1' });
  });

  test('matches only exact product sections', () => {
    expect(creativeStudioSectionForPath('/workshop')).toBe('canvases');
    expect(creativeStudioSectionForPath('/workshop/canvases')).toBe('canvases');
    expect(creativeStudioSectionForPath('/workshop/projects')).toBe('canvases');
    expect(creativeStudioSectionForPath('/workshop/canvas/canvas-1')).toBe('canvas');
    expect(creativeStudioSectionForPath('/workshop/director/canvas-1')).toBe('director');
    expect(creativeStudioSectionForPath('/workshop/image?draft=1')).toBe('image');
    expect(creativeStudioSectionForPath('/workshop/video/')).toBe('video');
    expect(creativeStudioSectionForPath('/workshop/prompts')).toBe('prompts');
    expect(creativeStudioSectionForPath('/workshop/assets#recent')).toBe('assets');
    expect(creativeStudioSectionForPath('/workshop/workflows?category=all')).toBe('workflows');
    expect(creativeStudioSectionForPath('/workshop-other')).toBe(null);
    expect(creativeStudioSectionForPath('/workshop/audio')).toBe(null);
    expect(creativeStudioSectionForPath('/workshop/image/draft')).toBe(null);
    expect(isCreativeStudioPath('/workshop/video')).toBe(true);
    expect(isCreativeStudioPath('/workshop/audio')).toBe(false);
    expect(isCreativeStudioPath('/workshop-other')).toBe(false);
  });
});
