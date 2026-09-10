/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import en from '@/renderer/services/i18n/locales/en-US/miniApps.json';
import { MiniAppStatePanel } from './MiniAppM1State';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': {
      translation: {
        miniApps: en,
      },
    },
  },
  interpolation: { escapeValue: false },
});

afterEach(() => cleanup());

const renderPanel = (children: React.ReactNode) =>
  render(
    <I18nextProvider i18n={i18n}>{children}</I18nextProvider>
  );

describe('MiniApp state panel accessibility and recovery', () => {
  test('announces loading as a busy status and hides decorative artwork', () => {
    const view = renderPanel(
      <MiniAppStatePanel
        loading
        title=''
        body='Loading the workbench'
      />
    );

    const panel = view.getByRole('status');
    expect(panel.getAttribute('aria-busy')).toBe('true');
    expect(panel.textContent?.includes('Loading the workbench')).toBe(true);
  });

  test('announces errors and keeps a keyboard-usable recovery action', () => {
    let retries = 0;
    const view = renderPanel(
      <MiniAppStatePanel
        title='Could not load'
        body='The request failed'
        actionLabel='Try again'
        onRetry={() => {
          retries += 1;
        }}
      />
    );

    const panel = view.getByRole('alert', { name: /Could not load/ });
    const retry = view.getByRole('button', { name: 'Try again' });
    fireEvent.click(retry);

    expect(panel.textContent?.includes('The request failed')).toBe(true);
    expect(retries).toBe(1);
  });
});
