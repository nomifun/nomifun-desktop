/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  effectiveImageReferenceInputLimit,
  IMAGE_REFERENCE_PRODUCT_MAX_INPUTS,
  imageReferenceInputPolicy,
} from './imageReferenceInputPolicy';

describe('imageReferenceInputPolicy', () => {
  test('reports the exact bounded image-edit contracts', () => {
    expect(imageReferenceInputPolicy('stepfun.images', 'image_edit')).toEqual({
      kind: 'bounded',
      maxInputs: 1,
    });
    expect(imageReferenceInputPolicy('siliconflow.images', 'image_edit')).toEqual({
      kind: 'bounded',
      maxInputs: 3,
    });
    expect(imageReferenceInputPolicy('xai.images_json', 'image_edit')).toEqual({
      kind: 'bounded',
      maxInputs: 3,
    });
  });

  test('keeps known multi-image transports distinct from an invented maximum', () => {
    for (const protocol of ['openai.images', 'gemini.generate_content']) {
      expect(imageReferenceInputPolicy(protocol, 'image_edit')).toEqual({
        kind: 'multiple',
        maxInputs: null,
      });
    }
  });

  test('reports no reference inputs for image generation', () => {
    expect(imageReferenceInputPolicy(undefined, 'image_generation')).toEqual({
      kind: 'none',
      maxInputs: 0,
    });
    expect(imageReferenceInputPolicy('xai.images_json', 'image_generation')).toEqual({
      kind: 'none',
      maxInputs: 0,
    });
  });

  test('fails closed when protocol or task data is not exact', () => {
    for (const protocol of [undefined, null, '', 'XAI.IMAGES_JSON', ' xai.images_json ']) {
      expect(imageReferenceInputPolicy(protocol, 'image_edit')).toEqual({
        kind: 'unknown',
        maxInputs: null,
      });
    }
    expect(imageReferenceInputPolicy('xai.images_json', 'video_generation')).toEqual({
      kind: 'unknown',
      maxInputs: null,
    });
  });

  test('applies an explicit product safety ceiling without rewriting Provider policy', () => {
    expect(
      effectiveImageReferenceInputLimit({ kind: 'multiple', maxInputs: null })
    ).toBe(IMAGE_REFERENCE_PRODUCT_MAX_INPUTS);
    expect(
      effectiveImageReferenceInputLimit({ kind: 'bounded', maxInputs: 3 })
    ).toBe(3);
    expect(
      effectiveImageReferenceInputLimit({ kind: 'bounded', maxInputs: 99 })
    ).toBe(IMAGE_REFERENCE_PRODUCT_MAX_INPUTS);
    expect(
      effectiveImageReferenceInputLimit({ kind: 'unknown', maxInputs: null })
    ).toBeNull();
  });
});
