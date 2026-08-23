/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  createDirectorCharacter,
  createDirectorState,
  isDirectorAssetId,
  normalizeDirectorAspectRatio,
} from './model';

describe('director model factories', () => {
  test('requires caller-owned stable IDs and does not create placeholder assets', () => {
    const state = createDirectorState({
      projectId: 'project-1',
      name: 'Project',
      sceneName: 'Scene',
    });
    expect(state.cameras).toEqual([]);
    expect(state.characters).toEqual([]);
    expect(state.objects).toEqual([]);
    expect(state.lights).toEqual([]);

    const character = createDirectorCharacter({ id: 'character-1', name: 'Character' });
    expect(character.asset).toBeNull();
  });

  test('accepts asset IDs but rejects URLs at the domain boundary', () => {
    expect(isDirectorAssetId('asset-0190')).toBe(true);
    expect(isDirectorAssetId('https://example.test/model.glb')).toBe(false);
    let error: unknown;
    try {
      createDirectorCharacter({
        id: 'character-1',
        name: 'Character',
        assetId: 'https://example.test/model.glb',
      });
    } catch (reason) {
      error = reason;
    }
    expect(error instanceof TypeError).toBe(true);
  });

  test('reduces integer camera ratios and rejects invalid dimensions', () => {
    expect(normalizeDirectorAspectRatio({ width: 3840, height: 2160 })).toEqual({
      width: 16,
      height: 9,
    });
    expect(normalizeDirectorAspectRatio({ width: 0, height: 9 })).toBeNull();
    expect(normalizeDirectorAspectRatio({ width: 1.5, height: 1 })).toBeNull();
  });
});
