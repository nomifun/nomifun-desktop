/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const rosterSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const detailSource = readFileSync(new URL('./CsAgentDetailPage.tsx', import.meta.url), 'utf8');
const createSource = readFileSync(new URL('./CreateCsAgentModal.tsx', import.meta.url), 'utf8');

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

  test('detail page manages bindings via the full-replacement PUT contract', () => {
    expect(detailSource.includes('ipcBridge.customerService.replaceBindings.invoke')).toBe(true);
    expect(detailSource.includes('channel_plugin_ids: next')).toBe(true);
  });

  test('detail page exposes notes CRUD against the notes REST surface', () => {
    expect(detailSource.includes('ipcBridge.customerService.listNotes.invoke')).toBe(true);
    expect(detailSource.includes('ipcBridge.customerService.createNote.invoke')).toBe(true);
    expect(detailSource.includes('ipcBridge.customerService.removeNote')).toBe(true);
    expect(detailSource.includes('ipcBridge.customerService.patchNote')).toBe(true);
  });

  test('create modal reuses the shared model and knowledge catalogs', () => {
    expect(createSource.includes('useModelProviderList')).toBe(true);
    expect(createSource.includes('useKnowledgeBaseOptions')).toBe(true);
    expect(createSource.includes("max={64}")).toBe(true);
  });
});
