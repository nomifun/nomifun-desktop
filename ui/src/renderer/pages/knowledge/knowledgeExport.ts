/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

const WINDOWS_RESERVED_NAME = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\s*\.|$)/i;

/** A portable basename for the native save dialog and WebUI filename field. */
export function defaultKnowledgeExportName(name: string, date = new Date()): string {
  const cleaned = name.replace(/[\\/:*?"<>|\x00-\x1f]/g, '-').replace(/[.\s]+$/g, '').trim().slice(0, 100) || 'knowledge';
  const stem = WINDOWS_RESERVED_NAME.test(cleaned) ? `_${cleaned}` : cleaned;
  const day = [date.getFullYear(), String(date.getMonth() + 1).padStart(2, '0'), String(date.getDate()).padStart(2, '0')].join('-');
  return `${stem}-${day}.zip`;
}

/** Never allow a filename field to change the selected destination directory. */
export function isKnowledgeExportNameValid(name: string): boolean {
  return name.trim() === name && name.length > 4 && name.length <= 180 &&
    !/[\\/:*?"<>|\x00-\x1f]/.test(name) && !/[.\s]$/.test(name) &&
    !WINDOWS_RESERVED_NAME.test(name) && /\.zip$/i.test(name);
}

export async function pickKnowledgeExportFile(defaultName: string): Promise<string | null> {
  const { save } = await import('@tauri-apps/plugin-dialog');
  return save({ defaultPath: defaultName, filters: [{ name: 'ZIP', extensions: ['zip'] }] });
}
