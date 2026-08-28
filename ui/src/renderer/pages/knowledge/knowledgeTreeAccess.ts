import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';

export type KnowledgeTreeAccess = IKnowledgeBase['tree_access'];

/**
 * Selecting a local folder creates a file-management knowledge base. Read-only
 * indexing remains available, but it must be an explicit user choice rather
 * than the accidental result of an uninitialised form value.
 */
export const DEFAULT_LOCAL_FOLDER_TREE_ACCESS: KnowledgeTreeAccess = 'editable';

export function resolveLocalFolderTreeAccess(
  access: KnowledgeTreeAccess | undefined,
): KnowledgeTreeAccess {
  return access ?? DEFAULT_LOCAL_FOLDER_TREE_ACCESS;
}
