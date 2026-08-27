import type { IKnowledgeTreeEntry } from '@/common/adapter/ipcBridge';
import {
  knowledgeRelocationIssue,
  type KnowledgeRelocationIssue,
} from './treeModel';

export const KNOWLEDGE_ROOT_DROP_ID = 'knowledge-tree-drop:root';

export type KnowledgeDropTarget = {
  accepts: boolean;
  destinationParentPath: string;
  entry?: IKnowledgeTreeEntry;
};

export type KnowledgeDropDecision =
  | { accepted: true; destinationParentPath: string }
  | { accepted: false; issue: KnowledgeRelocationIssue | 'invalid-target' };

type KnowledgeDragData = {
  entry: IKnowledgeTreeEntry;
};

function isKnowledgeTreeEntry(value: unknown): value is IKnowledgeTreeEntry {
  if (!value || typeof value !== 'object') return false;
  const entry = value as Partial<IKnowledgeTreeEntry>;
  return (
    typeof entry.name === 'string' &&
    typeof entry.rel_path === 'string' &&
    typeof entry.is_dir === 'boolean' &&
    typeof entry.is_file === 'boolean'
  );
}

export function knowledgeTreeDragId(item: IKnowledgeTreeEntry): string {
  return `knowledge-tree-drag:${item.entry_id ?? item.rel_path}`;
}

export function knowledgeTreeDropId(item: IKnowledgeTreeEntry): string {
  return `knowledge-tree-drop:${item.entry_id ?? item.rel_path}`;
}

export function knowledgeDragData(entry: IKnowledgeTreeEntry): KnowledgeDragData {
  return { entry };
}

export function knowledgeDropData(entry: IKnowledgeTreeEntry): KnowledgeDropTarget {
  return {
    accepts: entry.is_dir,
    destinationParentPath: entry.is_dir ? entry.rel_path : '',
    entry,
  };
}

export function knowledgeRootDropData(): KnowledgeDropTarget {
  return { accepts: true, destinationParentPath: '' };
}

export function knowledgeDragEntryFromData(data: unknown): IKnowledgeTreeEntry | null {
  if (!data || typeof data !== 'object') return null;
  const entry = (data as Partial<KnowledgeDragData>).entry;
  return isKnowledgeTreeEntry(entry) ? entry : null;
}

export function knowledgeDropTargetFromData(data: unknown): KnowledgeDropTarget | null {
  if (!data || typeof data !== 'object') return null;
  const target = data as Partial<KnowledgeDropTarget>;
  if (
    typeof target.destinationParentPath !== 'string' ||
    typeof target.accepts !== 'boolean' ||
    (target.entry != null && !isKnowledgeTreeEntry(target.entry))
  ) {
    return null;
  }
  return {
    accepts: target.accepts,
    destinationParentPath: target.destinationParentPath,
    ...(target.entry ? { entry: target.entry } : {}),
  };
}

export function resolveKnowledgeDrop(
  source: IKnowledgeTreeEntry,
  target: KnowledgeDropTarget | null
): KnowledgeDropDecision {
  if (!target?.accepts) return { accepted: false, issue: 'invalid-target' };
  const issue = knowledgeRelocationIssue(
    source.rel_path,
    source.is_dir,
    target.destinationParentPath
  );
  return issue
    ? { accepted: false, issue }
    : { accepted: true, destinationParentPath: target.destinationParentPath };
}
