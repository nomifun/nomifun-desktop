/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import { cloneWorkflowRunAggregate } from '../domain';
import { IDS, createWorkflowRunFixture } from '../domain/testFixtures';
import WorkflowRunCenter from './WorkflowRunCenter';

const testI18n = createInstance();
testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: {} } },
  interpolation: { escapeValue: false },
});

describe('Workflow Run Center', () => {
  test('renders durable status, progress, recovery actions, and real result URLs', () => {
    const succeeded = createWorkflowRunFixture();
    succeeded.revision = 4;
    succeeded.record.status = 'succeeded';
    succeeded.record.taskIds = [IDS.task];
    succeeded.record.resultAssetIds = [IDS.asset];
    succeeded.record.queuedAt = 2_100;
    succeeded.record.startedAt = 2_200;
    succeeded.record.completedAt = 2_300;
    const paused = cloneWorkflowRunAggregate(succeeded);
    paused.request.id = IDS.idempotency;
    paused.request.idempotencyKey = IDS.idempotency;
    paused.record.requestId = IDS.idempotency;
    paused.revision = 3;
    paused.record.status = 'running';
    paused.record.resultAssetIds = [];
    paused.record.completedAt = null;

    const html = renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <WorkflowRunCenter
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

    expect(html.includes('data-workflow-run-center="true"')).toBe(true);
    expect(html.includes('Template runs')).toBe(true);
    expect(html.includes('Waiting to resume')).toBe(true);
    expect(html.includes('Resume')).toBe(true);
    expect(html.includes('Completed')).toBe(true);
    expect(html.includes(`/api/creative-studio/files/${IDS.asset}`)).toBe(true);
    expect(html.includes('refresh or restart')).toBe(true);
  });
});
