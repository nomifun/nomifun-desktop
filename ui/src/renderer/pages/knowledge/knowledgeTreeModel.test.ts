import { describe, expect, test } from 'bun:test';
import type {
  IKnowledgeEntryCapabilities,
  IKnowledgeFileEntry,
  IKnowledgeTreeEntry,
} from '@/common/adapter/ipcBridge';
import {
  parseKnowledgeEntryId,
  parseKnowledgeSourceId,
  parseKnowledgeSourceItemId,
} from '@/common/types/ids';
import {
  applyKnowledgePathRelocation,
  buildKnowledgeSearchTree,
  collectKnowledgeDirectoryPaths,
  initialKnowledgeTreeViewState,
  isKnowledgePathWithin,
  isNewerKnowledgeTreeRevision,
  knowledgeDirectoryOnlyTree,
  knowledgeFolderPathChain,
  knowledgeRelocationIssue,
  knowledgeRelocationPath,
  knowledgeTreeViewReducer,
  mergeKnowledgeTreeChildren,
  preserveKnowledgeTreeChildren,
  replaceKnowledgePathPrefix,
} from './KnowledgeDetailPage/treeModel';

const file = (rel_path: string): IKnowledgeFileEntry => ({
  rel_path,
  size: rel_path.length,
  modified_at: null,
});

