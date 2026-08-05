/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Structure tests for the in-session companion summon UI (设计 B5) — same
 * source-assertion style as `pages/nomi/workspace/tabs/MemoryTab`.
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');

describe('SummonPanel structure', () => {
  test('talks to the summon lifecycle endpoints through the bridge', () => {
    expect(source.includes('ipcBridge.conversation.setSummon.invoke')).toBe(true);
    expect(source.includes('ipcBridge.conversation.clearSummon.invoke')).toBe(true);
  });

  test('memory picker rides the A-track FTS retrieval face, scoped to the companion', () => {
    expect(source.includes('ipcBridge.companion')).toBe(true);
    expect(source.includes('listMemories.invoke')).toBe(true);
    expect(source.includes('scope_companion_id: companionId')).toBe(true);
    expect(source.includes("status: 'all'")).toBe(true);
    expect(source.includes("'relevance'")).toBe(true);
  });

  test('mirrors the backend snapshot budget and shows a meter', () => {
    expect(source.includes('SUMMON_CONTEXT_BUDGET = 8000')).toBe(true);
    expect(source.includes('summon-budget-meter')).toBe(true);
  });

  test('three-step flow: companion cards, skill exclusions, memory multi-select', () => {
    expect(source.includes('summon-companion-card')).toBe(true);
    expect(source.includes('excludedSkills')).toBe(true);
    expect(source.includes('selectedMemoryIds')).toBe(true);
    expect(source.includes('conversation.summon.stepCompanion')).toBe(true);
    expect(source.includes('conversation.summon.stepSkills')).toBe(true);
    expect(source.includes('conversation.summon.stepMemories')).toBe(true);
  });

  test('selected companion card is visibly highlighted (solid border + tinted fill)', () => {
    // This app's UnoCSS preflight carries no Tailwind-style reset, so a bare
    // `border` utility sets only border-width and paints nothing — the card
    // MUST spell out `border-solid` or neither the idle nor the selected
    // border ever renders and the picker looks selection-less.
    expect(source.includes('border-solid')).toBe(true);
    // Border color alone is a 1px signal; the selected card also gets a
    // primary-tinted fill so the chosen companion is obvious at a glance.
    expect(source.includes('bg-[rgba(var(--primary-6),0.08)]')).toBe(true);
    expect(source.includes("data-selected=")).toBe(true);
  });

  test('companion prefill follows a late-arriving roster instead of staying empty', () => {
    // The drawer can open before the roster SWR resolves; the prefill must
    // re-run when companions arrive, without overriding a manual pick.
    expect(source.includes('current ??')).toBe(true);
  });

  test('summoned state exposes update + release, and 409 maps to the busy toast', () => {
    expect(source.includes('summon-release-button')).toBe(true);
    expect(source.includes('summon-apply-button')).toBe(true);
    expect(source.includes('error.status === 409')).toBe(true);
    expect(source.includes('conversation.summon.busy')).toBe(true);
  });

  test('effective skills mirror normalized_effective_skill_names (auto minus disabled plus enabled)', () => {
    expect(source.includes('listBuiltinAutoSkills')).toBe(true);
    expect(source.includes('disabled_auto')).toBe(true);
  });

  test('never offers a direct memory write from the work session (read-only boundary)', () => {
    expect(source.includes('save_memory')).toBe(false);
    expect(source.includes('addMemory')).toBe(false);
  });
});

describe('summon surface integration', () => {
  test('NomiSendBox renders the summon control inside the config group', () => {
    const sendbox = readFileSync(
      new URL('../../platforms/nomi/NomiSendBox.tsx', import.meta.url),
      'utf8'
    );
    expect(sendbox.includes('<SummonControl conversationId={conversation_id} />')).toBe(true);
  });

  test('header + sidebar carry the summon badge', () => {
    const header = readFileSync(new URL('../ChatLayout/index.tsx', import.meta.url), 'utf8');
    expect(header.includes('<SummonHeaderBadge conversationId={conversation_id} />')).toBe(true);
    const items = readFileSync(
      new URL('../../SessionList/utils/sessionCapabilityItems.tsx', import.meta.url),
      'utf8'
    );
    expect(items.includes("key: 'summon'")).toBe(true);
    const row = readFileSync(new URL('../../SessionList/ConversationRow.tsx', import.meta.url), 'utf8');
    expect(row.includes('summoned')).toBe(true);
  });
});
