/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { describe, expect, test } from 'bun:test';
import { DEFAULT_WORKPATH_KEY, workpathKey } from './workpathKey';

describe('workpathKey', () => {
  test('去尾斜杠', () => {
    expect(workpathKey('/Users/a/proj/')).toBe('/Users/a/proj');
  });
  test('保留根路径', () => {
    expect(workpathKey('/')).toBe('/');
  });
  test('空白输入归 default', () => {
    expect(workpathKey('')).toBe(DEFAULT_WORKPATH_KEY);
    expect(workpathKey('   ')).toBe(DEFAULT_WORKPATH_KEY);
    expect(workpathKey(undefined)).toBe(DEFAULT_WORKPATH_KEY);
  });
  test('不做大小写折叠', () => {
    expect(workpathKey('/Users/A')).toBe('/Users/A');
  });
  test('Windows 反斜杠归一为正斜杠并去尾', () => {
    expect(workpathKey('C:\\work\\proj\\')).toBe('C:/work/proj');
  });
});
