/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { splitFileDisplayPath } from './pathDisplay';

describe('splitFileDisplayPath', () => {
  test('keeps the trailing slash with the muted directory segment', () => {
    expect(splitFileDisplayPath('backend/main.go', 'main.go')).toEqual({
      directoryPath: 'backend/',
      fileName: 'main.go',
      fullPath: 'backend/main.go',
    });
  });

  test('separates a relative directory prefix from the filename', () => {
    expect(splitFileDisplayPath('ui/src/components/FileRow.tsx', 'FileRow.tsx')).toEqual({
      directoryPath: 'ui/src/components/',
      fileName: 'FileRow.tsx',
      fullPath: 'ui/src/components/FileRow.tsx',
    });
  });

  test('normalizes Windows separators for a consistent relative-path display', () => {
    expect(splitFileDisplayPath('.\\outputs\\report.md', 'report.md')).toEqual({
      directoryPath: 'outputs/',
      fileName: 'report.md',
      fullPath: 'outputs/report.md',
    });
  });

  test('keeps a root-level filename as the primary segment', () => {
    expect(splitFileDisplayPath('report.md', 'report.md')).toEqual({
      directoryPath: '',
      fileName: 'report.md',
      fullPath: 'report.md',
    });
  });
});
