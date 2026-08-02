/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const modalSource = readFileSync(new URL('./UpdateModal.tsx', import.meta.url), 'utf8');

describe('update install interaction feedback', () => {
  test('install click immediately enters a guarded visible state', () => {
    const handlerStart = modalSource.indexOf('const quitAndInstall = async () =>');
    const handlerEnd = modalSource.indexOf('const formatSpeed', handlerStart);
    const handler = modalSource.slice(handlerStart, handlerEnd);

    expect(handlerStart).toBeGreaterThan(-1);
    expect(handler.includes('if (installRequestedRef.current) return')).toBe(true);
    expect(handler.indexOf("setStatus('installing')")).toBeLessThan(
      handler.indexOf('await ipcBridge.autoUpdate.quitAndInstall.invoke()')
    );
  });

  test('install progress has a dedicated live region and cannot be dismissed mid-handoff', () => {
    expect(modalSource.includes("case 'installing':")).toBe(true);
    expect(modalSource.includes("installPhase === 'downloading'")).toBe(true);
    expect(modalSource.includes("aria-live='polite'")).toBe(true);
    expect(modalSource.includes("showClose: status !== 'installing'")).toBe(true);
    expect(modalSource.includes('if (installRequestedRef.current) return;\n    setVisible(false)')).toBe(true);
  });

  test('reopening the modal cannot reset an in-flight install', () => {
    const openHandlerStart = modalSource.indexOf('const handleOpenUpdateModal');
    const openHandlerEnd = modalSource.indexOf('useEffect(() =>', openHandlerStart);
    const openHandler = modalSource.slice(openHandlerStart, openHandlerEnd);

    expect(openHandler.indexOf('if (installRequestedRef.current) return')).toBeLessThan(
      openHandler.indexOf('resetState()')
    );
  });
});
