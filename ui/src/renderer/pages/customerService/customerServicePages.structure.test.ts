/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const rosterSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const detailSource = readFileSync(new URL('./CsAgentDetailPage.tsx', import.meta.url), 'utf8');
const detailStyles = readFileSync(new URL('./CsAgentDetailPage.module.css', import.meta.url), 'utf8');
const createSource = readFileSync(new URL('./CreateCsAgentModal.tsx', import.meta.url), 'utf8');
const createStyles = readFileSync(new URL('./CreateCsAgentModal.module.css', import.meta.url), 'utf8');
const botsSectionSource = readFileSync(new URL('./CsChannelBotsSection.tsx', import.meta.url), 'utf8');

describe('customer service pages structure', () => {
  test('roster navigates by cs_agent_id business id', () => {
    expect(rosterSource.includes("navigate(`/customer-service/${csAgentId}`)")).toBe(true);
    expect(rosterSource.includes('useCsAgents()')).toBe(true);
    expect(rosterSource.includes('CreateCsAgentModal')).toBe(true);
  });

  test('roster never references the retired public-agent surface', () => {
    for (const source of [rosterSource, detailSource, createSource]) {
      expect(source.includes('publicAgent')).toBe(false);
      expect(source.includes('public-companions')).toBe(false);
    }
  });

  test('detail page delegates channel-bot management to the self-closed section', () => {
    expect(detailSource.includes('CsChannelBotsSection')).toBe(true);
    // The old cross-domain shared pool (raw getPluginStatus consumption) is gone.
    expect(detailSource.includes('channel.getPluginStatus')).toBe(false);
    expect(detailSource.includes('companionBound')).toBe(false);
  });

  test('bots section manages bindings via the full-replacement PUT contract', () => {
    expect(botsSectionSource.includes('ipcBridge.customerService.replaceBindings.invoke')).toBe(true);
    expect(botsSectionSource.includes('channel_plugin_ids:')).toBe(true);
  });

  test('bots section is a customer-service-domain self-closed loop', () => {
    // Only cs-domain bots are listed; companion bots never enter the pool.
    expect(botsSectionSource.includes('selectCsChannelBots')).toBe(true);
    // In-page creation reuses the shared platform config machinery, addressed
    // to the customer-service domain and never carrying a companion binding.
    expect(botsSectionSource.includes('PlatformConfigBody')).toBe(true);
    expect(botsSectionSource.includes("ownerDomain: 'customer_service'")).toBe(true);
    expect(botsSectionSource.includes('companionId')).toBe(false);
    // A bot created inside the modal is adopted and auto-bound to this agent.
    expect(botsSectionSource.includes('findNewlyCreatedCsBot')).toBe(true);
  });

  test('detail page exposes notes CRUD against the notes REST surface', () => {
    expect(detailSource.includes('ipcBridge.customerService.listNotes.invoke')).toBe(true);
    expect(detailSource.includes('ipcBridge.customerService.createNote.invoke')).toBe(true);
    expect(detailSource.includes('ipcBridge.customerService.removeNote')).toBe(true);
    expect(detailSource.includes('ipcBridge.customerService.patchNote')).toBe(true);
  });

  test('notes list supports compact previews, management actions, and pagination', () => {
    expect(detailStyles.includes('-webkit-line-clamp: 3')).toBe(true);
    expect(detailSource.includes("<Menu.Item key='view'>")).toBe(true);
    expect(detailSource.includes("<Menu.Item key='edit'>")).toBe(true);
    expect(detailSource.includes("<Menu.Item key='delete'>")).toBe(true);
    expect(detailSource.includes('pageSize: NOTE_PAGE_SIZE')).toBe(true);
  });

  test('create modal reuses the shared model and knowledge catalogs', () => {
    // Chat-filtered catalog hook (P3): the model list comes from resolve, not raw provider rows.
    expect(createSource.includes('useModelsForTask')).toBe(true);
    expect(createSource.includes('useKnowledgeBaseOptions')).toBe(true);
    expect(createSource.includes("max={64}")).toBe(true);
  });

  test('create modal uses a compact centered shell and reference-style horizontal form', () => {
    expect(createSource.includes('className={styles.modal}')).toBe(true);
    expect(createSource.includes('alignCenter')).toBe(true);
    expect(createSource.includes('style={{ width: 520 }}')).toBe(false);
    expect(createSource.match(/className=\{styles\.formRow\}/g)?.length).toBe(7);
    expect(createSource.includes('className={styles.modelFields}')).toBe(true);
    expect(createStyles.includes('display: inline-flex !important')).toBe(true);
    expect(createStyles.includes('width: min(680px, calc(100vw - 32px))')).toBe(true);
    expect(createStyles.includes('max-height: calc(100vh - 48px)')).toBe(true);
    expect(createStyles.includes('height: min(620px')).toBe(false);
    expect(createStyles.includes('grid-template-columns: 88px minmax(0, 1fr)')).toBe(true);
    expect(createStyles.includes('grid-template-columns: repeat(2, minmax(0, 1fr))')).toBe(true);
    expect(createStyles.includes('gap: 8px')).toBe(true);
    expect(createStyles.includes('overflow-y: auto')).toBe(true);
  });

  test('detail model selects fit their selected labels without overflowing narrow cards', () => {
    expect(detailSource.match(/contentMaxWidth='100%'/g)?.length).toBe(2);
    expect(detailSource.includes('contentMinWidth={132}')).toBe(false);
    expect(detailSource.includes('contentMinWidth={116}')).toBe(false);
    expect(detailSource.match(/size='small'/g)?.length).toBeGreaterThanOrEqual(2);
    expect(detailStyles.includes('flex-wrap: nowrap')).toBe(true);
    expect(detailStyles.includes('justify-content: flex-end')).toBe(true);
    expect(detailStyles.includes('gap: 6px')).toBe(true);
    expect(detailStyles.includes('flex: 1 1 0 !important')).toBe(true);
    expect(detailStyles.match(/grid-template-columns: 76px minmax\(0, 1fr\)/g)?.length).toBeGreaterThanOrEqual(2);
    expect(detailStyles.includes('font-size: 12px')).toBe(true);
  });
});
