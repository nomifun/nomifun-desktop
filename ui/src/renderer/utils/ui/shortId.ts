/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * A prefixed entity id (`cron_…`, `kb_…`): a lowercase prefix, an underscore,
 * then the 16-char base32 short-id body.
 *
 * NOTE: conversations / requirements / terminal sessions are no longer prefixed
 * TEXT ids — they are INTEGER primary keys rendered directly as `#N` at each
 * call site (ConversationRow / TerminalRow / renderConversationOption /
 * RequirementsListPage / TagSessionTab). This helper now serves only the
 * entities that remain TEXT short ids (cron jobs `cron_`, knowledge bases `kb_`,
 * and workpath / path-style binding targets). Do not route integer ids through
 * it — `#${id}` is the canonical display for those.
 */
const PREFIXED_ID = /^[a-z]+_([0-9a-z]{8,})$/i;

/**
 * Human-scannable short form of a TEXT entity locator, shared by the list rows
 * and dropdowns that still surface a prefixed (cron/kb) id or a path target
 * inline.
 *
 * - Prefixed entity ids → drop the `{prefix}_` and return the full short-id
 *   body. The body is only 16 chars (a sortable base32 short id, see
 *   `prefixedId`), so there is no need to truncate — and truncating would be
 *   actively misleading, because the leading chars encode the timestamp and
 *   are shared by everything created in the same window. The complete id is
 *   always one hover / copy-button away.
 * - Anything else (e.g. a workpath binding target) → the last path segment,
 *   capped so a long absolute path can't blow out the row.
 *
 * Replaces the per-call-site `id.replace(/^prefix_/, '').slice(0, 8)` snippets
 * (which hard-coded a single prefix and silently failed on others) and the two
 * sites that forgot to strip the prefix at all.
 */
export const shortSessionId = (value: string): string => {
  const match = PREFIXED_ID.exec(value);
  if (match) return match[1];
  const tail = value.split(/[\\/]/).filter(Boolean).pop() ?? value;
  return tail.length > 24 ? `…${tail.slice(-24)}` : tail;
};
