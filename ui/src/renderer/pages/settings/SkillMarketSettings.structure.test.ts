/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./SkillMarketSettings.tsx', import.meta.url), 'utf8');
const pageSource = readFileSync(new URL('../enhancedTools/index.tsx', import.meta.url), 'utf8');

describe('SkillMarketSettings installation boundary', () => {
  test('adds a market skill through the dedicated installer API', () => {
    expect(source.includes('ipcBridge.fs.installSkillMarketItem.invoke(')).toBe(true);
    expect(source.includes('showInstallCommand={false}')).toBe(true);
    expect(source.includes('detectAndCountExternalSkills')).toBe(false);
    expect(source.includes('recordInstalledMarketItem')).toBe(true);
    expect(source.includes('isSkillMarketItemInstalled')).toBe(false);
    expect(source.includes('listAvailableSkills')).toBe(false);
    expect(source.includes('reconcileInstalledMarketState')).toBe(false);
    expect(source.includes('installedStateLoading')).toBe(false);
    expect(source.includes('active?:')).toBe(false);
    expect(source.includes('addedStateLoading={addedStateLoading}')).toBe(true);
  });

  test('refreshes the single library owner and returns to its installed item after a successful install', () => {
    expect(pageSource.includes('onInstalled={handleMarketInstalled}')).toBe(true);
    expect(pageSource.includes('void library.refresh()')).toBe(true);
    expect(pageSource.includes("finishMarket('skills', skillNames[0])")).toBe(true);
    expect(source.includes('onInstalled?.(installed.skill_names)')).toBe(true);
    expect(pageSource.includes('refreshToken')).toBe(false);
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
