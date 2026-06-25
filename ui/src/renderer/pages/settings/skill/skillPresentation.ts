/**
 * Shared presentation helpers for the Skills Hub. Extracted from the old
 * flat-list SkillsHubSettings so the card grid and the page share one source.
 */

/** Normalize a skill name for use in a stable data-testid. */
export const normalizeTestId = (name: string): string => name.replace(/[:/\s<>"'|?*]/g, '-');

/**
 * Deterministic letter-avatar color class keyed off the skill name. These
 * fixed hexes are an intentional, pre-existing exception to the theme-variable
 * rule (the avatar palette must stay legible across all themes); carried over
 * verbatim from the previous SkillsHubSettings implementation.
 */
export const getAvatarColorClass = (name: string): string => {
  if (!name) return 'bg-[var(--color-primary)] text-white';
  const colors = [
    'bg-[#F53F3F] text-white', // Red
    'bg-[#F77234] text-white', // Orange
    'bg-[#B8860B] text-white', // Gold
    'bg-[#F5319D] text-white', // Pink
    'bg-[#C41D7F] text-white', // Raspberry
    'bg-[#722ED1] text-white', // Purple
  ];
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = name.charCodeAt(i) + ((hash << 5) - hash);
  }
  return colors[Math.abs(hash) % colors.length];
};
