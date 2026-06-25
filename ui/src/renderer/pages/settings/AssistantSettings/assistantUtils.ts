import { resolveExtensionAssetUrl } from '@/renderer/utils/platform';
import type { AssistantListItem } from './types';

/**
 * Check if a string is an emoji (simple check for common emoji patterns).
 */
export const isEmoji = (str: string): boolean => {
  if (!str) return false;
  const emojiRegex = /^(?:\p{Emoji_Presentation}|\p{Emoji}️)(?:‍(?:\p{Emoji_Presentation}|\p{Emoji}️))*$/u;
  return emojiRegex.test(str);
};

/**
 * Resolve an avatar string to an image src URL, or undefined if it is not an image.
 */
export const resolveAvatarImageSrc = (
  avatar: string | undefined,
  avatarImageMap: Record<string, string>
): string | undefined => {
  const value = avatar?.trim();
  if (!value) return undefined;

  const mapped = avatarImageMap[value];
  if (mapped) return mapped;

  const resolved = resolveExtensionAssetUrl(value) || value;
  const isImage = /\.(svg|png|jpe?g|webp|gif)$/i.test(resolved) || /^(https?:|file:\/\/|data:|\/)/i.test(resolved);
  return isImage ? resolved : undefined;
};

/**
 * Sort assistants by sortOrder. The backend already returns sorted lists; this
 * is a deterministic fallback for local reorder operations.
 */
export const sortAssistants = (list: AssistantListItem[]): AssistantListItem[] =>
  [...list].toSorted((a, b) => a.sort_order - b.sort_order);

/** Selected tag keys per dimension. Empty array = no constraint on that dimension. */
export type TagFilterState = { audience: string[]; scenario: string[] };

/**
 * Faceted filter: search text (name + description) AND audience-facet AND
 * scenario-facet. Within a facet, an assistant matches if it carries ANY of
 * the selected keys (OR). Empty facet = no constraint.
 */
export const filterAssistantsByTags = (
  assistants: AssistantListItem[],
  query: string,
  tagFilter: TagFilterState,
  localeKey: string
): AssistantListItem[] => {
  const normalizedQuery = query.trim().toLowerCase();
  const matchesFacet = (have: string[] | undefined, selected: string[]) =>
    selected.length === 0 || (have ?? []).some((k) => selected.includes(k));

  return assistants.filter((assistant) => {
    if (normalizedQuery) {
      const searchableText = [
        assistant.name_i18n?.[localeKey] || assistant.name,
        assistant.description_i18n?.[localeKey] || assistant.description || '',
      ]
        .join(' ')
        .toLowerCase();
      if (!searchableText.includes(normalizedQuery)) return false;
    }
    return (
      matchesFacet(assistant.audience_tags, tagFilter.audience) &&
      matchesFacet(assistant.scenario_tags, tagFilter.scenario)
    );
  });
};
