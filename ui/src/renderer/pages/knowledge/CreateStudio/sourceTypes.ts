/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { KnowledgeKind } from '../KnowledgeTagFilterBar';

/**
 * Kinds the create flow can be opened with: the persistent base kinds plus
 * 'feishu', which only exists as a disabled third-party placeholder in the
 * type rail / empty state (no backend integration).
 */
export type StudioInitialKind = KnowledgeKind | 'feishu';

/**
 * Extends the initial kinds with 'import' for the migration entry in the type
 * rail. 'import' is not a persistent base kind — it's a creation source path
 * only.
 */
export type StudioSourceType = StudioInitialKind | 'import';

export const FEISHU_KNOWLEDGE_CREATION_ENABLED = false;

export type StudioSourceConfigLike = {
  rootPath?: string | null;
};

export type StudioSourceConfigValidation =
  | { ok: true }
  | { ok: false; messageKey: 'knowledge.studio.feishuDisabled' | 'knowledge.studio.localFolderRequired' };

export function normalizeStudioInitialKind(initialKind?: StudioInitialKind): StudioSourceType {
  if (initialKind === 'feishu' && !FEISHU_KNOWLEDGE_CREATION_ENABLED) return 'blank';
  return initialKind ?? 'blank';
}

export function canSubmitStudioSourceType(sourceType: StudioSourceType): boolean {
  return sourceType !== 'feishu' || FEISHU_KNOWLEDGE_CREATION_ENABLED;
}

export function canSubmitStudioSourceConfig(
  sourceType: StudioSourceType,
  config: StudioSourceConfigLike = {}
): StudioSourceConfigValidation {
  if (!canSubmitStudioSourceType(sourceType)) {
    return { ok: false, messageKey: 'knowledge.studio.feishuDisabled' };
  }
  if (sourceType === 'local' && !config.rootPath?.trim()) {
    return { ok: false, messageKey: 'knowledge.studio.localFolderRequired' };
  }
  return { ok: true };
}
