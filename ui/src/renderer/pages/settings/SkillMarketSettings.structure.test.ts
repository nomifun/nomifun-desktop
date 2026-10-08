/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./SkillMarketSettings.tsx', import.meta.url), 'utf8');

describe('SkillMarketSettings installation boundary', () => {
  test('adds a market skill through the dedicated installer API', () => {
    expect(source.includes('ipcBridge.fs.installSkillMarketItem.invoke(')).toBe(true);
    expect(source.includes('showInstallCommand={false}')).toBe(true);
    expect(source.includes('detectAndCountExternalSkills')).toBe(false);
  });

  test('does not turn skill installation into an ordinary Agent Session', () => {
    expect(source.includes('useNomiQuickStart')).toBe(false);
    expect(source.includes('const { start }')).toBe(false);
    expect(/\bstart\s*\(/.test(source)).toBe(false);
    expect(source.includes('buildSkillMarketConversationName')).toBe(false);
    expect(source.includes('buildSkillMarketInstallPrompt')).toBe(false);
    expect(source.includes('agentPlatform.sessions.create')).toBe(false);
    expect(source.includes('ipcBridge.conversation.create')).toBe(false);
    expect(source.includes('ipcBridge.conversation.sendMessage')).toBe(false);
    expect(source.includes('sessionStorage')).toBe(false);
    expect(source.includes('/conversation')).toBe(false);
  });
});
