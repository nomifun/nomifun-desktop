/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';

import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import type {
  IKnowledgeEntryCapabilities,
  IKnowledgeTreeEntry,
} from '@/common/adapter/ipcBridge';
import {
  KnowledgeTreeDnd,
  KnowledgeTreeDndHandle,
  KnowledgeTreeDndRow,
} from './KnowledgeTreeDnd';

afterEach(cleanup);

const capabilities: IKnowledgeEntryCapabilities = {
  read_content: true,
  edit_content: true,
  rename: true,
  relocate: true,
  accept_children: false,
  delete_entry: true,
  remove_source: false,
  refresh_source: false,
  detach_source: false,
  copy_as_editable: false,
  export_entry: true,
  edit_metadata: true,
};

const fileEntry: IKnowledgeTreeEntry = {
  name: 'guide.md',
  rel_path: 'docs/guide.md',
  is_dir: false,
  is_file: true,
  modified_at: null,
  capabilities,
};

describe('KnowledgeTreeDnd', () => {
  test('exposes a keyboard activator without nesting the row action button inside it', () => {
    const { getByLabelText, getByRole } = render(
      <KnowledgeTreeDnd
        disabled={false}
        expandedDirectoryPaths={[]}
        labels={{
          dropHint: 'Drop on a folder',
          invalidTarget: 'Invalid target',
          rootFolder: 'Root',
          describeIssue: (issue) => issue,
          moveTo: (folder) => `Move to ${folder}`,
        }}
        onExpandDirectory={() => undefined}
        onInvalidDrop={() => undefined}
        onLoadDirectory={() => Promise.resolve()}
        onLoadError={() => undefined}
        onRelocate={() => undefined}
      >
        <KnowledgeTreeDndRow item={fileEntry}>
          <KnowledgeTreeDndHandle aria-label='Move guide.md'>guide.md</KnowledgeTreeDndHandle>
          <button type='button'>More actions</button>
        </KnowledgeTreeDndRow>
      </KnowledgeTreeDnd>
    );

    const handle = getByLabelText('Move guide.md');
    const action = getByRole('button', { name: 'More actions' });
    expect(handle.getAttribute('role')).toBe('button');
    expect(handle.getAttribute('tabindex')).toBe('0');
    expect(handle.getAttribute('aria-roledescription')).toBe('draggable');
    expect(handle.contains(action)).toBe(false);
    expect(handle.closest('[data-knowledge-path]')?.getAttribute('data-knowledge-path')).toBe(
      'docs/guide.md'
    );
  });

  test('allows a managed snapshot to drag when the server grants relocate', () => {
    const snapshot: IKnowledgeTreeEntry = {
      ...fileEntry,
      name: 'captured.md',
      rel_path: 'snapshots/captured.md',
      origin: 'url_snapshot',
      capabilities: { ...capabilities, edit_content: false, relocate: true },
    };
    const { getByLabelText } = render(
      <KnowledgeTreeDnd
        disabled={false}
        expandedDirectoryPaths={[]}
        labels={{
          dropHint: 'Drop on a folder',
          invalidTarget: 'Invalid target',
          rootFolder: 'Root',
          describeIssue: (issue) => issue,
          moveTo: (folder) => `Move to ${folder}`,
        }}
        onExpandDirectory={() => undefined}
        onInvalidDrop={() => undefined}
        onLoadDirectory={() => Promise.resolve()}
        onLoadError={() => undefined}
        onRelocate={() => undefined}
      >
        <KnowledgeTreeDndRow item={snapshot}>
          <KnowledgeTreeDndHandle aria-label='Move captured.md'>captured.md</KnowledgeTreeDndHandle>
        </KnowledgeTreeDndRow>
      </KnowledgeTreeDnd>
    );

    expect(getByLabelText('Move captured.md').getAttribute('aria-roledescription')).toBe(
      'draggable'
    );
  });
});
