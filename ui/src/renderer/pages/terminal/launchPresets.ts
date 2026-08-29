/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Terminal launch presets map a chosen preset (plain shell or an agent CLI) to
 * a concrete launch command + args. Agent CLIs always start in FullAuto. The
 * result remains editable before launch, and the backend resolves `$SHELL`.
 */

/** Sentinel meaning "the platform login shell" — resolved server-side. */
export const SHELL_SENTINEL = '$SHELL';

export type TerminalPresetId = 'shell' | 'claude' | 'codex' | 'gemini';

export type LaunchCommand = { command: string; args: string[] };

export type TerminalPreset = {
  id: TerminalPresetId;
  /** i18n key for the display label. */
  labelKey: string;
  /** Backend identifier stored on the session (undefined for plain shell). */
  backend?: string;
  /** The program to launch (the shell sentinel for `shell`). */
  command: string;
};

export const TERMINAL_PRESETS: TerminalPreset[] = [
  { id: 'shell', labelKey: 'terminal.preset.shell', command: SHELL_SENTINEL },
  { id: 'claude', labelKey: 'terminal.preset.claude', backend: 'claude', command: 'claude' },
  { id: 'codex', labelKey: 'terminal.preset.codex', backend: 'codex', command: 'codex' },
  { id: 'gemini', labelKey: 'terminal.preset.gemini', backend: 'gemini', command: 'gemini' },
];

/**
 * Full-auto launch flags per agent CLI. These bypass interactive approval so a
 * single terminal can drive the user's local agent with full OS capability.
 */
const FULL_AUTO_FLAGS: Record<string, string[]> = {
  claude: ['--dangerously-skip-permissions'],
  codex: ['--dangerously-bypass-approvals-and-sandbox'],
  gemini: ['--yolo'],
};

export function getPreset(id: TerminalPresetId): TerminalPreset {
  const preset = TERMINAL_PRESETS.find((p) => p.id === id);
  if (!preset) throw new Error(`Unknown terminal preset: ${id}`);
  return preset;
}

/** Build the fixed FullAuto launch for an Agent CLI, or a plain shell. */
export function buildLaunchCommand(id: TerminalPresetId): LaunchCommand {
  const preset = getPreset(id);
  if (id === 'shell') {
    return { command: SHELL_SENTINEL, args: [] };
  }
  return { command: preset.command, args: FULL_AUTO_FLAGS[id] ?? [] };
}

/**
 * Render a command + args list into a single editable preview string. Tokens
 * containing whitespace are double-quoted so the preview round-trips through
 * `parseCommandPreview` (paths like "/Applications/My App/agent").
 */
export function formatCommandPreview(cmd: LaunchCommand): string {
  return [cmd.command, ...cmd.args].map(quoteToken).join(' ').trim();
}

function quoteToken(token: string): string {
  if (!/\s/.test(token)) return token;
  return `"${token.replace(/"/g, '\\"')}"`;
}

/**
 * Parse an edited preview string back into command + args. Double- and
 * single-quoted tokens may contain whitespace; `\"` escapes a quote inside a
 * double-quoted token. An unterminated quote keeps the rest of the string as
 * one token (graceful while the user is still typing).
 */
export function parseCommandPreview(preview: string): LaunchCommand {
  const [command, ...args] = tokenizeCommandPreview(preview.trim());
  return { command: command ?? SHELL_SENTINEL, args };
}

function tokenizeCommandPreview(input: string): string[] {
  const tokens: string[] = [];
  let current = '';
  let quote: '"' | "'" | null = null;
  let hasToken = false;
  for (let i = 0; i < input.length; i++) {
    const ch = input[i];
    if (quote === '"' && ch === '\\' && input[i + 1] === '"') {
      current += '"';
      i++;
      continue;
    }
    if (quote) {
      if (ch === quote) quote = null;
      else current += ch;
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch as '"' | "'";
      hasToken = true;
      continue;
    }
    if (/\s/.test(ch)) {
      if (hasToken) {
        tokens.push(current);
        current = '';
        hasToken = false;
      }
      continue;
    }
    current += ch;
    hasToken = true;
  }
  if (hasToken) tokens.push(current);
  return tokens;
}
