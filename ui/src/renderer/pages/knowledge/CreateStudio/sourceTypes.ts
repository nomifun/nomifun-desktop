/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { KnowledgeKind } from '../KnowledgeTagFilterBar';

/**
 * Extends KnowledgeKind with 'import' for the migration entry in the type rail.
 * 'import' is not a persistent base kind — it's a creation source path only.
 */
export type StudioSourceType = KnowledgeKind | 'import';

export const FEISHU_KNOWLEDGE_CREATION_ENABLED = false;

export function normalizeStudioInitialKind(initialKind?: KnowledgeKind): StudioSourceType {
  if (initialKind === 'feishu' && !FEISHU_KNOWLEDGE_CREATION_ENABLED) return 'blank';
  return initialKind ?? 'blank';
}

export function canSubmitStudioSourceType(sourceType: StudioSourceType): boolean {
  return sourceType !== 'feishu' || FEISHU_KNOWLEDGE_CREATION_ENABLED;
}
