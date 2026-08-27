/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';

import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import type { IKnowledgeTreeEntry } from '@/common/adapter/ipcBridge';
import {
  KnowledgeTreeDnd,
  KnowledgeTreeDndHandle,
  KnowledgeTreeDndRow,
} from './KnowledgeTreeDnd';

afterEach(cleanup);

const fileEntry: IKnowledgeTreeEntry = {
  name: 'guide.md',
  rel_path: 'docs/guide.md',
  is_dir: false,
  is_file: true,
  modified_at: null,
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
});
