/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import React from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { renderToStaticMarkup } from 'react-dom/server';
import type { AssetId } from '@/common/types/ids';
import enWorkshopGeneration from '@/renderer/services/i18n/locales/en-US/workshopGeneration.json';
import type { GenMode } from './genTypes';
import ResultView from './ResultView';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': {
      translation: {
        workshopGeneration: enWorkshopGeneration,
      },
    },
  },
  interpolation: { escapeValue: false },
});

const RESULT_ASSET_ID = '019feebc-3f84-7400-ab90-f29fda42725e' as AssetId;

const renderResult = (mode: GenMode): string =>
  renderToStaticMarkup(
    <I18nextProvider i18n={testI18n}>
      <ResultView
        mode={mode}
        resultAssetIds={[RESULT_ASSET_ID]}
        onContinueEdit={() => undefined}
        onToTextNode={() => undefined}
      />
    </I18nextProvider>
  );

describe('workshop generation result actions', () => {
  test('keeps Continue editing available for image results', () => {
    expect(renderResult('image').includes('data-workshop-continue-edit')).toBe(true);
  });

  test('does not offer unsupported Continue editing for video results', () => {
    expect(renderResult('video').includes('data-workshop-continue-edit')).toBe(false);
  });
});
