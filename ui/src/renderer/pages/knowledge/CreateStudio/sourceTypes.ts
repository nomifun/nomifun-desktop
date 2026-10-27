/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { KnowledgeKind } from '../KnowledgeTagFilterBar';

/** Kinds the create flow can be opened with — includes 'feishu' (a managed
 * base whose curated Feishu links index as metadata stubs). */
export type StudioInitialKind = KnowledgeKind;

/**
 * Extends the initial kinds with 'import' for the migration entry in the type
 * rail. 'import' is not a persistent base kind — it's a creation source path
 * only.
 */
export type StudioSourceType = StudioInitialKind | 'import';

export type StudioSourceConfigLike = {
  rootPath?: string | null;
};

export type StudioSourceConfigValidation =
  | { ok: true }
  | { ok: false; messageKey: 'knowledge.studio.localFolderRequired' };

export function normalizeStudioInitialKind(initialKind?: StudioInitialKind): StudioSourceType {
  return initialKind ?? 'blank';
}

export function canSubmitStudioSourceConfig(
  sourceType: StudioSourceType,
  config: StudioSourceConfigLike = {}
): StudioSourceConfigValidation {
  if (sourceType === 'local' && !config.rootPath?.trim()) {
    return { ok: false, messageKey: 'knowledge.studio.localFolderRequired' };
  }
  return { ok: true };
}
