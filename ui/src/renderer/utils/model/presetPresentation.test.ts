/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parsePresetReference, type Preset } from '@/common/types/agent/presetTypes';
import {
  isEmoji,
  resolvePresetAvatarImageSrc,
  resolvePresetCatalogName,
} from './presetPresentation';

const preset = {
  preset_id: parsePresetReference('0190f5fe-7c00-7a00-8000-000000000011'),
  name: 'Bug troubleshooting',
  name_i18n: { 'zh-CN': 'Bug 排查', 'en-US': 'Bug troubleshooting' },
} as Pick<Preset, 'preset_id' | 'name' | 'name_i18n'>;

describe('preset presentation', () => {
  test('uses the configured localized name with canonical fallbacks', () => {
    expect(resolvePresetCatalogName(preset, 'zh-Hans')).toBe('Bug 排查');
    expect(resolvePresetCatalogName(preset, 'fr-FR')).toBe('Bug troubleshooting');
    expect(resolvePresetCatalogName({ ...preset, name_i18n: {}, name: '  Custom name  ' }, 'zh-CN')).toBe(
      'Custom name'
    );
  });

  test('classifies the reported CDN AVIF avatar as an image, never emoji text', () => {
    const avatar =
      'https://cloudcache.tencent-cloud.com/qcloud/tea/app/skillhub/assets/source/ai-buddy-decouple/expert-profiles/tech-bug-troubleshooting.v20260625.avif';

    expect(resolvePresetAvatarImageSrc(avatar, {})).toBe(avatar);
    expect(isEmoji(avatar)).toBe(false);
  });

  test('supports mapped images, relative AVIF files, and strict emoji fallback', () => {
    expect(resolvePresetAvatarImageSrc('cowork.svg', { 'cowork.svg': '/assets/cowork.svg' })).toBe(
      '/assets/cowork.svg'
    );
    expect(resolvePresetAvatarImageSrc('avatars/bug.avif?revision=2', {})).toBe('avatars/bug.avif?revision=2');
    expect(resolvePresetAvatarImageSrc('🛠️', {})).toBeUndefined();
    expect(isEmoji('🛠️')).toBe(true);
    expect(isEmoji('👋🏽')).toBe(true);
    expect(isEmoji('🇨🇳')).toBe(true);
    expect(isEmoji('1️⃣')).toBe(true);
    expect(isEmoji('not-an-avatar')).toBe(false);
  });
});
