/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  CREATIVE_STUDIO_ASSETS_PATH,
  CREATIVE_STUDIO_AUDIO_PATH,
  CREATIVE_STUDIO_CANVAS_PROJECT_PATTERN,
  CREATIVE_STUDIO_DIRECTOR_PROJECT_PATTERN,
  CREATIVE_STUDIO_IMAGE_PATH,
  CREATIVE_STUDIO_PROJECTS_PATH,
  CREATIVE_STUDIO_PROMPTS_PATH,
  CREATIVE_STUDIO_ROOT_PATH,
  CREATIVE_STUDIO_VIDEO_PATH,
  CREATIVE_STUDIO_WORKFLOWS_PATH,
  creativeStudioCanvasProjectPath,
  creativeStudioDirectorProjectPath,
  creativeStudioSectionForPath,
  isCreativeStudioPath,
  matchCreativeStudioCanvasProjectPath,
  matchCreativeStudioDirectorProjectPath,
} from './routes';

describe('Creative Studio routes', () => {
  test('publishes the canonical deep-link contract', () => {
    expect(CREATIVE_STUDIO_ROOT_PATH).toBe('/workshop');
    expect(CREATIVE_STUDIO_PROJECTS_PATH).toBe('/workshop/projects');
    expect(CREATIVE_STUDIO_CANVAS_PROJECT_PATTERN).toBe('/workshop/canvas/:projectId');
    expect(CREATIVE_STUDIO_DIRECTOR_PROJECT_PATTERN).toBe('/workshop/director/:projectId');
    expect(CREATIVE_STUDIO_IMAGE_PATH).toBe('/workshop/image');
    expect(CREATIVE_STUDIO_VIDEO_PATH).toBe('/workshop/video');
    expect(CREATIVE_STUDIO_AUDIO_PATH).toBe('/workshop/audio');
    expect(CREATIVE_STUDIO_PROMPTS_PATH).toBe('/workshop/prompts');
    expect(CREATIVE_STUDIO_ASSETS_PATH).toBe('/workshop/assets');
    expect(CREATIVE_STUDIO_WORKFLOWS_PATH).toBe('/workshop/workflows');
  });

  test('builds and matches encoded Director project links', () => {
    const path = creativeStudioDirectorProjectPath('  project/一  ');

    expect(path).toBe('/workshop/director/project%2F%E4%B8%80');
    expect(matchCreativeStudioDirectorProjectPath(`${path}/?camera=primary#timeline`)).toEqual({
      projectId: 'project/一',
    });
  });

  test('builds and matches encoded canvas project links', () => {
    const path = creativeStudioCanvasProjectPath('  project/一  ');

    expect(path).toBe('/workshop/canvas/project%2F%E4%B8%80');
    expect(matchCreativeStudioCanvasProjectPath(`${path}/?mode=focus#node-1`)).toEqual({
      projectId: 'project/一',
    });
  });

  test('rejects missing and malformed canvas project links', () => {
    let blankError: Error | null = null;
    try {
      creativeStudioCanvasProjectPath('   ');
    } catch (error) {
      blankError = error as Error;
    }

    expect(blankError?.message).toBe('Creative Studio project id is required');
    expect(matchCreativeStudioCanvasProjectPath('/workshop/canvas')).toBe(null);
    expect(matchCreativeStudioCanvasProjectPath('/workshop/canvas/a/extra')).toBe(null);
    expect(matchCreativeStudioCanvasProjectPath('/workshop/canvas/%E0%A4%A')).toBe(null);
    expect(matchCreativeStudioDirectorProjectPath('/workshop/director')).toBe(null);
    expect(matchCreativeStudioDirectorProjectPath('/workshop/director/a/extra')).toBe(null);
    expect(matchCreativeStudioDirectorProjectPath('/workshop/director/%E0%A4%A')).toBe(null);
  });

  test('matches only exact product sections', () => {
    expect(creativeStudioSectionForPath('/workshop')).toBe('home');
    expect(creativeStudioSectionForPath('/workshop/projects')).toBe('projects');
    expect(creativeStudioSectionForPath('/workshop/canvas/project-1')).toBe('canvas');
    expect(creativeStudioSectionForPath('/workshop/director/project-1')).toBe('director');
    expect(creativeStudioSectionForPath('/workshop/image?draft=1')).toBe('image');
    expect(creativeStudioSectionForPath('/workshop/video/')).toBe('video');
    expect(creativeStudioSectionForPath('/workshop/audio')).toBe('audio');
    expect(creativeStudioSectionForPath('/workshop/prompts')).toBe('prompts');
    expect(creativeStudioSectionForPath('/workshop/assets#recent')).toBe('assets');
    expect(creativeStudioSectionForPath('/workshop/workflows?category=all')).toBe('workflows');
    expect(creativeStudioSectionForPath('/workshop-other')).toBe(null);
    expect(creativeStudioSectionForPath('/workshop/image/draft')).toBe(null);
    expect(isCreativeStudioPath('/workshop/video')).toBe(true);
    expect(isCreativeStudioPath('/workshop-other')).toBe(false);
  });
});
