/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { CompanionId, KnowledgeBaseId } from '@/common/types/ids';

/**
 * Bundle plumbing for 其他 → 迁移: native file dialogs, filename defaults and
 * the knowledge-binding rebuild that follows a companion import. Kept out of
 * the components so the rows stay declarative.
 */

const ZIP_FILTERS = [{ name: 'Zip', extensions: ['zip'] }];

/** Tagged import result of POST /api/companion/import (backend `ImportOutcome`). */
export type ImportOutcome =
  | { kind: 'memory'; imported: number; skipped_duplicates: number }
  | { kind: 'companion'; companion_id: CompanionId; name: string; knowledge_names: string[] };

/** Native save dialog — desktop only, every caller is gated on `isTauriRuntime()`. */
export const pickSavePath = async (defaultName: string): Promise<string | null> => {
  const { save } = await import('@tauri-apps/plugin-dialog');
  return save({ defaultPath: defaultName, filters: ZIP_FILTERS });
};

/** Native open dialog via the existing ipcBridge dialog surface. */
export const pickZipPath = async (): Promise<string | null> => {
  const paths = await ipcBridge.dialog.showOpen.invoke({
    properties: ['openFile'],
    filters: ZIP_FILTERS,
  });
  return paths?.[0] ?? null;
};

/** `2026-06-11` — date suffix for default bundle filenames. */
const today = (): string => new Date().toISOString().slice(0, 10);

/** Companion name → filename-safe fragment. */
const safeName = (s: string): string =>
  s.replace(/[\\/:*?"<>|\s]+/g, '-').replace(/^-+|-+$/g, '') || 'companion';

/** `nomifun-companion-<name>-<date>.zip`. */
export const defaultBundleName = (companionName: string): string =>
  `nomifun-companion-${safeName(companionName)}-${today()}.zip`;

/** Backend 400 messages pass through verbatim; everything else falls back to String(e). */
export const errText = (e: unknown): string => {
  if (isBackendHttpError(e) && e.backendMessage) return e.backendMessage;
  return e instanceof Error ? e.message : String(e);
};

/**
 * Names of the knowledge bases bound to this companion. Per spec §4.8 the
 * frontend supplies them — the companion crate never reaches into the knowledge
 * domain. Any lookup failure degrades to "export without refs".
 */
export const collectKnowledgeNames = async (companionId: CompanionId): Promise<string[]> => {
  try {
    const [binding, bases] = await Promise.all([
      ipcBridge.knowledge.getBinding.invoke({ kind: 'companion', target_id: companionId }),
      ipcBridge.knowledge.listBases.invoke(),
    ]);
    const nameById = new Map(bases.map((b) => [b.knowledge_base_id, b.name]));
    return binding.kb_ids.map((id) => nameById.get(id)).filter((n): n is string => Boolean(n));
  } catch {
    return [];
  }
};

export interface RebuildResult {
  /** Bindings restored by name. */
  matched: number;
  /** Bundle names with no local base — the user must import those packs first. */
  unmatched: string[];
}

/**
 * Match an imported bundle's `knowledge_names` against local bases and rebuild
 * the new companion's binding. Names without a local base are reported back so
 * the caller can tell the user what to import by hand.
 */
export const rebuildKnowledgeBinding = async (
  outcome: Extract<ImportOutcome, { kind: 'companion' }>
): Promise<RebuildResult> => {
  if (!outcome.knowledge_names.length) return { matched: 0, unmatched: [] };
  const bases = await ipcBridge.knowledge.listBases.invoke();
  const idByName = new Map(bases.map((b) => [b.name, b.knowledge_base_id]));
  const matchedIds: KnowledgeBaseId[] = [];
  const unmatched: string[] = [];
  for (const name of outcome.knowledge_names) {
    const id = idByName.get(name);
    if (id) matchedIds.push(id);
    else unmatched.push(name);
  }
  if (matchedIds.length) {
    await ipcBridge.knowledge.setBinding.invoke({
      kind: 'companion',
      target_id: outcome.companion_id,
      enabled: true,
      writeback: false,
      writeback_mode: 'staged',
      writeback_eagerness: 'conservative',
      channel_write_enabled: false,
      kb_ids: matchedIds,
    });
  }
  return { matched: matchedIds.length, unmatched };
};
