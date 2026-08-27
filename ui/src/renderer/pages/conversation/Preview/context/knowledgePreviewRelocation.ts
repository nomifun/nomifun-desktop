import type { KnowledgeBaseId, KnowledgeEntryId } from '@/common/types/ids';

import type { PreviewTab } from './PreviewContext';

export interface KnowledgePreviewResource {
  kind: 'knowledge-document';
  knowledge_base_id: KnowledgeBaseId;
  /** Stable projection identity when the backend has reconciled this entry. */
  entry_id?: KnowledgeEntryId;
  rel_path: string;
}

export interface KnowledgePreviewRelocation {
  knowledge_base_id: KnowledgeBaseId;
  old_prefix: string;
  new_prefix: string;
}

export function replaceKnowledgePreviewPathPrefix(
  path: string,
  oldPrefix: string,
  newPrefix: string,
): string {
  if (path === oldPrefix) return newPrefix;
  if (path.startsWith(`${oldPrefix}/`)) return `${newPrefix}${path.slice(oldPrefix.length)}`;
  return path;
}

function absoluteKnowledgePath(workspace: string | undefined, relPath: string): string | undefined {
  if (!workspace) return undefined;
  return `${workspace.replace(/[\\/]+$/, '')}/${relPath}`;
}

function basename(path: string): string {
  return path.split('/').filter(Boolean).at(-1) ?? path;
}

/**
 * Rebind persisted preview tabs after a knowledge entry moves. Content and dirty
 * state are deliberately preserved: a namespace relocation changes the file's
 * locator, not the document session that the user already has open.
 */
export function relocateKnowledgePreviewTabs(
  tabs: PreviewTab[],
  change: KnowledgePreviewRelocation,
): PreviewTab[] {
  let changed = false;
  const next = tabs.map((tab) => {
    const resource = tab.metadata?.knowledge_resource;
    if (!resource || resource.knowledge_base_id !== change.knowledge_base_id) return tab;

    const nextRelPath = replaceKnowledgePreviewPathPrefix(
      resource.rel_path,
      change.old_prefix,
      change.new_prefix,
    );
    if (nextRelPath === resource.rel_path) return tab;

    changed = true;
    const oldName = basename(resource.rel_path);
    const nextName = basename(nextRelPath);
    const nextTitle = tab.title.endsWith(oldName)
      ? `${tab.title.slice(0, -oldName.length)}${nextName}`
      : tab.title;

    return {
      ...tab,
      title: nextTitle,
      metadata: {
        ...tab.metadata,
        title: tab.metadata?.title?.endsWith(oldName)
          ? `${tab.metadata.title.slice(0, -oldName.length)}${nextName}`
          : tab.metadata?.title,
        file_name: nextName,
        file_path: absoluteKnowledgePath(tab.metadata?.workspace, nextRelPath),
        knowledge_resource: { ...resource, rel_path: nextRelPath },
      },
    };
  });
  return changed ? next : tabs;
}
