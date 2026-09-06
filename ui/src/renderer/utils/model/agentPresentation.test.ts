/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  isEmoji,
  resolveAgentAvatarImageSrc,
  resolveAgentDisplayName,
} from './agentPresentation';

const preset = {
  id: 'agent-1',
  name: 'Bug troubleshooting',
  name_i18n: { 'zh-CN': 'Bug 排查', 'en-US': 'Bug troubleshooting' },
};

describe('agent presentation', () => {
  test('uses the configured localized name with canonical fallbacks', () => {
    expect(resolveAgentDisplayName(preset, 'zh-Hans')).toBe('Bug 排查');
    expect(resolveAgentDisplayName(preset, 'fr-FR')).toBe('Bug troubleshooting');
    expect(resolveAgentDisplayName({ ...preset, name_i18n: {}, name: '  Custom name  ' }, 'zh-CN')).toBe(
      'Custom name'
    );
  });

  test('classifies the reported CDN AVIF avatar as an image, never emoji text', () => {
    const avatar =
      'https://cloudcache.tencent-cloud.com/qcloud/tea/app/skillhub/assets/source/ai-buddy-decouple/expert-profiles/tech-bug-troubleshooting.v20260625.avif';

    expect(resolveAgentAvatarImageSrc(avatar, {})).toBe(avatar);
    expect(isEmoji(avatar)).toBe(false);
  });

  test('supports mapped images, relative AVIF files, and strict emoji fallback', () => {
    expect(resolveAgentAvatarImageSrc('cowork.svg', { 'cowork.svg': '/assets/cowork.svg' })).toBe(
      '/assets/cowork.svg'
    );
    expect(resolveAgentAvatarImageSrc('avatars/bug.avif?revision=2', {})).toBe('avatars/bug.avif?revision=2');
    expect(resolveAgentAvatarImageSrc('🛠️', {})).toBeUndefined();
    expect(isEmoji('🛠️')).toBe(true);
    expect(isEmoji('👋🏽')).toBe(true);
    expect(isEmoji('🇨🇳')).toBe(true);
    expect(isEmoji('1️⃣')).toBe(true);
    expect(isEmoji('not-an-avatar')).toBe(false);
  });
});
