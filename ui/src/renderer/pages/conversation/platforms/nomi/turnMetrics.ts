/**
 * Pure formatters for the per-turn metrics chip (token cost + wall-clock
 * duration) shown after a nomi turn completes. Kept separate from React so the
 * formatting rules are unit-testable in isolation.
 */

/**
 * Compact token count: `942`, `1.2k`, `2.3m`. One decimal place at each
 * magnitude so the chip stays narrow while still conveying scale.
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

/**
 * Human wall-clock duration: `840ms`, `3.5s`, `1m 30s`. Sub-second in ms,
 * seconds with one decimal under a minute, `Xm Ys` at or above a minute.
 */
export function formatTurnDuration(elapsedMs: number): string {
  if (elapsedMs < 1000) {
    return `${elapsedMs}ms`;
  }
  if (elapsedMs < 60_000) {
    return `${(elapsedMs / 1000).toFixed(1)}s`;
  }
  const totalSeconds = Math.floor(elapsedMs / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}m ${seconds}s`;
}

export function calculateContextUsagePercent(used?: number, max?: number): number | null {
  if (used == null || max == null || max <= 0) {
    return null;
  }
  return Math.min(100, Math.max(0, Math.round((used / max) * 100)));
}

export function calculateCacheHitRatePercent({
  inputTokens,
  cacheReadTokens = 0,
}: {
  inputTokens?: number;
  cacheReadTokens?: number;
}): number | null {
  if (inputTokens == null || inputTokens <= 0) {
    return null;
  }
  return Math.max(0, Math.round((cacheReadTokens / inputTokens) * 100));
}

export function formatPercent(percent: number | null | undefined): string {
  return percent == null ? '—' : `${percent}%`;
}

export type ContextUsageSegments = {
  cachedTokens: number;
  freshTokens: number;
  remainingTokens: number;
  cachedPercent: number;
  freshPercent: number;
  remainingPercent: number;
};

export function calculateContextUsageSegments({
  contextTokens,
  contextWindow,
  cacheReadTokens = 0,
}: {
  contextTokens?: number;
  contextWindow?: number;
  cacheReadTokens?: number;
}): ContextUsageSegments | null {
  if (contextTokens == null || contextWindow == null || contextWindow <= 0) {
    return null;
  }

  const usedTokens = Math.min(Math.max(0, contextTokens), contextWindow);
  const cachedTokens = Math.min(Math.max(0, cacheReadTokens), usedTokens);
  const freshTokens = Math.max(0, usedTokens - cachedTokens);
  const remainingTokens = Math.max(0, contextWindow - usedTokens);

  return {
    cachedTokens,
    freshTokens,
    remainingTokens,
    cachedPercent: Math.round((cachedTokens / contextWindow) * 100),
    freshPercent: Math.round((freshTokens / contextWindow) * 100),
    remainingPercent: Math.round((remainingTokens / contextWindow) * 100),
  };
}
