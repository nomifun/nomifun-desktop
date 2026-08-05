/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** One run of snippet text; `hit` marks a full-text match highlight. */
export interface SnippetSegment {
  text: string;
  hit: boolean;
}

/**
 * Parse an FTS5 highlight snippet into plain-text segments.
 *
 * Whitelist parser: ONLY the `<b>` / `</b>` marker pair the backend's
 * `snippet()` call emits is interpreted; every other character — including any
 * other tag-looking text inside a memory — stays literal text, so stored
 * content can never inject markup into the list.
 */
export function parseSnippetSegments(snippet: string): SnippetSegment[] {
  const segments: SnippetSegment[] = [];
  let rest = snippet;
  let hit = false;
  while (rest.length > 0) {
    const marker = hit ? '</b>' : '<b>';
    const index = rest.indexOf(marker);
    if (index === -1) {
      segments.push({ text: rest, hit });
      break;
    }
    if (index > 0) {
      segments.push({ text: rest.slice(0, index), hit });
    }
    rest = rest.slice(index + marker.length);
    hit = !hit;
  }
  return segments.filter((segment) => segment.text.length > 0);
}
