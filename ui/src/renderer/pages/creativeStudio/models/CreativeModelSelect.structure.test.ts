/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import i18next from 'i18next';
import { readFileSync } from 'node:fs';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import type { IProvider } from '@/common/config/storage';
import { parseProviderId } from '@/common/types/ids';
import CreativeModelSelect from './CreativeModelSelect';

const component = readFileSync(new URL('./CreativeModelSelect.tsx', import.meta.url), 'utf8');
const adapter = readFileSync(new URL('./useNomiCreativeModelCatalog.ts', import.meta.url), 'utf8');
const testI18n = i18next.createInstance();

await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: {
    'zh-CN': {
      translation: {
        settings: {
          modelHub: {
            free: {
              title: 'NomiFun Free Model',
            },
          },
        },
      },
    },
  },
});

describe('CreativeModelSelect integration boundary', () => {
  test('the view is controlled and does not own provider fetching', () => {
    expect(component.includes('catalog: CreativeModelCatalogSnapshot')).toBe(true);
    expect(component.includes('value: CreativeModelSelectionRef | null')).toBe(true);
    expect(component.includes('onChange: (selection: CreativeModelOption) => void')).toBe(true);
    expect(component.includes('getPopupContainer?: () => HTMLElement')).toBe(true);
    expect(component.includes('getPopupContainer={getPopupContainer}')).toBe(true);
    expect(component.includes("state === 'ready' ? copy.placeholder : stateCopy(state, copy)")).toBe(
      true
    );
    expect(component.includes('useProvidersQuery')).toBe(false);
  });

  test('the NomiFun adapter is the only provider-query connection', () => {
    expect(adapter.includes('useProvidersQuery()')).toBe(true);
    expect(adapter.includes('adaptCreativeModelCatalog')).toBe(true);
    expect(adapter.includes('fetch(')).toBe(false);
  });

  test('all required view states and a disabled stale selection are explicit', () => {
    for (const state of [
      'loading',
      'no-provider',
      'no-compatible-model',
      'disabled',
      'error',
      'ready',
    ]) {
      expect(component.includes(`'${state}'`)).toBe(true);
    }
    expect(component.includes('<NomiSelect.Option value={optionKey(value)} disabled>')).toBe(true);
    expect(component.includes("role={status === 'error' ? 'alert' : 'status'}")).toBe(true);
  });

  test('puts an empty-catalog explanation in both the field and status region', () => {
    const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-00000000000a');
    const chatOnlyProvider: IProvider = {
      id: providerId,
      platform: 'custom',
      name: 'Chat only',
      base_url: 'https://example.invalid/v1',
      auth_scheme: 'bearer',
      has_credentials: true,
      enabled: true,
      models: [
        {
          provider_id: providerId,
          model: 'chat-only',
          enabled: true,
          sort_order: 0,
          capabilities: [
            {
              task: 'chat',
              traits: [],
              protocol: 'openai.chat_text',
              connection_role: 'default',
              allow_cross_origin_credentials: false,
              provider_params: {},
              created_at: 1,
              updated_at: 1,
            },
          ],
          created_at: 1,
          updated_at: 1,
        },
      ],
    };
    const html = renderToStaticMarkup(
      React.createElement(
        I18nextProvider,
        { i18n: testI18n },
        React.createElement(CreativeModelSelect, {
          catalog: {
            status: 'ready',
            providers: [chatOnlyProvider],
            error: null,
          },
          filter: { capability: 'task', task: 'image_generation' },
          value: null,
          onChange: () => undefined,
          copy: {
            noCompatibleModel: 'NO_IMAGE_MODEL',
            configureModels: 'CONFIGURE_IMAGE_MODEL',
          },
          onOpenModelSettings: () => undefined,
        })
      )
    );

    expect(html.includes('data-state="no-compatible-model"')).toBe(true);
    expect(html.match(/NO_IMAGE_MODEL/g)?.length ?? 0).toBeGreaterThanOrEqual(2);
    expect(html.includes('role="status"')).toBe(true);
    expect(html.includes('CONFIGURE_IMAGE_MODEL')).toBe(true);
    expect(html.includes('arco-select-view-disabled')).toBe(true);
  });
});
