import type {
  IKnowledgeEntryCapabilities,
  IKnowledgeFileContent,
  IKnowledgeFileEntry,
  IKnowledgeTreeEntry,
} from '@/common/adapter/ipcBridge';

export type KnowledgeEntryWithPolicy = Pick<
  IKnowledgeTreeEntry,
  'capabilities' | 'source' | 'origin'
>;

export type KnowledgeEntryCapability = Exclude<
  keyof IKnowledgeEntryCapabilities,
  'read_only_reason'
>;

/**
 * Capability checks deliberately fail closed. Provenance and filesystem paths
 * explain an entry, but neither is an authorization signal.
 */
export function hasKnowledgeEntryCapability(
  entry: KnowledgeEntryWithPolicy | null | undefined,
  capability: KnowledgeEntryCapability
): boolean {
  return entry?.capabilities?.[capability] === true;
}

export function isManagedKnowledgeEntry(
  entry: KnowledgeEntryWithPolicy | null | undefined
): boolean {
  return entry?.source?.relationship === 'managed';
}

export function knowledgeEntryRestrictionReason(
  _entry: KnowledgeEntryWithPolicy | null | undefined,
  fallback: string
): string {
  // Backend reasons remain useful diagnostics, but user-facing copy is
  // localized by the surface that knows the current language and context.
  return fallback;
}

export function knowledgeTreeEntryFromFile(
  file: IKnowledgeFileEntry | IKnowledgeFileContent
): IKnowledgeTreeEntry {
  return {
    ...(file.entry_id ? { entry_id: file.entry_id } : {}),
    ...(file.revision != null ? { revision: file.revision } : {}),
    ...('parent_entry_id' in file && file.parent_entry_id
      ? { parent_entry_id: file.parent_entry_id }
      : {}),
    ...(file.origin ? { origin: file.origin } : {}),
    ...(file.capabilities ? { capabilities: file.capabilities } : {}),
    ...(file.source ? { source: file.source } : {}),
    name: file.rel_path.split('/').filter(Boolean).at(-1) ?? file.rel_path,
    rel_path: file.rel_path,
    is_dir: false,
    is_file: true,
    size: file.size,
    modified_at: file.modified_at,
  };
}
