/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { inferPromptAspectRatio, resolveMediaAspectRatio } from './aspectRatio';
import {
  imageWorkbenchSizeOptionForAspectRatio,
  imageWorkbenchSizePolicyForModel,
} from './image';

const IMAGE_RATIOS = ['1:1', '3:2', '2:3', '4:3', '3:4', '16:9', '9:16', '21:9'];

describe('prompt aspect-ratio inference', () => {
  test('gives a fixed generation parameter absolute priority over the prompt', () => {
    expect(resolveMediaAspectRatio('16:9', '9:16 竖构图', IMAGE_RATIOS)).toBe('16:9');
  });

  test('uses an explicit numeric ratio before broader orientation wording', () => {
    expect(inferPromptAspectRatio('3:4 竖构图，古风人物', IMAGE_RATIOS)).toBe('3:4');
    expect(inferPromptAspectRatio('横构图，但画幅比例 9：16', IMAGE_RATIOS)).toBe('9:16');
    expect(inferPromptAspectRatio('生成 1080×1920 的海报', IMAGE_RATIOS)).toBe('9:16');
  });

  test('projects an inferred image ratio into the selected provider wire size', () => {
    const policy = imageWorkbenchSizePolicyForModel({
      model: 'step-image-edit-2',
      protocol: 'stepfun.images',
    });
    const automatic = policy.options.find((option) => option.value === 'auto') ?? null;
    const inferred = resolveMediaAspectRatio(
      'auto',
      '3:4 竖构图，古风人物',
      policy.options.map((option) => option.aspectRatio ?? option.value)
    );
    const requestOption = inferred
      ? imageWorkbenchSizeOptionForAspectRatio(policy.options, automatic, inferred)
      : null;

    expect(requestOption).toMatchObject({
      value: '3:4',
      width: 896,
      height: 1184,
      requestSize: '1184x896',
    });
  });

  test('normalizes mathematically equivalent ratios to a supported option', () => {
    expect(inferPromptAspectRatio('6:8 portrait', IMAGE_RATIOS)).toBe('3:4');
    expect(inferPromptAspectRatio('画幅 7:3', IMAGE_RATIOS)).toBe('21:9');
  });

  test('maps direction-only prompts to stable supported defaults', () => {
    expect(inferPromptAspectRatio('竖版人物海报', IMAGE_RATIOS)).toBe('9:16');
    expect(inferPromptAspectRatio('cinematic landscape composition', IMAGE_RATIOS)).toBe('16:9');
    expect(inferPromptAspectRatio('正方形头像', IMAGE_RATIOS)).toBe('1:1');
  });

  test('keeps model-side automatic sizing when there is no supported ratio signal', () => {
    expect(inferPromptAspectRatio('柔和自然光下的人物肖像', IMAGE_RATIOS)).toBeNull();
    expect(inferPromptAspectRatio('5:4 竖构图', ['1:1', '16:9', '9:16'])).toBeNull();
    expect(resolveMediaAspectRatio('auto', '柔和自然光', IMAGE_RATIOS)).toBeNull();
  });
});
