/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { injectBackgroundCssBlock } from '@renderer/pages/settings/DisplaySettings/backgroundUtils';
import { addImportantToAll } from './customCssProcessor';

describe('addImportantToAll', () => {
  test('preserves semicolons inside image data URLs', () => {
    const processed = addImportantToAll('body { background-image: url("data:image/png;base64,AAAA"); background-size: cover; }');

    expect(processed.includes('url("data:image/png;base64,AAAA")')).toBe(true);
    expect(processed.includes('background-size: cover !important;')).toBe(true);
    expect(processed.includes('data:image/png !important')).toBe(false);
  });

  test('preserves manual background images after adding important flags', () => {
    const css = injectBackgroundCssBlock('', 'data:image/png;base64,AAAA');
    const processed = addImportantToAll(css);

    expect(processed.includes('url("data:image/png;base64,AAAA")')).toBe(true);
    expect(processed.includes('linear-gradient(var(--nomi-manual-bg-mask), var(--nomi-manual-bg-mask))')).toBe(true);
    expect(processed.includes('data:image/png !important')).toBe(false);
  });
});
