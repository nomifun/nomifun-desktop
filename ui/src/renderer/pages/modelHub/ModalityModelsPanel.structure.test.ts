/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import enSettings from '@/renderer/services/i18n/locales/en-US/settings.json';

const read = (file: string) => readFileSync(new URL(file, import.meta.url), 'utf8');
const panel = read('./ModalityModelsPanel.tsx');
const chat = read('./ChatModelsContent.tsx');
const realtime = read('./RealtimeModelsContent.tsx');
const image = read('./ImageModelsContent.tsx');
const imageEdit = read('./ImageEditModelsContent.tsx');
const embedding = read('./EmbeddingModelsContent.tsx');
const rerank = read('./RerankModelsContent.tsx');
const asr = read('./SpeechToTextContent.tsx');
const tts = read('./TextToSpeechContent.tsx');

const MODALITY_KEYS = [
  'chatTitle',
  'chatSubtitle',
  'realtimeTitle',
  'realtimeSubtitle',
  'visionTitle',
  'visionSubtitle',
  'imageEditTitle',
  'imageEditSubtitle',
  'ttsTitle',
  'ttsSubtitle',
  'asrTitle',
  'asrSubtitle',
  'embeddingTitle',
  'embeddingSubtitle',
  'rerankTitle',
  'rerankSubtitle',
  'modelCount',
  'empty',
  'emptyHint',
  'manageModels',
  'toggleFailed',
  'descriptionPlaceholder',
  'descriptionSave',
  'descriptionFailed',
  'modelDisabled',
  'defaultRow',
  'chatDefaultHint',
  'traitVision',
] as const;

describe('modality panel', () => {
  test('lists nested provider models so disabled rows stay reachable', () => {
    expect(panel.includes('useProvidersQuery()')).toBe(true);
    expect(panel.includes('buildModalityGroups(providers')).toBe(true);
    expect(panel.includes('providerModel.list')).toBe(false);
    expect(panel.includes('useModelsForTask')).toBe(false);
    expect(panel.includes('modelDisabled')).toBe(true);
  });

  test('uses the full-replacement model save route', () => {
    expect(panel.includes('providerModel.save.invoke')).toBe(true);
    expect(panel.includes('toProviderModelInput(definition)')).toBe(true);
    expect(panel.includes('model: toProviderModelInput(definition)')).toBe(true);
    expect(panel.includes('<Switch')).toBe(true);
  });

  test('all endpoint tasks remain independent', () => {
    expect(realtime.includes("modality='realtime'")).toBe(true);
    expect(image.includes("modality='image'")).toBe(true);
    expect(imageEdit.includes("modality='image_edit'")).toBe(true);
    expect(embedding.includes("modality='embedding'")).toBe(true);
    expect(rerank.includes("modality='rerank'")).toBe(true);
    expect(asr.includes("modality='asr'")).toBe(true);
    expect(tts.includes("modality='tts'")).toBe(true);
  });

  test('only chat carries the conversation default model', () => {
    expect(chat.includes('showDefaultModel')).toBe(true);
    expect(realtime.includes('showDefaultModel')).toBe(false);
    expect(panel.includes("'nomi.defaultModel'")).toBe(true);
    expect(panel.includes('<TaskModelSelect')).toBe(true);
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
