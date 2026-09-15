/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  imageGenerationAspectRatioChoices,
  imageGenerationAspectRatioValue,
  imageGenerationResolutionOptions,
  imageGenerationSizeOptionForAspectRatio,
  imageGenerationSizeOptionForSettings,
  imageGenerationSizePolicyForModel,
  normalizeImageGenerationSettingsSize,
} from './image';

const defaultOptions = imageGenerationSizePolicyForModel(null).options;

describe('separate image ratio and resolution selection', () => {
  test('retains every existing exact size, including restored high-resolution selections', () => {
    const policies = [
      imageGenerationSizePolicyForModel(null),
      ...['step-image-edit-2', 'step-2x-large', 'unknown'].map((model) =>
        imageGenerationSizePolicyForModel({ model, protocol: 'stepfun.images' })
      ),
      imageGenerationSizePolicyForModel({ model: 'ep-model', protocol: 'ark.images' }),
    ];
    for (const policy of policies) {
      const { options } = policy;
      const groups = imageGenerationAspectRatioChoices(options);
      expect(groups.flatMap((group) => imageGenerationResolutionOptions(options, group.value)).length)
        .toBe(options.length);
      for (const option of options) {
        const restored = normalizeImageGenerationSettingsSize({
          model: null, interfaceMode: 'images', quality: 'auto', count: 1,
          aspectRatio: option.value, width: option.width, height: option.height,
        }, policy);
        expect(imageGenerationSizeOptionForSettings(options, restored)).toBe(option);
        expect(imageGenerationSizeOptionForAspectRatio(options, option, imageGenerationAspectRatioValue(option)))
          .toBe(option);
      }
    }
  });

  test('groups by metadata even when display labels are renamed or translated', () => {
    const options = defaultOptions.map((option) => ({ ...option, label: '自定义显示名称' }));
    const current = options.find((option) => option.value === '2048x2048')!;
    expect(imageGenerationAspectRatioValue(current)).toBe('1:1');
    expect(imageGenerationSizeOptionForAspectRatio(options, current, '9:16')?.value)
      .toBe('1152x2048');
  });

  test('does not invent unsupported ratio/resolution combinations', () => {
    const current = defaultOptions.find((option) => option.value === '3840x2160')!;
    expect(imageGenerationSizeOptionForAspectRatio(defaultOptions, current, '9:16')?.value)
      .toBe('2160x3840');
    expect(imageGenerationSizeOptionForAspectRatio(defaultOptions, current, '4:3')?.value)
      .toBe('4:3');
    expect(imageGenerationSizeOptionForAspectRatio(defaultOptions, current, '99:1')).toBeNull();
  });

  test('keeps automatic sizing separate and preserves provider-native width ordering', () => {
    const options = imageGenerationSizePolicyForModel({
      model: 'step-image-edit-2', protocol: 'stepfun.images',
    }).options;
    const auto = options.find((option) => option.value === 'auto')!;
    expect(imageGenerationResolutionOptions(options, 'auto')).toEqual([auto]);
    expect(imageGenerationSizeOptionForAspectRatio(options, auto, '16:9'))
      .toMatchObject({ value: '16:9', width: 1360, height: 768, requestSize: '768x1360' });
  });

  test('skips disabled sizes and handles empty policies', () => {
    const options = defaultOptions.map((option) => ({
      ...option, disabled: ['1:1', '2048x1152'].includes(option.value),
    }));
    expect(imageGenerationAspectRatioChoices(options).find((choice) => choice.value === '1:1')?.disabled)
      .toBe(false);
    const current = options.find((option) => option.value === '2048x2048')!;
    expect(imageGenerationSizeOptionForAspectRatio(options, current, '16:9')?.value).toBe('16:9');
    expect(imageGenerationSizeOptionForAspectRatio(options.map((option) => ({ ...option, disabled: true })), current, '16:9'))
      .toBeNull();
    expect(imageGenerationAspectRatioChoices([])).toEqual([]);
    expect(imageGenerationSizeOptionForSettings([], { aspectRatio: '1:1' })).toBeNull();
  });
});
