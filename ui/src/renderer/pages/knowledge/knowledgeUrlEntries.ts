/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IKnowledgeSourceEntry } from '@/common/adapter/ipcBridge';

export const MAX_KNOWLEDGE_SOURCE_ENTRIES = 16;

export interface KnowledgeUrlDraft {
  url: string;
  title: string;
}

export type ParseKnowledgeUrlDraftsResult =
  | { ok: true; entries: IKnowledgeSourceEntry[] }
  | { ok: false; reason: 'empty' | 'invalid' | 'duplicate' | 'limit'; url?: string; limit?: number };

/**
 * One normalization and validation boundary shared by create-time URL sources
 * and post-creation web imports. The backend validates again; doing it here
 * keeps form feedback immediate and identical in both surfaces.
 */
export function parseKnowledgeUrlDrafts(
  drafts: KnowledgeUrlDraft[],
  rendered: boolean,
  limit = MAX_KNOWLEDGE_SOURCE_ENTRIES,
): ParseKnowledgeUrlDraftsResult {
  const nonEmpty = drafts.filter((draft) => draft.url.trim().length > 0);
  if (nonEmpty.length === 0) return { ok: false, reason: 'empty' };
  if (nonEmpty.length > limit) return { ok: false, reason: 'limit', limit };

  const identities = new Set<string>();
  const entries: IKnowledgeSourceEntry[] = [];
  for (const draft of nonEmpty) {
    const rawUrl = draft.url.trim();
    let parsed: URL;
    try {
      parsed = new URL(rawUrl);
    } catch {
      return { ok: false, reason: 'invalid', url: rawUrl };
    }
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      return { ok: false, reason: 'invalid', url: rawUrl };
    }
    if (parsed.username || parsed.password) {
      return { ok: false, reason: 'invalid', url: rawUrl };
    }
    parsed.hash = '';
    const identity = parsed.toString();
    if (identities.has(identity)) {
      return { ok: false, reason: 'duplicate', url: rawUrl };
    }
    identities.add(identity);
    entries.push({
      url: rawUrl,
      title: draft.title.trim() || undefined,
      rendered: rendered || undefined,
    });
  }
  return { ok: true, entries };
}
