/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ITerminalSession } from '@/common/adapter/ipcBridge';
import { DEFAULT_WORKPATH_KEY } from './workpathKey';
import { workpathKeyForTerminal } from './sessionWorkpath';

const term = (o: Partial<ITerminalSession>): Pick<ITerminalSession, 'cwd' | 'is_default_workpath'> => ({
  cwd: o.cwd ?? '/w',
  is_default_workpath: o.is_default_workpath,
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
