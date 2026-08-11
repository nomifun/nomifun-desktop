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
    expect(modalSource.includes("status === 'installing'")).toBe(true);
    expect(modalSource.includes("installPhase === 'downloading'")).toBe(false);
    expect(modalSource.includes("aria-live='polite'")).toBe(true);
    expect(modalSource.includes("const canDismiss = status !== 'installing'")).toBe(true);
    expect(modalSource.includes('if (installRequestedRef.current) return;\n    setVisible(false)')).toBe(true);
  });

  test('download and install actions use distinct labels and states', () => {
    const compactStart = modalSource.indexOf('const renderCompactContent = () =>');
    const detailStart = modalSource.indexOf('const renderDetailContent = () =>', compactStart);
    const compact = modalSource.slice(compactStart, detailStart);

    expect(compact.includes("status === 'available'")).toBe(true);
    expect(compact.includes("t('update.updateNow')")).toBe(true);
    expect(compact.includes("status === 'downloaded'")).toBe(true);
    expect(compact.includes("t('update.installNow')")).toBe(true);
    expect(compact.includes("t('update.downloadAndInstall')")).toBe(false);
  });

  test('reopening the modal cannot reset an in-flight install', () => {
    const openHandlerStart = modalSource.indexOf('const handleOpenUpdateModal');
    const openHandlerEnd = modalSource.indexOf('useEffect(() =>', openHandlerStart);
    const openHandler = modalSource.slice(openHandlerStart, openHandlerEnd);

    expect(openHandler.indexOf('if (installRequestedRef.current) return')).toBeLessThan(
      openHandler.indexOf('resetState()')
    );
  });

  test('a download already in flight cannot be started a second time', () => {
    // startDownload had no re-entrancy guard (quitAndInstall has always had one).
    // Two live download flows each keep their OWN byte accumulator but publish to
    // one shared status emitter, so the single progress bar flips between two
    // unrelated series — "two installers downloading with different progress".
    const start = modalSource.indexOf('const startDownload = async () =>');
    const end = modalSource.indexOf('const quitAndInstall', start);
    const handler = modalSource.slice(start, end);

    expect(start).toBeGreaterThan(-1);
    // Tolerant of braces/formatting: the guard must return before anything else.
    expect(/if\s*\(downloadRequestedRef\.current\)\s*\{?\s*return/.test(handler)).toBe(true);
    expect(handler.indexOf('downloadRequestedRef')).toBeLessThan(handler.indexOf('setStatus'));
  });

  test('reopening the modal always re-checks so a wedged download cannot strand it', () => {
    // Guarding the re-check looked safer but disabled the only recovery path: a
    // download whose invoke never settles left the guard set and the modal frozen
    // for the rest of the session. The re-check re-derives the live state instead.
    const openHandlerStart = modalSource.indexOf('const handleOpenUpdateModal');
    const openHandlerEnd = modalSource.indexOf('useEffect(() =>', openHandlerStart);
    const openHandler = modalSource.slice(openHandlerStart, openHandlerEnd);

    expect(openHandler.includes('resetState()')).toBe(true);
    expect(openHandler.includes('void checkForUpdates()')).toBe(true);
    // Only an in-flight INSTALL may skip the reset.
    expect(openHandler.includes('downloadRequestedRef.current) return')).toBe(false);
  });

  test('the check derives every post-check state from the native slot', () => {
    const start = modalSource.indexOf('const checkForUpdates = async () =>');
    const end = modalSource.indexOf('const startDownload', start);
    const handler = modalSource.slice(start, end);

    expect(handler.includes('deriveUpdateStatus(')).toBe(true);
    expect(handler.includes('retainedVersion')).toBe(true);
    expect(handler.includes('slotState')).toBe(true);
    // A live native download must re-attach, not re-arm the Download button.
    expect(handler.includes("derived === 'downloading'")).toBe(true);

    // Scope to the Tauri path (autoUpdateOk). The hardcoded 'available' that used
    // to hide a retained package must be gone from THIS branch; the manual-mode
    // branch below it is the WebUI fallback and legitimately still uses it.
    const autoStart = handler.indexOf('if (autoUpdateOk) {');
    const autoEnd = handler.indexOf('// Manual mode', autoStart);
    const autoBranch = handler.slice(autoStart, autoEnd);
    expect(autoStart).toBeGreaterThan(-1);
    expect(autoEnd).toBeGreaterThan(autoStart);
    expect(autoBranch.includes("setStatus('available')")).toBe(false);
    expect(autoBranch.includes('setStatus(derived)')).toBe(true);
  });

  test('every download status frame is filtered through the shared predicate', () => {
    // A bare `includes('downloadVersionRef')` passed even with the whole guard
    // deleted, because the identifier survives at its declaration. Assert the
    // predicate call and that it covers the TERMINAL frames too — a stale
    // completion used to flip the modal to Install mid-transfer.
    expect(modalSource.includes('shouldApplyDownloadEvent(evt.version, downloadVersionRef.current)')).toBe(
      true
    );
    const guardStart = modalSource.indexOf('shouldApplyDownloadEvent(');
    const guard = modalSource.slice(modalSource.lastIndexOf('if (', guardStart), guardStart);
    expect(guard.includes("evt.status === 'downloading'")).toBe(true);
    expect(guard.includes("evt.status === 'downloaded'")).toBe(true);
    expect(guard.includes("evt.status === 'error'")).toBe(true);
  });

  test('an install refused for a missing package lands on a screen that can act', () => {
    // 'downloaded' offers only Install — the button that just failed. The new
    // recoverable path must send the user somewhere the prescribed re-download
    // actually exists, or "recoverable" is only true in the adapter.
    const start = modalSource.indexOf('const quitAndInstall = async () =>');
    const end = modalSource.indexOf('const formatSpeed', start);
    const handler = modalSource.slice(start, end);

    expect(handler.includes("messageKey === 'update.packageNoLongerReady'")).toBe(true);
    expect(handler.indexOf("messageKey === 'update.packageNoLongerReady'")).toBeLessThan(
      handler.indexOf("setStatus('downloaded')")
    );
    expect(handler.includes('void checkForUpdates()')).toBe(true);
  });
});
