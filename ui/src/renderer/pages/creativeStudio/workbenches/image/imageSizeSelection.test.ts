/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  imageWorkbenchAspectRatioChoices,
  imageWorkbenchAspectRatioValue,
  imageWorkbenchResolutionOptions,
  imageWorkbenchSizeOptionForAspectRatio,
  imageWorkbenchSizeOptionForSettings,
  imageWorkbenchSizePolicyForModel,
  normalizeImageWorkbenchSettingsSize,
} from './types';

const defaultOptions = imageWorkbenchSizePolicyForModel(null).options;

describe('separate image ratio and resolution selection', () => {
  test('retains every existing exact size, including restored high-resolution selections', () => {
    const policies = [
      imageWorkbenchSizePolicyForModel(null),
      ...['step-image-edit-2', 'step-2x-large', 'unknown'].map((model) =>
        imageWorkbenchSizePolicyForModel({ model, protocol: 'stepfun.images' })
      ),
      imageWorkbenchSizePolicyForModel({ model: 'ep-model', protocol: 'ark.images' }),
    ];
    for (const policy of policies) {
      const { options } = policy;
      const groups = imageWorkbenchAspectRatioChoices(options);
      expect(groups.flatMap((group) => imageWorkbenchResolutionOptions(options, group.value)).length)
        .toBe(options.length);
      for (const option of options) {
        const restored = normalizeImageWorkbenchSettingsSize({
          model: null, interfaceMode: 'images', quality: 'auto', count: 1,
          aspectRatio: option.value, width: option.width, height: option.height,
        }, policy);
        expect(imageWorkbenchSizeOptionForSettings(options, restored)).toBe(option);
        expect(imageWorkbenchSizeOptionForAspectRatio(options, option, imageWorkbenchAspectRatioValue(option)))
          .toBe(option);
      }
    }
  });

  test('groups by metadata even when display labels are renamed or translated', () => {
    const options = defaultOptions.map((option) => ({ ...option, label: '自定义显示名称' }));
    const current = options.find((option) => option.value === '2048x2048')!;
    expect(imageWorkbenchAspectRatioValue(current)).toBe('1:1');
    expect(imageWorkbenchSizeOptionForAspectRatio(options, current, '9:16')?.value)
      .toBe('1152x2048');
  });

  test('does not invent unsupported ratio/resolution combinations', () => {
    const current = defaultOptions.find((option) => option.value === '3840x2160')!;
    expect(imageWorkbenchSizeOptionForAspectRatio(defaultOptions, current, '9:16')?.value)
      .toBe('2160x3840');
    expect(imageWorkbenchSizeOptionForAspectRatio(defaultOptions, current, '4:3')?.value)
      .toBe('4:3');
    expect(imageWorkbenchSizeOptionForAspectRatio(defaultOptions, current, '99:1')).toBeNull();
  });

  test('keeps automatic sizing separate and preserves provider-native width ordering', () => {
    const options = imageWorkbenchSizePolicyForModel({
      model: 'step-image-edit-2', protocol: 'stepfun.images',
    }).options;
    const auto = options.find((option) => option.value === 'auto')!;
    expect(imageWorkbenchResolutionOptions(options, 'auto')).toEqual([auto]);
    expect(imageWorkbenchSizeOptionForAspectRatio(options, auto, '16:9'))
      .toMatchObject({ value: '16:9', width: 1360, height: 768, requestSize: '768x1360' });
  });

  test('skips disabled sizes and handles empty policies', () => {
    const options = defaultOptions.map((option) => ({
      ...option, disabled: ['1:1', '2048x1152'].includes(option.value),
    }));
    expect(imageWorkbenchAspectRatioChoices(options).find((choice) => choice.value === '1:1')?.disabled)
      .toBe(false);
    const current = options.find((option) => option.value === '2048x2048')!;
    expect(imageWorkbenchSizeOptionForAspectRatio(options, current, '16:9')?.value).toBe('16:9');
    expect(imageWorkbenchSizeOptionForAspectRatio(options.map((option) => ({ ...option, disabled: true })), current, '16:9'))
      .toBeNull();
    expect(imageWorkbenchAspectRatioChoices([])).toEqual([]);
    expect(imageWorkbenchSizeOptionForSettings([], { aspectRatio: '1:1' })).toBeNull();
  });
});
