/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  formatFileTargetPreview,
  formatWorkspaceFileTarget,
  splitToolReceiptTargets,
} from './processFileTargetLabel';

const workspaceRoot = '/Users/muri/Library/Application Support/NomiFun/Nomi-dev/conversations/nomi-temp-39';

describe('process file target labels', () => {
  test('shows only the file name for absolute targets inside the current workspace', () => {
    const target = `${workspaceRoot}/snake.html`;

    expect(formatWorkspaceFileTarget(target, { workspaceRoots: [workspaceRoot] })).toEqual({
      label: 'snake.html',
      title: target,
      isWorkspaceTarget: true,
    });
  });

  test('keeps full absolute paths outside the current workspace', () => {
    const target = '/Users/muri/Desktop/snake.html';

    expect(formatWorkspaceFileTarget(target, { workspaceRoots: [workspaceRoot] })).toEqual({
      label: target,
      title: target,
      isWorkspaceTarget: false,
    });
  });

  test('uses file names for relative workspace targets', () => {
    expect(
      formatWorkspaceFileTarget('ui/src/renderer/pages/conversation/Messages/MessageList.tsx', {
        workspaceRoots: [workspaceRoot],
      }).label
    ).toBe('MessageList.tsx');
  });

  test('builds a compact preview from joined receipt targets', () => {
    const targets = splitToolReceiptTargets(`${workspaceRoot}/snake.html, ${workspaceRoot}/game.py`);

    expect(formatFileTargetPreview(targets, { workspaceRoots: [workspaceRoot] })).toBe('snake.html, game.py');
  });
});
