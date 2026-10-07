/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterAll, afterEach, beforeAll, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { SWRConfig } from 'swr';
import messages from '@/renderer/services/i18n/locales/zh-CN/index';
import ModalityModelsPanel from './ModalityModelsPanel';
import { MODALITY_SPECS, type ModalityKey } from './modalityModels';
import { modelAdditionTask } from './modelAdditionIntent';

// Use the complete configured-provider query without opening a real backend.
const realWebSocket = globalThis.WebSocket;
class IntentFixtureWebSocket extends EventTarget {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  readyState = IntentFixtureWebSocket.CONNECTING;
  send() {}
  close() { this.readyState = IntentFixtureWebSocket.CLOSED; }
}
beforeAll(() => { globalThis.WebSocket = IntentFixtureWebSocket as unknown as typeof WebSocket; });
afterAll(() => { globalThis.WebSocket = realWebSocket; });
afterEach(cleanup);

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN',
  resources: { 'zh-CN': { translation: messages } },
});

const RouteState = () => {
  const location = useLocation();
  return <output data-testid='addition-route'>{location.pathname}{location.search}</output>;
};

test('actual specialized management buttons keep ASR, TTS, and every other requested task', () => {
  for (const [modality, spec] of Object.entries(MODALITY_SPECS)) {
    const page = render(
      <I18nextProvider i18n={i18n}>
        <MemoryRouter initialEntries={[`/models?section=${modality}`]}>
          <SWRConfig value={{ provider: () => new Map(), fallback: { providers: [] }, revalidateOnMount: false }}>
            <RouteState />
            <ModalityModelsPanel
              modality={modality as ModalityKey}
              titleKey='settings.modelHub.title'
              subtitleKey='settings.modelHub.subtitle'
            />
          </SWRConfig>
        </MemoryRouter>
      </I18nextProvider>
    );
    fireEvent.click(page.getByRole('button', { name: i18n.t('settings.modelHub.modality.manageModels') }));
    const route = page.getByTestId('addition-route').textContent ?? '';
    expect(route.startsWith('/models?section=models&')).toBe(true);
    expect(modelAdditionTask(new URLSearchParams(route.slice(route.indexOf('?') + 1)))).toBe(spec.task);
    cleanup();
  }
});
