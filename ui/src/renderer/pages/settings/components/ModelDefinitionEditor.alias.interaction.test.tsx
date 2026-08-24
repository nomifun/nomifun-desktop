/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import React, { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import ModelDefinitionEditor from './ModelDefinitionEditor';
import type { ModelDefinitionDraft } from './providerModelAdvanced';

const EMPTY_LIST: never[] = [];
const EMPTY_MANIFESTS = {};
const EMPTY_VALIDATION_ERRORS: never[] = [];

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: { settings: zhSettings } } },
  interpolation: { escapeValue: false },
});

afterEach(() => {
  cleanup();
});

const AliasHarness: React.FC = () => {
  const [value, setValue] = useState<ModelDefinitionDraft>({
    model: '',
    capabilities: [],
  });

  return (
    <I18nextProvider i18n={testI18n}>
      <ModelDefinitionEditor
        value={value}
        onChange={setValue}
        providerBaseUrl=''
        providerAuthScheme='bearer'
        manifests={EMPTY_MANIFESTS}
        manifestLoadingTasks={EMPTY_LIST}
        manifestErrorTasks={EMPTY_LIST}
        validationErrors={EMPTY_VALIDATION_ERRORS}
        existingModelIds={EMPTY_LIST}
        catalogSuggestions={EMPTY_LIST}
        connections={EMPTY_LIST}
      />
    </I18nextProvider>
  );
};

describe('ModelDefinitionEditor model alias disclosure', () => {
  test('renders the alias input only after expansion', () => {
    const { container, getByLabelText } = render(<AliasHarness />);
    const currentDisclosure = () =>
      container.querySelector<HTMLButtonElement>('button[data-model-alias-disclosure]');
    const disclosure = currentDisclosure();

    expect(disclosure).not.toBeNull();
    expect(disclosure?.getAttribute('aria-expanded')).toBe('false');
    expect(disclosure?.getAttribute('aria-label')).toBe('添加模型别名');
    expect(disclosure?.getAttribute('data-model-alias-configured')).toBe('false');
    expect(container.querySelector('[data-model-alias-input]')).toBeNull();

    fireEvent.click(disclosure!);
    expect(disclosure?.getAttribute('aria-expanded')).toBe('true');

    const aliasInput = getByLabelText('模型别名（选填）') as HTMLInputElement;
    expect(aliasInput.value).toBe('');

    fireEvent.keyDown(aliasInput, { key: 'Escape' });
    expect(currentDisclosure()?.getAttribute('aria-expanded')).toBe('false');
    expect(container.querySelector('[data-model-alias-input]')).toBeNull();

    fireEvent.click(currentDisclosure()!);
    expect(currentDisclosure()?.getAttribute('aria-expanded')).toBe('true');
    expect((getByLabelText('模型别名（选填）') as HTMLInputElement).value).toBe('');
  });
});
