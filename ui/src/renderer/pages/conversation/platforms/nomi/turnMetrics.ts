/**
 * Pure formatter for the context-usage ring beside the model selector.
 * Kept separate from React so the formatting rule is unit-testable.
 */

/**
 * Compact token count: `942`, `1.2k`, `2.3m`. One decimal place at each
 * magnitude so the ring stays narrow while still conveying scale.
 */
export function formatTokenCount(tokens: number): string {
  if (tokens < 1000) {
    return String(tokens);
  }
  if (tokens < 1_000_000) {
    return `${(tokens / 1000).toFixed(1)}k`;
  }
  return `${(tokens / 1_000_000).toFixed(1)}m`;
}
