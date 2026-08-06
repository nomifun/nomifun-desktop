/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const readLocalSource = (fileName: string) =>
  readFileSync(join(dirname(fileURLToPath(import.meta.url)), fileName), 'utf8');

describe('workpath section toolbar structure', () => {
  test('keeps every creation action in one grid and mode switches out of it', () => {
    const createBarSource = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), '../components/ConversationShell/SessionCreateBar.tsx'),
      'utf8'
    );
    const actionGridIndex = createBarSource.indexOf("data-testid='session-action-grid'");
    const newChatIndex = createBarSource.indexOf("data-testid='session-new-conversation-entry'");
    const newTerminalIndex = createBarSource.indexOf("data-testid='session-new-terminal-entry'");
    const createProjectIndex = createBarSource.indexOf("data-testid='workpath-create-project-btn'");
    const remoteIndex = createBarSource.indexOf('<RemoteSessionPopover');
    const batchIndex = createBarSource.indexOf("data-testid='workpath-batch-select-btn'");
    const searchIndex = createBarSource.indexOf('<ConversationSearchPopover');

    expect(actionGridIndex).toBeGreaterThan(-1);
    for (const index of [newChatIndex, newTerminalIndex, createProjectIndex, remoteIndex]) {
      expect(index).toBeGreaterThan(actionGridIndex);
    }
    expect(searchIndex).toBeGreaterThan(remoteIndex);

    // Batch selection is a mode switch, not a creation. Keeping it in the title
    // strip is what freed the grid's fourth cell for remote sessions, so moving
    // it back into the grid has to fail here rather than silently cost that cell.
    expect(batchIndex).toBeGreaterThan(-1);
    expect(batchIndex).toBeLessThan(actionGridIndex);
    expect(createBarSource.includes("t(batchMode ? 'sessionList.exitBatchSelect' : 'sessionList.batchSelect')")).toBe(true);
  });

  test('labels the create actions as icon-verb plus noun, with the full phrase on aria-label', () => {
    const createBarSource = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), '../components/ConversationShell/SessionCreateBar.tsx'),
      'utf8'
    );

    // The `+` is the verb for all four, so a per-button "新建…" label would say it
    // twice — and that duplication is what kept the grid from holding a fourth
    // action at the 240px minimum width.
    for (const key of ['actionChat', 'actionTerminal', 'actionProject']) {
      expect(createBarSource.includes(`t('sessionList.${key}')`)).toBe(true);
    }
    expect(createBarSource.includes("aria-label={t('terminal.newConversation')}")).toBe(true);
    expect(createBarSource.includes("aria-label={t('terminal.newTerminal')}")).toBe(true);
    expect(createBarSource.includes("aria-label={t('sessionList.createProject')}")).toBe(true);
  });

  test('keeps the workpath area as a section label without duplicated action buttons', () => {
    const source = readLocalSource('index.tsx');

    expect(source.includes("data-testid='workpath-section-toolbar'")).toBe(true);
    expect(source.includes("t('sessionList.workpathSection')")).toBe(true);
    expect(source.includes("data-testid='workpath-create-project-btn'")).toBe(false);
    expect(source.includes("data-testid='workpath-batch-select-btn'")).toBe(false);
  });

  test('routes project creation through the session shell instead of a hidden header icon', () => {
    const shellSource = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), '../components/ConversationShell/index.tsx'),
      'utf8'
    );
    const createBarSource = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), '../components/ConversationShell/SessionCreateBar.tsx'),
      'utf8'
    );

    expect(shellSource.includes('onCreateProject={handleCreateProject}')).toBe(true);
    expect(shellSource.includes("navigate('/guid', { state: { workspace: projectPath } })")).toBe(true);
    expect(createBarSource.includes('onCreateProject')).toBe(true);
    expect(createBarSource.includes('onToggleBatchMode')).toBe(true);
    expect(createBarSource.includes('ConversationSiderActions')).toBe(false);
  });

  test('does not backfill the project registry from existing session workpaths', () => {
    const source = readLocalSource('index.tsx');

    expect(source.includes('migrateProjectWorkpaths')).toBe(false);
  });
});
