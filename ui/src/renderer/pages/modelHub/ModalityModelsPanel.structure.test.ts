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
const pageHeader = read('./ModelHubPageHeader.tsx');
const free = read('./FreeModelsContent.tsx');
const providers = read('../../components/settings/SettingsModal/contents/ModelModalContent.tsx');
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
    // The management list reads the complete provider tree so disabled rows
    // remain visible. Only the default image picker asks for runnable models.
    expect(panel.includes("useModelsForTask('image_generation')")).toBe(true);
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

  test('page shells share the compact failover title treatment without a card background', () => {
    expect(pageHeader.includes("text-15px font-600 leading-20px text-t-primary")).toBe(true);
    expect(pageHeader.includes("text-12px leading-18px text-t-tertiary")).toBe(true);

    for (const surface of [panel, free, providers]) {
      expect(surface.includes('<ModelHubPageHeader')).toBe(true);
      expect(surface.includes("flex flex-col bg-2 rd-16px")).toBe(false);
      expect(surface.includes("flex min-h-0 flex-col rd-16px bg-2")).toBe(false);
    }

    for (const capability of [chat, realtime, image, imageEdit, embedding, rerank]) {
      expect(capability.includes('icon={<')).toBe(false);
    }
    expect(asr.includes('icon={<HeadsetOne')).toBe(false);
    expect(tts.includes('icon={<Voice')).toBe(false);
  });

  test('model labels do not visually outrank their provider categories', () => {
    expect(panel.includes("text-13px font-500 leading-18px text-t-primary truncate")).toBe(true);
    expect(panel.includes("text-14px font-600 text-t-primary truncate")).toBe(false);
    expect(panel.includes("truncate text-13px font-400 leading-18px")).toBe(true);
  });

  test('speech-recognition settings use the shared packaged setting-list layout', () => {
    expect(asr.includes('NomiSettingList')).toBe(true);
    expect(asr.match(/<NomiSettingRow/g)?.length).toBe(3);
    expect(asr.includes("className='mt-16px'")).toBe(true);
    expect(asr.includes('<Form')).toBe(false);
    expect(asr.includes('saveSpeechToTextConfig')).toBe(true);
    expect(asr.includes('actions={')).toBe(true);
    expect(asr.includes('description={sourceHint || undefined}')).toBe(true);
    expect(asr.includes('onHintChange={setSourceHint}')).toBe(true);
    expect(asr.includes("size='mini'")).toBe(true);
    expect(asr.includes('<NomiInput')).toBe(true);
    expect(asr.includes('contentFit')).toBe(true);
    expect(asr.includes('contentMinWidth={120}')).toBe(true);
    expect(asr.includes('contentMaxWidth={180}')).toBe(true);
    expect(asr.includes("className='compact-dark-switch shrink-0'")).toBe(true);
    expect(asr.includes("description={t('settings.modelHub.speech.languagePlaceholder')}")).toBe(
      false
    );
  });

  test('speech-synthesis settings use the same compact packaged layout', () => {
    expect(tts.includes('NomiSettingList')).toBe(true);
    expect(tts.match(/<NomiSettingRow/g)?.length).toBe(1);
    expect(tts.includes("className='mt-16px'")).toBe(true);
    expect(tts.includes('<Form')).toBe(false);
    expect(tts.includes("size='mini'")).toBe(true);
    expect(tts.includes('onHintChange={setSourceHint}')).toBe(false);
    expect(tts.includes('hideHint')).toBe(false);
    expect(tts.includes("description={t('settings.taskModel.voiceFreeTextHint')}")).toBe(true);
    expect(tts.includes("<Button size='mini' onClick={() => persist(null)}>")).toBe(true);
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