const editableCapabilities: IKnowledgeEntryCapabilities = {
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

const node = (name: string, rel_path: string, is_dir: boolean): IKnowledgeTreeEntry => ({
  name,
  rel_path,
  is_dir,
  is_file: !is_dir,
  modified_at: null,
  capabilities: editableCapabilities,
  ...(is_dir ? {} : { size: rel_path.length }),
});

describe('knowledge detail tree model', () => {
  test('builds a search tree that keeps ancestor folders for matched documents', () => {
    const tree = buildKnowledgeSearchTree(
      [
        file('README.md'),
        file('raw/python3-type-conversion.md'),
        file('raw/string.md'),
        file('tutorials/overview.md'),
      ],
      'python3'
    );

    expect(tree).toEqual([
      {
        name: 'raw',
        rel_path: 'raw',
        is_dir: true,
        is_file: false,
        modified_at: null,
        children: [
          {
            name: 'python3-type-conversion.md',
            rel_path: 'raw/python3-type-conversion.md',
            is_dir: false,
            is_file: true,
            size: 'raw/python3-type-conversion.md'.length,
            modified_at: null,
          },
        ],
      },
    ]);
  });

  test('search preserves stable identity, revision, capabilities, and source metadata', () => {
    const entryId = parseKnowledgeEntryId('01912345-6789-7abc-8def-0123456789ab');
    const sourceFile: IKnowledgeFileEntry = {
      ...file('docs/captured.md'),
      entry_id: entryId,
      revision: 9,
      origin: 'url_snapshot',
      capabilities: {
        ...editableCapabilities,
        edit_content: false,
        remove_source: true,
        refresh_source: true,
        detach_source: true,
        copy_as_editable: true,
      },
      source: {
        source_id: parseKnowledgeSourceId('0190f5fe-7c00-7a00-8000-000000000705'),
        source_item_id: parseKnowledgeSourceItemId('0190f5fe-7c00-7a00-8000-000000000706'),
        source_url: 'https://example.com/docs',
        relationship: 'managed',
        sync_status: 'synced',
      },
    };

    const tree = buildKnowledgeSearchTree([sourceFile], 'captured');
    const result = tree[0]?.children?.[0];

    expect(result?.entry_id).toBe(entryId);
    expect(result?.revision).toBe(9);
    expect(result?.capabilities?.edit_content).toBe(false);
    expect(result?.capabilities?.relocate).toBe(true);
    expect(result?.source?.relationship).toBe('managed');
    expect(result?.source?.source_url).toBe('https://example.com/docs');
  });

  test('merges lazy-loaded children into the matching directory only', () => {
    const root = [node('raw', 'raw', true), node('README.md', 'README.md', false)];
    const merged = mergeKnowledgeTreeChildren(root, 'raw', [
      node('python3-type-conversion.md', 'raw/python3-type-conversion.md', false),
    ]);

    expect(merged[0].children?.map((child) => child.rel_path)).toEqual(['raw/python3-type-conversion.md']);
    expect(merged[1].children).toBeUndefined();
  });

  test('preserves already loaded folder children when the root tree refreshes', () => {
    const previous = [
      {
        ...node('raw', 'raw', true),
        children: [node('python3-type-conversion.md', 'raw/python3-type-conversion.md', false)],
      },
    ];
    const refreshedRoot = [node('raw', 'raw', true), node('README.md', 'README.md', false)];

    const preserved = preserveKnowledgeTreeChildren(refreshedRoot, previous);

    expect(preserved[0].children?.map((child) => child.rel_path)).toEqual(['raw/python3-type-conversion.md']);
    expect(preserved[1].children).toBeUndefined();
  });

  test('builds a folder path chain for branch refresh and expansion', () => {
    expect(knowledgeFolderPathChain('raw/tutorials/deep')).toEqual(['raw', 'raw/tutorials', 'raw/tutorials/deep']);
    expect(knowledgeFolderPathChain('/raw//tutorials/')).toEqual(['raw', 'raw/tutorials']);
    expect(knowledgeFolderPathChain('')).toEqual([]);
  });

  test('builds directory-only picker data without the moving directory subtree', () => {
    const tree = [
      {
        ...node('docs', 'docs', true),
        children: [
          { ...node('drafts', 'docs/drafts', true), children: [] },
          node('guide.md', 'docs/guide.md', false),
        ],
      },
      node('archive', 'archive', true),
    ];

    const result = knowledgeDirectoryOnlyTree(tree, node('drafts', 'docs/drafts', true));

    expect(collectKnowledgeDirectoryPaths(result)).toEqual(['docs', 'archive']);
    expect(result[0].children).toEqual([]);
    expect(result[1].children).toBeUndefined();
  });

  test('detects and rewrites paths inside a renamed folder', () => {
    expect(isKnowledgePathWithin('raw/tutorials/topic.md', 'raw/tutorials')).toBe(true);
    expect(isKnowledgePathWithin('raw/tutorials', 'raw/tutorials')).toBe(true);
    expect(isKnowledgePathWithin('raw/tutorials-old/topic.md', 'raw/tutorials')).toBe(false);
    expect(replaceKnowledgePathPrefix('raw/tutorials/topic.md', 'raw/tutorials', 'wiki/tutorials')).toBe('wiki/tutorials/topic.md');
    expect(replaceKnowledgePathPrefix('raw/tutorials', 'raw/tutorials', 'wiki/tutorials')).toBe('wiki/tutorials');
    expect(replaceKnowledgePathPrefix(null, 'raw/tutorials', 'wiki/tutorials')).toBeNull();
  });

  test('rejects no-op, self, and descendant drops while allowing a real move', () => {
    expect(knowledgeRelocationPath('raw/topic.md', 'archive')).toBe('archive/topic.md');
    expect(knowledgeRelocationIssue('raw/topic.md', false, 'raw')).toBe('same-parent');
    expect(knowledgeRelocationIssue('raw', true, 'raw')).toBe('self');
    expect(knowledgeRelocationIssue('raw', true, 'raw/tutorials')).toBe('descendant');
    expect(knowledgeRelocationIssue('raw', true, 'archive')).toBeNull();
    expect(knowledgeRelocationIssue('raw/topic.md', false, '', 'renamed.md')).toBeNull();
  });

  test('deduplicates and rejects out-of-order tree-changed revisions per base', () => {
    expect(isNewerKnowledgeTreeRevision(null, 40)).toBe(true);
    expect(isNewerKnowledgeTreeRevision(40, 41)).toBe(true);
    expect(isNewerKnowledgeTreeRevision(40, 40)).toBe(false);
    expect(isNewerKnowledgeTreeRevision(40, 39)).toBe(false);
  });

  test('atomically remaps every path-backed view when a loaded folder moves', () => {
    const raw = {
      ...node('raw', 'raw', true),
      revision: 3,
      children: [
        {
          ...node('tutorials', 'raw/tutorials', true),
          revision: 7,
          children: [
            { ...node('topic.md', 'raw/tutorials/topic.md', false), revision: 11 },
          ],
        },
      ],
    };
    const state = {
      files: [file('raw/tutorials/topic.md'), file('README.md')],
      treeData: [raw, { ...node('archive', 'archive', true), children: [] }],
      expandedTreeKeys: ['raw', 'raw/tutorials'],
      selectedPath: 'raw/tutorials/topic.md',
      selectedTreeKey: 'raw/tutorials/topic.md',
      selectedFolderPath: 'raw/tutorials',
    };

    const relocated = applyKnowledgePathRelocation(state, 'raw/tutorials', 'archive/tutorials');

    expect(relocated.selectedPath).toBe('archive/tutorials/topic.md');
    expect(relocated.selectedTreeKey).toBe('archive/tutorials/topic.md');
    expect(relocated.selectedFolderPath).toBe('archive/tutorials');
    expect(relocated.expandedTreeKeys).toEqual(['raw', 'archive/tutorials', 'archive']);
    expect(relocated.files.map((entry) => entry.rel_path)).toEqual([
      'archive/tutorials/topic.md',
      'README.md',
    ]);
    expect(relocated.treeData[0].children).toEqual([]);
    expect(relocated.treeData[1].children?.[0].rel_path).toBe('archive/tutorials');
    expect(relocated.treeData[1].children?.[0].children?.[0].rel_path).toBe(
      'archive/tutorials/topic.md'
    );
    expect(relocated.treeData[1].children?.[0].revision).toBe(8);
    expect(relocated.treeData[1].children?.[0].children?.[0].revision).toBe(12);
    // The ancestor outside the moved subtree was not mutated.
    expect(relocated.treeData[0].revision).toBe(3);
  });

  test('keeps a lazy destination unloaded instead of hiding its existing children', () => {
    const state = {
      ...initialKnowledgeTreeViewState,
      files: [file('docs/guide.md')],
      treeData: [
        { ...node('docs', 'docs', true), children: [node('guide.md', 'docs/guide.md', false)] },
        node('archive', 'archive', true),
      ],
      expandedTreeKeys: ['docs'],
      selectedPath: 'docs/guide.md',
      selectedTreeKey: 'docs/guide.md',
      selectedFolderPath: 'docs',
    };

    const relocated = applyKnowledgePathRelocation(state, 'docs/guide.md', 'archive/guide.md');

    expect(relocated.treeData[0].children).toEqual([]);
    expect(relocated.treeData[1].children).toBeUndefined();
    expect(relocated.selectedFolderPath).toBe('archive');
    expect(relocated.expandedTreeKeys).toEqual(['docs', 'archive']);
  });

  test('preserves an independently selected folder when the open document moves', () => {
    const state = {
      ...initialKnowledgeTreeViewState,
      files: [file('docs/guide.md')],
      selectedPath: 'docs/guide.md',
      selectedTreeKey: 'archive',
      selectedFolderPath: 'archive',
    };

    const relocated = applyKnowledgePathRelocation(state, 'docs/guide.md', 'published/guide.md');

    expect(relocated.selectedPath).toBe('published/guide.md');
    expect(relocated.selectedTreeKey).toBe('archive');
    expect(relocated.selectedFolderPath).toBe('archive');
  });

  test('is idempotent when the same tree-changed relocation is delivered twice', () => {
    const state = {
      ...initialKnowledgeTreeViewState,
      files: [file('docs/guide.md')],
      treeData: [node('guide.md', 'docs/guide.md', false)],
      selectedPath: 'docs/guide.md',
      selectedTreeKey: 'docs/guide.md',
      selectedFolderPath: 'docs',
    };

    const once = applyKnowledgePathRelocation(state, 'docs/guide.md', 'archive/guide.md');
    const twice = applyKnowledgePathRelocation(once, 'docs/guide.md', 'archive/guide.md');

    expect(twice).toEqual(once);
  });

  test('keeps the active path across a racing remote snapshot until relocate mapping arrives', () => {
    const active = {
      ...initialKnowledgeTreeViewState,
      files: [file('drafts/live.md')],
      selectedPath: 'drafts/live.md',
      selectedTreeKey: 'drafts/live.md',
      selectedFolderPath: 'drafts',
    };

    const raced = knowledgeTreeViewReducer(active, {
      type: 'sync',
      files: [file('archive/live.md')],
      tree: [node('archive', 'archive', true)],
    });
    expect(raced.selectedPath).toBe('drafts/live.md');

    const resolved = knowledgeTreeViewReducer(raced, {
      type: 'relocated',
      oldPath: 'drafts/live.md',
      newPath: 'archive/live.md',
    });
    expect(resolved.selectedPath).toBe('archive/live.md');
    expect(resolved.selectedTreeKey).toBe('archive/live.md');
  });
});
