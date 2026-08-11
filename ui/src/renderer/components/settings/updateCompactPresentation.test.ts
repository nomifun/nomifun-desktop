/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const modalSource = readFileSync(new URL('./UpdateModal.tsx', import.meta.url), 'utf8');
const modalCss = readFileSync(new URL('./UpdateModal.css', import.meta.url), 'utf8');

describe('compact update presentation', () => {
  test('opens update checks in a non-modal bottom-right card', () => {
    expect(modalSource.includes("useState<UpdatePresentation>('compact')")).toBe(true);
    expect(modalSource.includes("presentation === 'compact' || !canShowDetails")).toBe(true);
    expect(modalSource.includes("className='update-compact-card-host'")).toBe(true);
    expect(modalCss.includes('.update-compact-card-host {')).toBe(true);
    expect(modalCss.includes('position: fixed')).toBe(true);
    expect(modalCss.includes('right: 24px')).toBe(true);
    expect(modalCss.includes('bottom: 24px')).toBe(true);
  });

  test('keeps available-update actions wired to the existing download and dismiss handlers', () => {
    const compactStart = modalSource.indexOf('const renderCompactContent = () =>');
    const detailStart = modalSource.indexOf('const renderDetailContent = () =>', compactStart);
    const compact = modalSource.slice(compactStart, detailStart);

    expect(compact.includes("status === 'available'")).toBe(true);
    expect(compact.includes('onClick={startDownload}')).toBe(true);
    expect(compact.includes("t('update.updateNow')")).toBe(true);
    expect(compact.includes('onClick={handleClose}')).toBe(true);
    expect(compact.includes("t('update.later')")).toBe(true);
  });

  test('shows current state and download progress in the same compact surface', () => {
    const compactStart = modalSource.indexOf('const renderCompactContent = () =>');
    const detailStart = modalSource.indexOf('const renderDetailContent = () =>', compactStart);
    const compact = modalSource.slice(compactStart, detailStart);

    expect(compact.includes("status === 'checking'")).toBe(true);
    expect(compact.includes("status === 'upToDate'")).toBe(true);
    expect(compact.includes("status === 'downloading'")).toBe(true);
    expect(compact.includes('percent={progress.percent}')).toBe(true);
    expect(compact.includes('progress.transferred')).toBe(true);
    expect(compact.includes('progress.total')).toBe(true);
  });

  test('view details reveals the retained expanded dialog without changing update data', () => {
    expect(modalSource.includes("setPresentation('detail')")).toBe(true);
    expect(modalSource.includes('onClick={showDetails}')).toBe(true);
    expect(modalSource.includes('<NomiModal')).toBe(true);
    expect(modalSource.includes("className='update-modal__release-scroll custom-scrollbar'")).toBe(true);
    expect(modalSource.includes('updateInfo?.body || autoUpdateInfo?.releaseNotes')).toBe(true);
  });

  test('expanded rendering retains details only and cannot revive legacy status modals', () => {
    const detailStart = modalSource.indexOf('const renderDetailContent = () =>');
    const detailEnd = modalSource.indexOf('const isAvailableDialog', detailStart);
    const detail = modalSource.slice(detailStart, detailEnd);

    expect(detail.includes("case 'available':")).toBe(true);
    expect(detail.includes("case 'error':")).toBe(true);
    expect(detail.includes("case 'checking':")).toBe(false);
    expect(detail.includes("case 'upToDate':")).toBe(false);
    expect(detail.includes("case 'downloading':")).toBe(false);
    expect(detail.includes("case 'downloaded':")).toBe(false);
    expect(detail.includes("case 'installing':")).toBe(false);
    expect(detail.includes("case 'success':")).toBe(false);
  });

  test('retry collapses details before entering the checking state', () => {
    const checkStart = modalSource.indexOf('const checkForUpdates = async () =>');
    const checkEnd = modalSource.indexOf('const startDownload', checkStart);
    const check = modalSource.slice(checkStart, checkEnd);

    expect(check.indexOf("setPresentation('compact')")).toBeLessThan(check.indexOf("setStatus('checking')"));
  });

  test('starting a download only collapses presentation before the original guarded path', () => {
    const start = modalSource.indexOf('const startDownload = async () =>');
    const end = modalSource.indexOf('const quitAndInstall', start);
    const handler = modalSource.slice(start, end);

    expect(handler.includes("setPresentation('compact')")).toBe(true);
    expect(handler.includes('await ipcBridge.autoUpdate.download.invoke()')).toBe(true);
    expect((handler.match(/ipcBridge\.autoUpdate\.download\.invoke\(\)/g) ?? [])).toHaveLength(1);
  });
});
