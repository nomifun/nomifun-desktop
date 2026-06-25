/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ITerminalSession } from '@/common/adapter/ipcBridge';
import { DEFAULT_WORKPATH_KEY } from './workpathKey';
import { workpathKeyForConversation, workpathKeyForDraftDir, workpathKeyForTerminal } from './sessionWorkpath';

const term = (o: Partial<ITerminalSession>): Pick<ITerminalSession, 'cwd' | 'is_default_workpath'> => ({
  cwd: o.cwd ?? '/w',
  is_default_workpath: o.is_default_workpath,
});

describe('workpathKeyForConversation', () => {
  test('custom_workspace + workspace string → workpathKey(workspace)，尾斜杠归一', () => {
    expect(workpathKeyForConversation({ custom_workspace: true, workspace: '/w/p1/' })).toBe('/w/p1');
  });
  test('未标 custom_workspace（即便有 workspace）→ default', () => {
    expect(workpathKeyForConversation({ workspace: '/w/p1' })).toBe(DEFAULT_WORKPATH_KEY);
  });
  test('custom_workspace 但 workspace 非字符串 → default', () => {
    expect(workpathKeyForConversation({ custom_workspace: true })).toBe(DEFAULT_WORKPATH_KEY);
  });
  test('空/缺失 extra → default', () => {
    expect(workpathKeyForConversation(undefined)).toBe(DEFAULT_WORKPATH_KEY);
    expect(workpathKeyForConversation(null)).toBe(DEFAULT_WORKPATH_KEY);
    expect(workpathKeyForConversation({})).toBe(DEFAULT_WORKPATH_KEY);
  });
});

describe('workpathKeyForTerminal', () => {
  test('is_default_workpath === true → default（即便 cwd 有值）', () => {
    expect(workpathKeyForTerminal(term({ cwd: '/w/p1', is_default_workpath: true }))).toBe(DEFAULT_WORKPATH_KEY);
  });
  test('非默认 → workpathKey(cwd)，反斜杠/尾斜杠归一', () => {
    expect(workpathKeyForTerminal(term({ cwd: 'C:\\w\\p1\\', is_default_workpath: false }))).toBe('C:/w/p1');
  });
  test('is_default_workpath 缺省（undefined）按非默认处理', () => {
    expect(workpathKeyForTerminal(term({ cwd: '/w/p2' }))).toBe('/w/p2');
  });
});

describe('workpathKeyForDraftDir', () => {
  test('选了目录 → workpathKey(dir)', () => {
    expect(workpathKeyForDraftDir('/w/p1/')).toBe('/w/p1');
  });
  test('空字符串/空白/未选 → default', () => {
    expect(workpathKeyForDraftDir('')).toBe(DEFAULT_WORKPATH_KEY);
    expect(workpathKeyForDraftDir('   ')).toBe(DEFAULT_WORKPATH_KEY);
    expect(workpathKeyForDraftDir(undefined)).toBe(DEFAULT_WORKPATH_KEY);
    expect(workpathKeyForDraftDir(null)).toBe(DEFAULT_WORKPATH_KEY);
  });
});
