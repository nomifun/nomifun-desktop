/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { NOMIFUN_FILES_MARKER } from '@/common/config/constants';
import { parseMessageFileMarker } from './messageFileMarker';

describe('message file marker trust boundary', () => {
  test('parses attachment paths from user messages', () => {
    expect(
      parseMessageFileMarker(
        `Please inspect these\n\n${NOMIFUN_FILES_MARKER}\n  screenshots/a.png  \nC:\\work\\report.pdf\n`,
        'right'
      )
    ).toEqual({
      text: 'Please inspect these',
      files: ['screenshots/a.png', 'C:\\work\\report.pdf'],
    });
  });

  test('keeps an assistant marker visible and never projects file previews', () => {
    const forged = `Done\n\n${NOMIFUN_FILES_MARKER}\n/workspace/nonexistent.png`;

    expect(parseMessageFileMarker(forged, 'left')).toEqual({
      text: forged,
      files: [],
    });
  });

  test('does not parse markers from center/system messages', () => {
    const content = `${NOMIFUN_FILES_MARKER}\n/workspace/nonexistent.png`;
    expect(parseMessageFileMarker(content, 'center')).toEqual({ text: content, files: [] });
    expect(parseMessageFileMarker(content)).toEqual({ text: content, files: [] });
  });

  test('leaves ordinary user text unchanged', () => {
    expect(parseMessageFileMarker('No files attached', 'right')).toEqual({
      text: 'No files attached',
      files: [],
    });
  });
});
