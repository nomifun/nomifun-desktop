/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { parseSnippetSegments } from './memorySnippet';

const source = readFileSync(new URL('./MemoriesTab.tsx', import.meta.url), 'utf8');

describe('desktop companion memories pagination', () => {
  test('requests 10 memories by default and renders a controllable pager', () => {
    expect(source.includes('const [page, setPage] = useState(1);')).toBe(true);
    expect(source.includes('const [pageSize, setPageSize] = useState(10);')).toBe(true);
    expect(source.includes('limit: pageSize')).toBe(true);
    expect(source.includes('offset: (page - 1) * pageSize')).toBe(true);
    expect(source.includes('<Pagination')).toBe(true);
    expect(source.includes('sizeCanChange')).toBe(true);
    expect(source.includes('sizeOptions={[10, 20, 50]}')).toBe(true);
  });

  test('keeps memory content prominent and groups secondary actions in a menu', () => {
    expect(source.includes('line-clamp-2')).toBe(true);
    expect(source.includes('<Dropdown')).toBe(true);
    expect(source.includes('<More')).toBe(true);
  });
});

describe('batch operations', () => {
  test('rows are checkbox-selectable and a toolbar drives the batch endpoint', () => {
    expect(source.includes('const [selected, setSelected] = useState<CompanionMemoryId[]>([]);')).toBe(true);
    expect(source.includes('ipcBridge.companion.batchMemories.invoke({ ids: selected, action, kind: batchKind })')).toBe(true);
    // Destructive batch ops go through an explicit confirm.
    expect(source.includes('Modal.confirm({')).toBe(true);
    // All four actions are reachable from the toolbar.
    expect(source.includes("confirmBatch('archive')")).toBe(true);
    expect(source.includes("confirmBatch('restore')")).toBe(true);
    expect(source.includes("confirmBatch('delete')")).toBe(true);
    expect(source.includes("runBatch('reclassify', reclassifyKind)")).toBe(true);
  });

  test('selection resets when the result set changes', () => {
    expect(source.includes('setSelected([]);\n  }, [kind, q, memStatus, sort, scopeMode, companionId, page, pageSize]);')).toBe(true);
  });
});

describe('archive browsing and sorting', () => {
  test('status is a segmented control, not a dropdown, with inline restore for archived rows', () => {
    expect(source.includes("value={memStatus} onChange={(v: 'active' | 'archived') => setMemStatus(v)}")).toBe(true);
    expect(source.includes("{m.status === 'archived' && (")).toBe(true);
  });

  test('a sort selector feeds the list request', () => {
    expect(source.includes("useState<ICompanionMemorySort>('relevance')")).toBe(true);
    expect(source.includes('sort,\n        limit: pageSize')).toBe(true);
    expect(source.includes("<Select.Option value='relevance'>")).toBe(true);
    expect(source.includes("<Select.Option value='time'>")).toBe(true);
    expect(source.includes("<Select.Option value='importance'>")).toBe(true);
  });
});

describe('merge assistant', () => {
  test('a drawer lists duplicate groups and confirms merges per group', () => {
    expect(source.includes('<Drawer')).toBe(true);
    expect(source.includes('ipcBridge.companion.memoryMergeSuggestions.invoke()')).toBe(true);
    expect(source.includes('ipcBridge.companion.mergeMemories.invoke({')).toBe(true);
    // A merge needs at least two members and a non-empty merged text.
    expect(source.includes('draft.ids.length < 2 || !draft.content.trim()')).toBe(true);
  });
});

describe('snippet highlighting', () => {
  test('memory content renders parsed snippet segments, never raw HTML', () => {
    expect(source.includes('parseSnippetSegments(m.snippet)')).toBe(true);
    expect(source.includes('dangerouslySetInnerHTML')).toBe(false);
  });

  test('parseSnippetSegments interprets only the <b> marker pair', () => {
    expect(parseSnippetSegments('主人喜欢<b>咖啡</b>豆')).toEqual([
      { text: '主人喜欢', hit: false },
      { text: '咖啡', hit: true },
      { text: '豆', hit: false },
    ]);
    // Other tag-looking text stays literal — nothing else is interpreted.
    expect(parseSnippetSegments('<script>x</script>')).toEqual([{ text: '<script>x</script>', hit: false }]);
    // Unterminated markers do not lose text.
    expect(parseSnippetSegments('…<b>结尾')).toEqual([
      { text: '…', hit: false },
      { text: '结尾', hit: true },
    ]);
    expect(parseSnippetSegments('')).toEqual([]);
  });
});
