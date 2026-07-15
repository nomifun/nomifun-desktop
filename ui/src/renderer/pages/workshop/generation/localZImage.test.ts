/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseAssetId, parseProviderId } from '@/common/types/ids';
import type { CreateTaskBody } from '../types';
import {
  LOCAL_Z_IMAGE_MODEL_ID,
  LOCAL_Z_IMAGE_PLATFORM,
  normalizeLocalZImageDimension,
  normalizeLocalZImageParams,
  validateLocalZImageRun,
  validateLocalZImageTask,
} from './localZImage';

const localTask = (patch: Partial<CreateTaskBody> = {}): CreateTaskBody => ({
  provider_id: parseProviderId('prov_019bef52-a123-7abc-8def-0123456789ab'),
  provider_platform: LOCAL_Z_IMAGE_PLATFORM,
  model: LOCAL_Z_IMAGE_MODEL_ID,
  capability: 't2i',
  params: { prompt: 'a cat', width: 1024, height: 1024, count: 1 },
  inputs: [],
  ...patch,
});

describe('local Z-Image frontend contract', () => {
  test('normalizes stale dimensions and batch count', () => {
    expect(normalizeLocalZImageDimension(4096)).toBe(2048);
    expect(normalizeLocalZImageDimension(515)).toBe(512);
    expect(normalizeLocalZImageDimension(1)).toBe(256);
    expect(normalizeLocalZImageParams({ preset: '4k', width: 4096, height: 4096, count: 8 })).toMatchObject({
      preset: '2k',
      width: 2048,
      height: 2048,
      count: 1,
    });
  });

  test('accepts only text-to-image runs without resolved image inputs', () => {
    const model = { platform: LOCAL_Z_IMAGE_PLATFORM, model: LOCAL_Z_IMAGE_MODEL_ID };
    expect(validateLocalZImageRun(model, 't2i', [])).toBeNull();
    expect(validateLocalZImageRun(model, 'i2i', [])).toBe('text_to_image_only');
    expect(validateLocalZImageRun(model, 'inpaint', [])).toBe('text_to_image_only');
    expect(validateLocalZImageRun(model, 't2i', [{ asset_id: parseAssetId('wsa_019bef52-a123-7abc-8def-0123456789ab'), role: 'reference' }])).toBe(
      'text_to_image_only'
    );
  });

  test('guards every task submission against stale local parameters', () => {
    expect(validateLocalZImageTask(localTask())).toBeNull();
    expect(validateLocalZImageTask(localTask({ capability: 'i2i' }))).toBe('text_to_image_only');
    expect(validateLocalZImageTask(localTask({ params: { width: 4096, height: 1024, count: 1 } }))).toBe(
      'invalid_dimensions'
    );
    expect(validateLocalZImageTask(localTask({ params: { width: 512, height: 513, count: 1 } }))).toBe(
      'invalid_dimensions'
    );
    expect(validateLocalZImageTask(localTask({ params: { width: 512, height: 512, count: 2 } }))).toBe(
      'single_image_only'
    );
    expect(
      validateLocalZImageTask({
        ...localTask(),
        provider_id: parseProviderId('prov_019bef52-a123-7abc-8def-0123456789ac'),
        provider_platform: 'openai',
        model: 'gpt-image-1',
        capability: 'i2i',
        params: { width: 4096, height: 4096, count: 4 },
        inputs: [{ asset_id: parseAssetId('wsa_019bef52-a123-7abc-8def-0123456789ac'), role: 'reference' }],
      })
    ).toBeNull();
  });
});
