/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';
import { cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import { cloneTemplateRunAggregate } from '../domain';
import { IDS, createTemplateRunFixture } from '../domain/testFixtures';
import TemplateRunCenter from './TemplateRunCenter';
import type { CreativeAsset } from '../../assets';

afterEach(cleanup);

const testI18n = createInstance();
testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: {} } },
  interpolation: { escapeValue: false },
});

describe('Template Run Center', () => {
  test('renders an available image result while preserving its original link and accessible name', async () => {
    const run = createTemplateRunFixture();
    run.record.status = 'succeeded';
    run.record.resultAssetIds = [IDS.asset];
    const originalUrl = `/api/creative-studio/files/${IDS.asset}`;
    const asset: CreativeAsset = {
      id: IDS.asset, kind: 'image', title: 'Portrait result', collection: null, tags: [],
      mimeType: 'image/png', width: 720, height: 1280, bytes: 1_024, inLibrary: true,
      textContent: null, origin: null, originalUrl, thumbnailUrl: null,
      createdAt: 1, updatedAt: 2,
    };
    const view = render(<I18nextProvider i18n={testI18n}><TemplateRunCenter port={{
      snapshot: { loading: false, loadError: null, runs: [run], activities: {} },
      assetReader: { get: async () => asset }, assetUrl: () => originalUrl,
      resume: async () => undefined, cancel: async () => undefined,
      review: async () => undefined, retry: async () => undefined,
    }} /></I18nextProvider>);

    await waitFor(() => expect(view.container.querySelector('img')).not.toBeNull());
    const image = view.container.querySelector('img')!;
    const link = image.closest('a')!;
    expect(image.getAttribute('src')).toBe(originalUrl);
    expect(image.getAttribute('alt')).toBe(`${run.templateSnapshot.metadata.name} result 1`);
    expect(link.getAttribute('href')).toBe(originalUrl);
    expect(link.getAttribute('target')).toBe('_blank');
    expect(link.getAttribute('rel')).toBe('noreferrer');
    expect(link.getAttribute('title')).toBe('View result 1');
  });

  test('renders durable status and recovery while awaiting result metadata', () => {
    const succeeded = createTemplateRunFixture();
    succeeded.revision = 4;
    succeeded.record.status = 'succeeded';
    succeeded.record.taskIds = [IDS.task];
    succeeded.record.resultAssetIds = [IDS.asset];
    succeeded.record.queuedAt = 2_100;
    succeeded.record.startedAt = 2_200;
    succeeded.record.completedAt = 2_300;
    const paused = cloneTemplateRunAggregate(succeeded);
    paused.request.id = IDS.idempotency;
    paused.request.idempotencyKey = IDS.idempotency;
    paused.record.requestId = IDS.idempotency;
    paused.revision = 3;
    paused.record.status = 'running';
    paused.record.resultAssetIds = [];
    paused.record.completedAt = null;

    const html = renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <TemplateRunCenter
          port={{
            snapshot: {
              loading: false,
              loadError: null,
              runs: [paused, succeeded],
              activities: {
                [IDS.idempotency]: {
                  state: 'paused',
                  taskStatuses: { [IDS.task]: 'running' },
                  error: 'network offline',
                },
              },
            },
            assetUrl: (assetId) => `/api/creative-studio/files/${assetId}`,
            resume: async () => undefined,
            cancel: async () => undefined,
            review: async () => undefined,
            retry: async () => undefined,
          }}
        />
      </I18nextProvider>
    );

    expect(html.includes('data-template-run-center="true"')).toBe(true);
    expect(html.includes('Template runs')).toBe(true);
    expect(html.includes('Waiting to resume')).toBe(true);
    expect(html.includes('Resume')).toBe(true);
    expect(html.includes('Completed')).toBe(true);
    expect(html.includes(`/api/creative-studio/files/${IDS.asset}`)).toBe(false);
    expect(html.includes('data-asset-media-state="loading"')).toBe(true);
    expect(html.includes('refresh or restart')).toBe(true);
  });

  test('renders a deleted template result as a placeholder without an image or download link', async () => {
    const run = createTemplateRunFixture();
    run.record.status = 'succeeded';
    run.record.resultAssetIds = [IDS.asset];
    const tombstone: CreativeAsset = {
      id: IDS.asset, kind: 'image', title: 'Old result', collection: null, tags: [],
      mimeType: 'image/png', width: 1, height: 1, bytes: 1, inLibrary: false,
      textContent: null, origin: null, originalUrl: '', thumbnailUrl: null,
      createdAt: 1, updatedAt: 2, deletedAt: 2,
    };
    const view = render(<I18nextProvider i18n={testI18n}><TemplateRunCenter port={{
      snapshot: { loading: false, loadError: null, runs: [run], activities: {} },
      assetReader: { get: async () => tombstone }, assetUrl: () => '/deleted-original',
      resume: async () => undefined, cancel: async () => undefined,
      review: async () => undefined, retry: async () => undefined,
    }} /></I18nextProvider>);
    await waitFor(() => expect(view.container.querySelector('[data-asset-media-state="deleted"]') !== null).toBe(true));
    expect(view.container.querySelector('img')).toBe(null);
    expect(view.container.querySelector('a[href]')).toBe(null);
    expect(view.container.textContent?.includes('素材已删除')).toBe(true);
  });
});
