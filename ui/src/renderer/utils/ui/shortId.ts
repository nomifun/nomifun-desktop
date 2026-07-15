/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * A canonical prefixed UUIDv7 entity id (`cron_<uuid-v7>`, `kb_<uuid-v7>`,
 * `conv_<uuid-v7>`, …).
 */
const PREFIXED_ID =
  /^[a-z][a-z0-9]{0,31}_([0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})$/;

/**
 * Human-scannable short form of a TEXT entity locator, shared by the list rows
 * and dropdowns that still surface a prefixed (cron/kb) id or a path target
 * inline.
 *
 * - Prefixed entity ids → show the final 12 UUID hex digits. UUIDv7's leading
 *   digits mostly encode time, so taking the leading characters is a poor
 *   discriminator for nearby creations. The complete id remains available via
 *   hover/copy affordances.
 * - Anything else (e.g. a workpath binding target) → the last path segment,
 *   capped so a long absolute path can't blow out the row.
 *
 * Replaces the per-call-site `id.replace(/^prefix_/, '').slice(0, 8)` snippets
 * (which hard-coded a single prefix and silently failed on others) and the two
 * sites that forgot to strip the prefix at all.
 */
export const shortSessionId = (value: string): string => {
  const match = PREFIXED_ID.exec(value);
  if (match) return match[1].slice(-12);
  const tail = value.split(/[\\/]/).filter(Boolean).pop() ?? value;
  return tail.length > 24 ? `…${tail.slice(-24)}` : tail;
};
