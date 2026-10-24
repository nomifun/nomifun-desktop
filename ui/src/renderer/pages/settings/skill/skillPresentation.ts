/** Shared stable selectors for installed and market skill cards. */

/** Normalize a skill name for use in a stable data-testid. */
export const normalizeTestId = (name: string): string => name.replace(/[:/\s<>"'|?*]/g, '-');
