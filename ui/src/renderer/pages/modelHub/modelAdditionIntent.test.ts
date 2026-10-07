/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { MODEL_TASK_ORDER } from '@/common/modelCapabilities';
import { MODALITY_SPECS } from './modalityModels';
import {
  modelAdditionTask,
  modelProviderManagementRoute,
  withoutModelAdditionTask,
} from './modelAdditionIntent';

const routeParams = (route: string): URLSearchParams =>
  new URLSearchParams(route.slice(route.indexOf('?') + 1));
const read = (relative: string): string =>
  readFileSync(new URL(relative, import.meta.url), 'utf8');

describe('model addition task intent', () => {
  test('every specialized page carries its own endpoint task to provider management', () => {
    for (const spec of Object.values(MODALITY_SPECS)) {
      const params = routeParams(modelProviderManagementRoute(spec.task));
      expect(params.get('section')).toBe('models');
      expect(modelAdditionTask(params)).toBe(spec.task);
    }
    expect(modelAdditionTask(routeParams(modelProviderManagementRoute(MODALITY_SPECS.asr.task))))
      .toBe('speech_recognition');
    expect(modelAdditionTask(routeParams(modelProviderManagementRoute(MODALITY_SPECS.tts.task))))
      .toBe('speech_synthesis');
    expect(MODALITY_SPECS.vision.task).toBe('chat');
  });

  test('generic management, invalid tasks, and unrelated sections do not imply Chat', () => {
    expect(modelAdditionTask(routeParams(modelProviderManagementRoute()))).toBeUndefined();
    expect(modelAdditionTask(new URLSearchParams('section=models&addTask=unknown'))).toBeUndefined();
    expect(modelAdditionTask(new URLSearchParams('section=chat&addTask=speech_recognition')))
      .toBeUndefined();
    expect(modelAdditionTask(new URLSearchParams('addTask=chat'))).toBeUndefined();
    for (const task of MODEL_TASK_ORDER) {
      expect(modelAdditionTask(routeParams(modelProviderManagementRoute(task)))).toBe(task);
    }
  });

  test('explicit sidebar navigation clears an earlier use case without mutating other route state', () => {
    const params = new URLSearchParams('section=models&addTask=speech_recognition&from=guid');
    const next = withoutModelAdditionTask(params);
    expect(next.toString()).toBe('section=models&from=guid');
    expect(modelAdditionTask(next)).toBeUndefined();
    expect(params.get('addTask')).toBe('speech_recognition');
  });

  test('the actual page links and shared add dialogs forward task intent', () => {
    const modality = read('./ModalityModelsPanel.tsx');
    const asr = read('./SpeechToTextContent.tsx');
    const hub = read('./index.tsx');
    const providers = read('../../components/settings/SettingsModal/contents/ModelModalContent.tsx');

    expect(modality.includes('navigate(modelProviderManagementRoute(MODALITY_SPECS[modality].task))'))
      .toBe(true);
    expect(asr.includes("navigate(modelProviderManagementRoute('speech_recognition'))")).toBe(true);
    expect(hub.includes('const next = withoutModelAdditionTask(searchParams)')).toBe(true);
    expect(providers.includes('const initialTask = modelAdditionTask(searchParams)')).toBe(true);
    expect(providers.includes('addModelModalCtrl.open({ data: platform, initialTask })')).toBe(true);
    expect(providers.includes('addPlatformModalCtrl.open({ initialTask, deepLinkData: undefined })'))
      .toBe(true);
    expect(providers.includes('deepLinkData: pending, initialTask: pending.task ?? initialTask'))
      .toBe(true);
  });
});
