import { describe, expect, test } from 'bun:test';
import type { IKnowledgeTreeEntry } from '@/common/adapter/ipcBridge';
import { parseKnowledgeEntryId } from '@/common/types/ids';
import {
  knowledgeDragData,
  knowledgeDragEntryFromData,
  knowledgeDropData,
  knowledgeDropTargetFromData,
  knowledgeRootDropData,
  knowledgeTreeDragId,
  knowledgeTreeDropId,
  resolveKnowledgeDrop,
} from './KnowledgeDetailPage/knowledgeTreeDndModel';

const entry = (
  name: string,
  rel_path: string,
  is_dir: boolean,
  entry_id?: IKnowledgeTreeEntry['entry_id']
): IKnowledgeTreeEntry => ({
  name,
  rel_path,
  is_dir,
  is_file: !is_dir,
  modified_at: null,
  ...(entry_id ? { entry_id } : {}),
});

describe('knowledge tree drag-and-drop model', () => {
  test('uses stable entry identity for DnD ids with a path fallback', () => {
    const stableId = parseKnowledgeEntryId('01912345-6789-7abc-8def-0123456789ab');
    const stable = entry('guide.md', 'docs/guide.md', false, stableId);
    const legacy = entry('guide.md', 'docs/guide.md', false);

    expect(knowledgeTreeDragId(stable)).toBe(`knowledge-tree-drag:${stableId}`);
    expect(knowledgeTreeDropId(stable)).toBe(`knowledge-tree-drop:${stableId}`);
    expect(knowledgeTreeDragId(legacy)).toBe('knowledge-tree-drag:docs/guide.md');
  });

  test('round-trips valid DnD payloads and rejects malformed external payloads', () => {
    const source = entry('guide.md', 'docs/guide.md', false);
    const destination = entry('archive', 'archive', true);

    expect(knowledgeDragEntryFromData(knowledgeDragData(source))).toEqual(source);
    expect(knowledgeDropTargetFromData(knowledgeDropData(destination))).toEqual({
      accepts: true,
      destinationParentPath: 'archive',
      entry: destination,
    });
    expect(knowledgeDragEntryFromData({ entry: { rel_path: 'broken' } })).toBeNull();
    expect(knowledgeDropTargetFromData({ accepts: true, destinationParentPath: 42 })).toBeNull();
  });

  test('accepts directory and root targets but rejects files, no-ops, and descendants', () => {
    const sourceFile = entry('guide.md', 'docs/guide.md', false);
    const sourceDirectory = entry('docs', 'docs', true);

    expect(resolveKnowledgeDrop(sourceFile, knowledgeRootDropData())).toEqual({
      accepted: true,
      destinationParentPath: '',
    });
    expect(
      resolveKnowledgeDrop(sourceFile, knowledgeDropData(entry('archive', 'archive', true)))
    ).toEqual({ accepted: true, destinationParentPath: 'archive' });
    expect(
      resolveKnowledgeDrop(sourceFile, knowledgeDropData(entry('README.md', 'README.md', false)))
    ).toEqual({ accepted: false, issue: 'invalid-target' });
    expect(
      resolveKnowledgeDrop(sourceFile, knowledgeDropData(entry('docs', 'docs', true)))
    ).toEqual({ accepted: false, issue: 'same-parent' });
    expect(
      resolveKnowledgeDrop(
        sourceDirectory,
        knowledgeDropData(entry('nested', 'docs/nested', true))
      )
    ).toEqual({ accepted: false, issue: 'descendant' });
  });
});
