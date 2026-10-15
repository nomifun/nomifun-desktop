/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
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
  test('keeps automatic, Responses, Chat Completions, and Claude presets visible together', () => {
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <ProviderCompatibilityModePicker value='anthropic' onChange={() => undefined} />
      </I18nextProvider>
    );

    expect(html.includes('data-provider-compatibility-mode="true"')).toBe(true);
    expect(html.includes('data-provider-compatibility-option="auto"')).toBe(true);
    expect(html.includes('data-provider-compatibility-option="openai"')).toBe(true);
    expect(html.includes('data-provider-compatibility-option="openai-responses"')).toBe(true);
    expect(html.includes('data-provider-compatibility-option="anthropic"')).toBe(true);
    expect(html.includes('自动识别')).toBe(true);
    expect(html.includes('OpenAI Responses')).toBe(true);
    expect(html.includes('Chat Completions')).toBe(true);
    expect(html.includes('Claude 兼容')).toBe(true);
    expect(html.includes('anthropic.messages')).toBe(true);
    expect(html.includes('x-api-key')).toBe(true);
  });

  test('explains the distinct Responses endpoint when explicitly selected', () => {
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <ProviderCompatibilityModePicker value='openai-responses' onChange={() => undefined} />
      </I18nextProvider>
    );
    expect(html).toContain('/responses');
    expect(html).toContain('网关需明确支持此接口');
  });
});
