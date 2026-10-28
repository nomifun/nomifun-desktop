/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Knowledge-base level policy predicates shared by the list page and the
 * detail page settings tab. Kept in a dependency-free module so the rules can
 * be unit-tested without rendering.
 */
import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';

/**
 * True when deleting this base may offer the optional "also delete files on
 * disk" purge: the base directory is backend-managed AND its documents live
 * on disk. Feishu bases keep their documents in the cloud — purge would only
 * remove local index stubs, so the cascade-delete option stays hidden.
 */
export function canPurgeKnowledgeBase(
  base: Pick<IKnowledgeBase, 'managed' | 'kind'>
): boolean {
  return base.managed && base.kind !== 'feishu';
}
