/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { SUPPORTED_AGENTS } from './supportedAgents';

const agent = (backend: string) => {
  const found = SUPPORTED_AGENTS.find((item) => item.backend === backend);
  if (!found) throw new Error(`Missing supported agent: ${backend}`);
  return found;
};

describe('SUPPORTED_AGENTS install guidance', () => {
  test('limits one-click install to curated, low-risk commands', () => {
    const oneClickBackends = SUPPORTED_AGENTS.filter((item) => item.installHint.trim().length > 0).map(
      (item) => item.backend
    );

    expect(oneClickBackends).toEqual(['claude', 'codex', 'qwen', 'opencode', 'goose']);
  });

  test('uses product-owned manual install pages for high-traffic agents', () => {
    expect(agent('claude').website).toBe('https://code.claude.com/docs/en/setup');
    expect(agent('codex').website).toBe('https://developers.openai.com/codex/cli');
    expect(agent('gemini').website).toBe('https://geminicli.com/');
    expect(agent('qwen').website).toBe('https://qwenlm.github.io/qwen-code-docs/en/users/overview/');
    expect(agent('copilot').website).toBe(
      'https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli'
    );
  });

  test('does not expose known stale or third-party manual install URLs', () => {
    const websites = SUPPORTED_AGENTS.flatMap((item) => (item.website ? [item.website] : []));

    expect(websites.some((url) => url.includes('github.com/anthropics/claude-code'))).toBe(false);
    expect(websites.some((url) => url.includes('github.com/github/copilot-cli'))).toBe(false);
    expect(websites.some((url) => url.includes('block.github.io/goose'))).toBe(false);
    expect(websites.some((url) => url.includes('snowcli.dev'))).toBe(false);
  });
});
