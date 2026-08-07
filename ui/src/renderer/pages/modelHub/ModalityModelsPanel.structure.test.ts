/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import enSettings from '@/renderer/services/i18n/locales/en-US/settings.json';

const panel = readFileSync(new URL('./ModalityModelsPanel.tsx', import.meta.url), 'utf8');
const chat = readFileSync(new URL('./ChatModelsContent.tsx', import.meta.url), 'utf8');
const vision = readFileSync(new URL('./VisionModelsContent.tsx', import.meta.url), 'utf8');
const embedding = readFileSync(new URL('./EmbeddingModelsContent.tsx', import.meta.url), 'utf8');

const MODALITY_KEYS = [
  'chatTitle',
  'chatSubtitle',
  'visionTitle',
  'visionSubtitle',
  'embeddingTitle',
  'embeddingSubtitle',
  'modelCount',
  'empty',
  'emptyHint',
  'manageModels',
  'toggleFailed',
  'descriptionPlaceholder',
  'descriptionSave',
  'descriptionFailed',
  'defaultRow',
  'chatDefaultHint',
  'noDefault',
  'untaggedTitle',
  'untaggedHint',
  'taskChat',
  'taskEmbedding',
  'taskRerank',
  'traitVision',
] as const;

describe('modality panel', () => {
  test('lists catalog ROWS so a disabled model is still reachable', () => {
    expect(panel.includes('providerModel.list')).toBe(true);
    expect(panel.includes('buildModalityGroups')).toBe(true);
    // Resolve is for selectors, not for a management list.
    expect(panel.includes('useModelsForTask')).toBe(false);
  });

  test('provider groups come from the ONE shared selector ordering', () => {
    // `useProvidersQuery` is the raw backend order, and that order LEADS with the
    // managed free provider (auto-created before the user configures anything).
    // Reaching for it here made the hub's own sections contradict every model
    // selector in the app, and put models the user never configured in the first
    // slot of a management view. `useModelProviderList` already filters disabled
    // providers and applies `orderModelSelectorProviders` (free last).
    expect(panel.includes('useModelProviderList()')).toBe(true);
    expect(panel.includes('useProvidersQuery(')).toBe(false);
  });

  test('a row can be switched on and off in place', () => {
    expect(panel.includes('providerModel.update')).toBe(true);
    expect(panel.includes('<Switch')).toBe(true);
    expect(panel.includes("t('settings.modelHub.modality.toggleFailed')")).toBe(true);
  });

  test('task tagging is NOT re-implemented here; it links to the one editor', () => {
    // Duplicating the tasks/traits editor would create a second write path for
    // the same row — the exact double-write shape this repo already carries as
    // known debt.
    expect(panel.includes('ModelModalityEditor')).toBe(false);
    expect(panel.includes("navigate('/models?section=models')")).toBe(true);
  });

  test('only 对话 carries a modality default, and it writes the existing key', () => {
    expect(chat.includes('showDefaultModel')).toBe(true);
    expect(vision.includes('showDefaultModel')).toBe(false);
    expect(embedding.includes('showDefaultModel')).toBe(false);
    expect(panel.includes("'nomi.defaultModel'")).toBe(true);
    expect(panel.includes('<TaskModelSelect')).toBe(true);
    expect(panel.includes("t('settings.modelHub.modality.noDefault')")).toBe(true);
  });

  test('untagged rows surface in the chat section only', () => {
    expect(chat.includes('showUntagged')).toBe(true);
    expect(panel.includes('buildUntaggedGroups')).toBe(true);
  });

  test('copy exists in both locales', () => {
    for (const locale of [zhSettings, enSettings]) {
      const modality = (locale as unknown as { modelHub: { modality: Record<string, string> } })
        .modelHub.modality;
      for (const key of MODALITY_KEYS) {
        expect(typeof modality[key]).toBe('string');
        expect(modality[key].trim().length > 0).toBe(true);
      }
    }
  });
});
