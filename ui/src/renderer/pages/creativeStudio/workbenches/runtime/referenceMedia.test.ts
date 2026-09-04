/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeAsset } from '../../assets';
import { imageWorkbenchReferencesFromAssets, videoWorkbenchReferencesFromAssets } from './adapters';

const asset = (kind: CreativeAsset['kind'], thumbnailUrl: string | null = null): CreativeAsset => ({
  id: `asset-${kind}`,
  kind,
  title: `${kind} reference`,
  collection: null,
  tags: [],
  mimeType: `${kind}/example`,
  width: null,
  height: null,
  bytes: 1_024,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/assets/${kind}.original`,
  thumbnailUrl,
  createdAt: 1,
  updatedAt: 1,
});

describe('workbench reference media sources', () => {
  test('retains original images for thumbnail failure recovery in both workbenches', () => {
    const image = asset('image', '/assets/image.jpg');
    for (const references of [
      imageWorkbenchReferencesFromAssets([image]),
      videoWorkbenchReferencesFromAssets([image]),
    ]) {
      expect(references[0]?.previewUrl).toBe('/assets/image.jpg');
      expect(references[0]?.originalUrl).toBe(image.originalUrl);
    }
    expect(imageWorkbenchReferencesFromAssets([asset('image')])[0]?.previewUrl).toBe('/assets/image.original');
    expect(videoWorkbenchReferencesFromAssets([asset('image')])[0]?.previewUrl).toBe('/assets/image.original');
  });

  test('keeps video thumbnails and playable originals distinct, including videos without posters', () => {
    const withoutPoster = asset('video');
    const withPoster = asset('video', '/assets/video-poster.jpg');

    expect(videoWorkbenchReferencesFromAssets([withoutPoster])[0]).toMatchObject({
      kind: 'video',
      originalUrl: withoutPoster.originalUrl,
      previewUrl: undefined,
    });
    expect(videoWorkbenchReferencesFromAssets([withPoster])[0]).toMatchObject({
      kind: 'video',
      originalUrl: withPoster.originalUrl,
      previewUrl: '/assets/video-poster.jpg',
    });
  });

  test('never sends audio originals or untyped thumbnails to an image preview', () => {
    const reference = videoWorkbenchReferencesFromAssets([asset('audio', '/assets/audio.original')])[0];
    expect(reference?.kind).toBe('audio');
    expect(reference?.name).toBe('audio reference');
    expect(reference?.previewUrl).toBeUndefined();
    expect(reference?.originalUrl).toBeUndefined();
  });
});
