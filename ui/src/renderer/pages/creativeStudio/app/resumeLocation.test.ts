/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  CREATIVE_STUDIO_RESUME_LOCATION_KEY,
  normalizeCreativeStudioResumeLocation,
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
      '/workshop/projects',
      `/workshop/canvas/${PROJECT_ID}`,
      `/workshop/director/${PROJECT_ID}#timeline`,
      // Legacy standalone projectId is preserved by this checkpoint only.
      // Phase 1 of the Canvas-domain redesign removes it from canonical URLs.
      `/workshop/image?projectId=${PROJECT_ID}&panel=history#result-2`,
      `/workshop/video?projectId=${PROJECT_ID}`,
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

  test('round-trips through session-scoped storage and tolerates unavailable storage', () => {
    const storage = memoryStorage();
    const path = `/workshop/image?projectId=${PROJECT_ID}`;

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
    expect(rememberCreativeStudioResumeLocation('/workshop/prompts', unavailable)).toBe(
      '/workshop/prompts'
    );
  });
});
