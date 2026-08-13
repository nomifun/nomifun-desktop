/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const SHARED_PAIRING_FORMS = [
  'TelegramConfigForm.tsx',
  'DiscordConfigForm.tsx',
  'SlackConfigForm.tsx',
  'MatrixConfigForm.tsx',
  'MattermostConfigForm.tsx',
  'TwitchConfigForm.tsx',
  'NostrConfigForm.tsx',
  'QQBotConfigForm.tsx',
  'LarkConfigForm.tsx',
  'DingTalkConfigForm.tsx',
  'WecomConfigForm.tsx',
] as const;

describe('channel pairing list visibility', () => {
  for (const file of SHARED_PAIRING_FORMS) {
    test(`${file} keeps pending approvals visible alongside authorized users`, () => {
      const source = readFileSync(new URL(file, import.meta.url), 'utf8');

      expect(/\{pluginStatus\?\.enabled\s*&&\s*\(\s*<PendingPairingList/.test(source)).toBe(true);
      expect(
        /\{pluginStatus\?\.enabled\s*&&\s*authorizedUsers\.length\s*>\s*0\s*&&\s*\(\s*<AuthorizedUserList/.test(
          source
        )
      ).toBe(true);
      expect(
        /authorizedUsers\.length\s*===\s*0\s*&&\s*\(\s*<PendingPairingList/.test(source)
      ).toBe(false);
    });
  }
});
