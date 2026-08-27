import { describe, expect, test } from 'bun:test';
import type {
  IKnowledgeEntryCapabilities,
  IKnowledgeTreeEntry,
} from '@/common/adapter/ipcBridge';
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

const capabilities: IKnowledgeEntryCapabilities = {
  read_content: true,
  edit_content: true,
  rename: true,
  relocate: true,
  accept_children: true,
  delete_entry: true,
  remove_source: false,
  refresh_source: false,
  detach_source: false,
  copy_as_editable: false,
  export_entry: true,
  edit_metadata: true,
};

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
  capabilities,
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

  test('uses accept_children capability instead of provenance for drop targets', () => {
    const source = {
      ...entry('snapshot.md', 'snapshots/snapshot.md', false),
      origin: 'url_snapshot' as const,
      capabilities: { ...capabilities, edit_content: false, relocate: true },
    };
    const restrictedDirectory = {
      ...entry('locked', 'locked', true),
      capabilities: { ...capabilities, accept_children: false },
    };

    expect(resolveKnowledgeDrop(source, knowledgeRootDropData())).toEqual({
      accepted: true,
      destinationParentPath: '',
    });
    expect(resolveKnowledgeDrop(source, knowledgeDropData(restrictedDirectory))).toEqual({
      accepted: false,
      issue: 'invalid-target',
    });
  });
});
