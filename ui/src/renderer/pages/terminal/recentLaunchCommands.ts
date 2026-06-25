/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Persists the user's most recently launched custom ("自定义"/shell preset)
 * terminal commands so they can be re-selected from the create page. Mirrors the
 * recent-workspaces pattern: localStorage-backed, most-recent-first, deduped,
 * capped at MAX_RECENT_COMMANDS. Only the editable launch-command string is
 * stored — no other session metadata.
 */

export const RECENT_LAUNCH_COMMANDS_KEY = 'nomifun:recent-terminal-commands';
const MAX_RECENT_COMMANDS = 5;

export const getRecentLaunchCommands = (): string[] => {
  try {
    const parsed = JSON.parse(localStorage.getItem(RECENT_LAUNCH_COMMANDS_KEY) ?? '[]');
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((item): item is string => typeof item === 'string');
  } catch {
    return [];
  }
};

export const addRecentLaunchCommand = (command: string): void => {
  const trimmed = command.trim();
  if (!trimmed) return;
  try {
    const prev = getRecentLaunchCommands();
    const next = [trimmed, ...prev.filter((item) => item !== trimmed)].slice(0, MAX_RECENT_COMMANDS);
    localStorage.setItem(RECENT_LAUNCH_COMMANDS_KEY, JSON.stringify(next));
  } catch {}
};
