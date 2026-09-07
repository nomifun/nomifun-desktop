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
const composer = readSource(
  new URL('./components/ComposerEntryStrip.tsx', import.meta.url)
);
const quickStartPath = new URL(
  '../../hooks/agent/useMiniAppQuickStart.ts',
  import.meta.url
);
const autoPreviewPath = new URL(
  '../../hooks/file/useAutoPreviewMiniApp.ts',
  import.meta.url
);

describe('Guid and MiniApp surface separation', () => {
  test('Guid has one ordinary send path and no MiniApp mode', () => {
    expect(guidPage.includes('send.sendMessageHandler')).toBe(true);
    expect(guidPage.includes('miniAppMode')).toBe(false);
    expect(guidPage.includes('miniAppQuickStart')).toBe(false);
    expect(guidPage.includes('miniapp=')).toBe(false);
    expect(guidPage.includes('new URLSearchParams(location.search)')).toBe(
      false
    );
    expect(composer.includes('onCreateMiniApp')).toBe(false);
  });

  test('retired conversation launch hooks are physically absent', () => {
    let quickStartMissing = false;
    try {
      readFileSync(quickStartPath);
    } catch {
      quickStartMissing = true;
    }
    let autoPreviewMissing = false;
    try {
      readFileSync(autoPreviewPath);
    } catch {
      autoPreviewMissing = true;
    }
    expect(quickStartMissing).toBe(true);
    expect(autoPreviewMissing).toBe(true);
  });
});
