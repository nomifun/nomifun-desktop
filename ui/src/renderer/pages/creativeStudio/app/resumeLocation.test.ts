/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  CREATIVE_STUDIO_CANVASES_RESUME_LOCATION_KEY,
  CREATIVE_STUDIO_RESUME_LOCATION_KEY,
  normalizeCreativeStudioCanvasesResumeLocation,
  normalizeCreativeStudioResumeLocation,
  readCreativeStudioCanvasesResumeLocation,
  readCreativeStudioResumeLocation,
  rememberCreativeStudioResumeLocation,
} from './resumeLocation';

const PROJECT_ID = '0190f5fe-7c00-7a00-8000-000000000701';

const memoryStorage = () => {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    values,
  };
};

describe('Creative Studio resume location', () => {
  test('keeps exact current routes with their search and hash intact', () => {
    for (const path of [
      '/workshop',
      '/workshop/canvases',
      '/workshop/projects',
      `/workshop/canvas/${PROJECT_ID}`,
      `/workshop/director/${PROJECT_ID}#timeline`,
      '/workshop/image?panel=history#result-2',
      '/workshop/video',
      '/workshop/prompts?query=portrait',
      '/workshop/assets?page=3',
      '/workshop/workflows#runs',
    ]) {
      expect(normalizeCreativeStudioResumeLocation(path)).toBe(path);
    }
  });

  test('fails closed for outside, unknown, absolute, and oversized locations', () => {
    for (const path of [
      '',
      '/guid',
      '/workshop-other',
      '/workshop/unknown',
      '//evil.example/workshop',
      'https://evil.example/workshop',
      `/workshop/prompts?value=${'x'.repeat(4096)}`,
    ]) {
      expect(normalizeCreativeStudioResumeLocation(path)).toBe('/workshop');
    }
  });

  test('keeps a separate My Canvases detail location across sibling product tabs', () => {
    const storage = memoryStorage();
    const canvasPath = `/workshop/canvas/${PROJECT_ID}?mode=focus#node-1`;
    const imagePath = '/workshop/image?panel=history';

    expect(readCreativeStudioCanvasesResumeLocation(storage)).toBe(
      '/workshop/canvases'
    );
    expect(rememberCreativeStudioResumeLocation(canvasPath, storage)).toBe(
      canvasPath
    );
    expect(
      storage.values.get(CREATIVE_STUDIO_CANVASES_RESUME_LOCATION_KEY)
    ).toBe(canvasPath);

    expect(rememberCreativeStudioResumeLocation(imagePath, storage)).toBe(
      imagePath
    );
    expect(readCreativeStudioResumeLocation(storage)).toBe(imagePath);
    expect(readCreativeStudioCanvasesResumeLocation(storage)).toBe(canvasPath);
  });

  test('accepts only Canvas-family resume routes and canonicalizes the legacy list', () => {
    const directorPath = `/workshop/director/${PROJECT_ID}?camera=primary#timeline`;

    expect(
      normalizeCreativeStudioCanvasesResumeLocation('/workshop/canvases')
    ).toBe('/workshop/canvases');
    expect(
      normalizeCreativeStudioCanvasesResumeLocation(
        `/workshop/canvas/${PROJECT_ID}#node-2`
      )
    ).toBe(`/workshop/canvas/${PROJECT_ID}#node-2`);
    expect(
      normalizeCreativeStudioCanvasesResumeLocation(directorPath)
    ).toBe(directorPath);
    expect(
      normalizeCreativeStudioCanvasesResumeLocation(
        '/workshop/projects?sort=updated#recent'
      )
    ).toBe('/workshop/canvases?sort=updated#recent');

    for (const path of [
      '/workshop',
      '/workshop/image',
      '/workshop/prompts',
      '/workshop/canvas',
      '/workshop/canvas/%E0%A4%A',
      '/guid',
      '//evil.example/workshop/canvases',
      'https://evil.example/workshop/canvases',
      `/workshop/canvases?value=${'x'.repeat(4096)}`,
    ]) {
      expect(normalizeCreativeStudioCanvasesResumeLocation(path)).toBe(null);
    }
  });

  test('lets an explicit Canvas list visit replace a prior detail resume', () => {
    const storage = memoryStorage();
    rememberCreativeStudioResumeLocation(
      `/workshop/canvas/${PROJECT_ID}`,
      storage
    );

    rememberCreativeStudioResumeLocation(
      '/workshop/canvases?sort=updated',
      storage
    );

    expect(readCreativeStudioCanvasesResumeLocation(storage)).toBe(
      '/workshop/canvases?sort=updated'
    );
  });

  test('bootstraps the section resume from the original product-wide key', () => {
    const storage = memoryStorage();
    const canvasPath = `/workshop/canvas/${PROJECT_ID}`;
    storage.setItem(CREATIVE_STUDIO_RESUME_LOCATION_KEY, canvasPath);

    expect(readCreativeStudioCanvasesResumeLocation(storage)).toBe(canvasPath);
  });

  test('round-trips through session-scoped storage and tolerates unavailable storage', () => {
    const storage = memoryStorage();
    const path = '/workshop/image?panel=history';

    expect(readCreativeStudioResumeLocation(storage)).toBe('/workshop');
    expect(rememberCreativeStudioResumeLocation(path, storage)).toBe(path);
    expect(storage.values.get(CREATIVE_STUDIO_RESUME_LOCATION_KEY)).toBe(path);
    expect(readCreativeStudioResumeLocation(storage)).toBe(path);

    const unavailable = {
      getItem: () => {
        throw new Error('unavailable');
      },
      setItem: () => {
        throw new Error('unavailable');
      },
    };
    expect(readCreativeStudioResumeLocation(unavailable)).toBe('/workshop');
    expect(readCreativeStudioCanvasesResumeLocation(unavailable)).toBe(
      '/workshop/canvases'
    );
    expect(rememberCreativeStudioResumeLocation('/workshop/prompts', unavailable)).toBe(
      '/workshop/prompts'
    );
  });
});
