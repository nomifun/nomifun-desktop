/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

const SUPPORTED_LANGUAGES = new Set([
  'bash',
  'c',
  'cpp',
  'csharp',
  'css',
  'diff',
  'docker',
  'go',
  'ini',
  'java',
  'javascript',
  'json',
  'jsx',
  'kotlin',
  'latex',
  'lua',
  'makefile',
  'markdown',
  'markup',
  'php',
  'powershell',
  'python',
  'ruby',
  'rust',
  'scss',
  'sql',
  'swift',
  'tsx',
  'typescript',
  'vbnet',
  'yaml',
]);

const LANGUAGE_ALIASES: Record<string, string> = {
  'c#': 'csharp',
  'c++': 'cpp',
  cjs: 'javascript',
  console: 'text',
  cs: 'csharp',
  dockerfile: 'docker',
  error: 'text',
  htm: 'markup',
  html: 'markup',
  js: 'javascript',
  json5: 'json',
  kt: 'kotlin',
  kts: 'kotlin',
  log: 'text',
  math: 'latex',
  md: 'markdown',
  mjs: 'javascript',
  plain: 'text',
  plaintext: 'text',
  patch: 'diff',
  ps1: 'powershell',
  py: 'python',
  rb: 'ruby',
  rs: 'rust',
  sh: 'bash',
  shell: 'bash',
  'shell-session': 'bash',
  stack: 'text',
  svg: 'markup',
  tex: 'latex',
  text: 'text',
  ts: 'typescript',
  txt: 'text',
  xml: 'markup',
  yml: 'yaml',
  zsh: 'bash',
};

/**
 * Resolve a Markdown fence label to a grammar we explicitly ship.
 *
 * Never auto-detect an unknown label. Error messages commonly use informal
 * fences such as `log`, `console`, or `error`; treating those as plain text is
 * both more accurate and safer than running every registered grammar.
 */
export const resolveSyntaxLanguage = (language: string | undefined): string => {
  const normalized = language?.trim().toLowerCase() || 'text';
  const aliased = LANGUAGE_ALIASES[normalized] ?? normalized;
  return SUPPORTED_LANGUAGES.has(aliased) ? aliased : 'text';
};
