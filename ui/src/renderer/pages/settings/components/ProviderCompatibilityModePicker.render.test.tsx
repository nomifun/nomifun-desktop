/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import ProviderCompatibilityModePicker from './ProviderCompatibilityModePicker';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: { settings: zhSettings } } },
  interpolation: { escapeValue: false },
});

describe('provider compatibility mode picker', () => {
  test('keeps automatic, OpenAI, and Claude presets visible together', () => {
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <ProviderCompatibilityModePicker value='anthropic' onChange={() => undefined} />
      </I18nextProvider>
    );

    expect(html.includes('data-provider-compatibility-mode="true"')).toBe(true);
    expect(html.includes('data-provider-compatibility-option="auto"')).toBe(true);
    expect(html.includes('data-provider-compatibility-option="openai"')).toBe(true);
    expect(html.includes('data-provider-compatibility-option="anthropic"')).toBe(true);
    expect(html.includes('自动识别')).toBe(true);
    expect(html.includes('OpenAI 兼容')).toBe(true);
    expect(html.includes('Claude 兼容')).toBe(true);
    expect(html.includes('anthropic.messages')).toBe(true);
    expect(html.includes('x-api-key')).toBe(true);
  });
});
