/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  fileUriToPath,
  isAbsoluteLocalPath,
  isDirectImageSource,
  isLocalImageSource,
  joinLocalPath,
  resolveImageSource,
  safeDecodeUriComponent,
} from './localPath';

const extendedUnc = String.raw`\\?\UNC\server\share\images\cat.png`;
const extendedDrive = String.raw`\\?\C:\images\cat.png`;

describe('cross-platform local path detection', () => {
  test('recognizes POSIX, Windows drive, UNC, and extended Windows paths', () => {
    const absolutePaths = [
      '/Users/alice/Pictures/cat.png',
      '/home/alice/Pictures/cat.png',
      'C:\\Users\\Alice\\Pictures\\cat.png',
      'D:/Pictures/cat.png',
      String.raw`\\server\share\images\cat.png`,
      '//server/share/images/cat.png',
      extendedUnc,
      extendedDrive,
    ];

    for (const value of absolutePaths) {
      expect(isAbsoluteLocalPath(value)).toBe(true);
    }
  });

  test('does not mistake drive-relative paths, relative paths, or URLs for absolute paths', () => {
    for (const value of ['C:images\\cat.png', 'images/cat.png', '../cat.png', 'https://example.com/cat.png']) {
      expect(isAbsoluteLocalPath(value)).toBe(false);
    }
  });
});

describe('file URI conversion', () => {
  test('converts macOS, Linux, Windows drive, and UNC file URIs', () => {
    const cases: Array<[string, string]> = [
      ['file:///Users/alice/My%20Image.png', '/Users/alice/My Image.png'],
      ['file:///home/alice/My%20Image.png', '/home/alice/My Image.png'],
      ['file:///C:/Users/Alice/My%20Image.png', 'C:/Users/Alice/My Image.png'],
      ['file://server/share/My%20Image.png', '//server/share/My Image.png'],
      ['file://localhost/tmp/My%20Image.png', '/tmp/My Image.png'],
    ];

    for (const [uri, expected] of cases) {
      expect(fileUriToPath(uri)).toBe(expected);
    }
  });

  test('malformed percent encoding never throws or corrupts the remaining path', () => {
    expect(safeDecodeUriComponent('/tmp/bad%.png')).toBe('/tmp/bad%.png');
    expect(fileUriToPath('file:///tmp/bad%.png')).toBe('/tmp/bad%.png');
    expect(fileUriToPath('https://example.com/cat.png')).toBeNull();
  });
});

describe('image source classification and resolution', () => {
  test('keeps http, data, blob, and other non-file schemes out of the filesystem API', () => {
    const direct = [
      'http://example.com/cat.png',
      'HTTPS://example.com/cat.png',
      'data:image/png;base64,aGVsbG8=',
      'blob:https://example.com/id',
      'custom-image:asset-id',
    ];
    for (const value of direct) {
      expect(isDirectImageSource(value)).toBe(true);
      expect(isLocalImageSource(value)).toBe(false);
      expect(resolveImageSource(value, '/workspace')).toEqual({ kind: 'direct', url: value });
    }
  });

  test('treats native paths, file URIs, and relative paths as local', () => {
    for (const value of [
      '/tmp/cat.png',
      'C:\\Pictures\\cat.png',
      String.raw`\\server\share\cat.png`,
      extendedUnc,
      'file:///tmp/cat.png',
      'images/cat.png',
    ]) {
      expect(isLocalImageSource(value)).toBe(true);
    }
  });

  test('resolves encoded relative and file URI paths without changing direct URLs', () => {
    expect(resolveImageSource('images/My%20Image.png', '/Users/alice/project')).toEqual({
      kind: 'local',
      path: '/Users/alice/project/images/My Image.png',
      workspace: '/Users/alice/project',
    });
    expect(resolveImageSource('images/My%20Image.png', 'C:\\Users\\Alice\\project')).toEqual({
      kind: 'local',
      path: 'C:\\Users\\Alice\\project\\images\\My Image.png',
      workspace: 'C:\\Users\\Alice\\project',
    });
    expect(resolveImageSource('file://server/share/My%20Image.png', 'C:\\unused')).toEqual({
      kind: 'local',
      path: '//server/share/My Image.png',
      workspace: 'C:\\unused',
    });
  });

  test('rejects relative traversal and Windows drive-relative paths at the workspace boundary', () => {
    for (const value of ['../old.png', 'images/../../old.png', '%2e%2e/old.png', 'C:old.png']) {
      expect(resolveImageSource(value, '/workspace')).toEqual({
        kind: 'local',
        path: '',
        workspace: '/workspace',
      });
    }
    expect(resolveImageSource('images/../generated.png', '/workspace')).toEqual({
      kind: 'local',
      path: '/workspace/generated.png',
      workspace: '/workspace',
    });
  });
});

describe('joinLocalPath', () => {
  test('resolves parent segments against the base path on POSIX and Windows', () => {
    expect(joinLocalPath('/Users/alice/project/output', '../cat.png')).toBe('/Users/alice/project/cat.png');
    expect(joinLocalPath('C:\\Users\\Alice\\project\\output', '..\\cat.png')).toBe(
      'C:\\Users\\Alice\\project\\cat.png'
    );
  });

  test('preserves UNC and extended UNC roots', () => {
    expect(joinLocalPath(String.raw`\\server\share\project`, String.raw`output\cat.png`)).toBe(
      String.raw`\\server\share\project\output\cat.png`
    );
    expect(joinLocalPath(String.raw`\\?\UNC\server\share\project`, String.raw`..\cat.png`)).toBe(
      String.raw`\\?\UNC\server\share\cat.png`
    );
  });

  test('preserves URI delimiters and applies URL parent semantics', () => {
    expect(joinLocalPath('https://example.com/assets/generated', '../cat.png')).toBe(
      'https://example.com/assets/cat.png'
    );
    expect(joinLocalPath('file:///C:/workspace', 'images/cat.png')).toBe(
      'file:///C:/workspace/images/cat.png'
    );
  });

  test('does not prepend a base to an already absolute target', () => {
    expect(joinLocalPath('/workspace', 'https://example.com/cat.png')).toBe('https://example.com/cat.png');
    expect(joinLocalPath('/workspace', String.raw`\\server\share\cat.png`)).toBe(
      String.raw`\\server\share\cat.png`
    );
  });
});
