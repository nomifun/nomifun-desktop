/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { resolveLocaleKey } from '@/common/utils';
import { resolveExtensionAssetUrl } from '@/renderer/utils/platform';

export type LocalizedAgentIdentity = {
  id?: string;
  name: string;
  name_i18n?: Record<string, string>;
};

/** Resolve a human-readable Agent name without consulting a mutable catalog. */
export const resolveAgentDisplayName = (agent: LocalizedAgentIdentity, language: string): string => {
  const localeKey = resolveLocaleKey(language);
  return (
    agent.name_i18n?.[language]?.trim() ||
    agent.name_i18n?.[localeKey]?.trim() ||
    agent.name_i18n?.['en-US']?.trim() ||
    agent.name.trim() ||
    agent.id || 'Agent'
  );
};

/** Strictly recognize emoji so arbitrary identifiers and URLs never become visible text. */
export const isEmoji = (value: string): boolean => {
  if (!value) return false;
  const emojiRegex = /^(?:\p{Regional_Indicator}{2}|[0-9#*]\uFE0F?\u20E3|\p{Extended_Pictographic}\uFE0F?\p{Emoji_Modifier}?(?:\u200D\p{Extended_Pictographic}\uFE0F?\p{Emoji_Modifier}?)*(?:[\u{E0020}-\u{E007E}]+\u{E007F})?)$/u;
  return emojiRegex.test(value);
};

/**
 * Resolve an Agent avatar to an image source. Remote/CDN URLs, backend paths,
 * data images, and common local image filenames are images; unknown text is
 * deliberately rejected so callers can fall back to a product icon.
 */
export const resolveAgentAvatarImageSrc = (
  avatar: string | undefined,
  avatarImageMap: Record<string, string>
): string | undefined => {
  const value = avatar?.trim();
  if (!value) return undefined;

  const mapped = avatarImageMap[value];
  if (mapped) return mapped;

  const resolved = resolveExtensionAssetUrl(value) || value;
  const isImage =
    /\.(?:svg|png|jpe?g|webp|gif|avif|bmp|ico)(?:[?#].*)?$/i.test(resolved) ||
    /^(?:https?:|file:\/\/|data:image\/|\/)/i.test(resolved);
  return isImage ? resolved : undefined;
};
