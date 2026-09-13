/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL): string =>
  readFileSync(url, 'utf8').replace(/\r\n/g, '\n');

const guidPage = readSource(new URL('./GuidPage.tsx', import.meta.url));
const composerPath = new URL(
  './components/ComposerEntryStrip.tsx',
  import.meta.url
);
const quickStartPath = new URL(
  '../../hooks/agent/usePluginRuntimeQuickStart.ts',
  import.meta.url
);
const autoPreviewPath = new URL(
  '../../hooks/file/useAutoPreviewPluginRuntime.ts',
  import.meta.url
);

const sourceIsMissing = (url: URL): boolean => {
  try {
    readFileSync(url);
    return false;
  } catch {
    return true;
  }
};

describe('Guid and PluginRuntime surface separation', () => {
  test('Guid has one ordinary send path and no retired session entry strip', () => {
    expect(guidPage.includes('send.sendMessageHandler')).toBe(true);
    expect(guidPage.includes('pluginRuntimeMode')).toBe(false);
    expect(guidPage.includes('pluginRuntimeQuickStart')).toBe(false);
    expect(guidPage.includes('miniapp=')).toBe(false);
    expect(guidPage.includes('new URLSearchParams(location.search)')).toBe(
      false
    );
    expect(guidPage.includes('ComposerEntryStrip')).toBe(false);
    expect(guidPage.includes('SummonDrawer')).toBe(false);
    expect(sourceIsMissing(composerPath)).toBe(true);
  });

  test('retired conversation launch hooks are physically absent', () => {
    expect(sourceIsMissing(quickStartPath)).toBe(true);
    expect(sourceIsMissing(autoPreviewPath)).toBe(true);
  });
});
