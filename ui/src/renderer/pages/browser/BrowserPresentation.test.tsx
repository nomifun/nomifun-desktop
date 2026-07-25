/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import React from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { renderToStaticMarkup } from 'react-dom/server';
import type { IBrowserLane } from '@/common/browser/browserTypes';
import enBrowser from '../../services/i18n/locales/en-US/browser.json';
import BrowserInventoryTree from './BrowserInventoryTree';
import BrowserLaneDetails from './BrowserLaneDetails';
import BrowserPageHeader from './BrowserPageHeader';
import type { BrowserConversationGroup } from './browserInventoryModel';

const browserPageSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const inventoryTreeSource = readFileSync(
  new URL('./BrowserInventoryTree.tsx', import.meta.url),
  'utf8'
);

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': {
      translation: {
        browser: enBrowser,
      },
    },
  },
  interpolation: { escapeValue: false },
});

const renderBrowser = (content: React.ReactElement): string =>
  renderToStaticMarkup(<I18nextProvider i18n={testI18n}>{content}</I18nextProvider>);

const lane = (overrides: Partial<IBrowserLane> = {}): IBrowserLane => ({
  lane_id: 'lane-1',
  lane_name: 'Lane one',
  lifecycle_state: 'running',
  control_state: 'agent',
  tabs: [],
  ...overrides,
});

const group = (
  conversationId: string,
  label: string,
  lanes: IBrowserLane[]
): BrowserConversationGroup => ({
  conversationId,
  key: conversationId,
  label,
  owners: [
    {
      key: `runtime:${conversationId}`,
      label: `Runtime ${conversationId}`,
      lanes,
      lastActiveAt: 1,
    },
  ],
  lanes,
  runningCount: lanes.filter((item) => item.lifecycle_state === 'running').length,
  queuedCount: lanes.filter((item) => item.lifecycle_state === 'queued').length,
  lastActiveAt: 1,
});

describe('Browser management presentation', () => {
  test('keeps lane selection and conversation query scope synchronized', () => {
    expect(browserPageSource.includes('browserConversationSearchParamsForLane(searchParams, lane)')).toBe(
      true
    );
    expect(browserPageSource.includes("next.toString() !== searchParams.toString()")).toBe(true);
  });

  test('marks the requested conversation and keeps lane/conversation close controls available', () => {
    const currentLane = lane({
      lane_id: 'current-lane',
      lane_name: 'Current lane',
      lifecycle_state: 'queued',
      queue: { position: 2 },
    });
    const html = renderBrowser(
      <BrowserInventoryTree
        groups={[
          group('conversation-current', 'Current conversation', [currentLane]),
          group('conversation-old', 'Older conversation', [lane({ lane_id: 'old-lane' })]),
        ]}
        selectedLaneId='current-lane'
        currentConversationId='conversation-current'
        onSelectLane={() => undefined}
        onCloseLane={() => undefined}
        onCloseConversation={() => undefined}
      />
    );

    expect(html.indexOf('Current conversation')).toBeLessThan(html.indexOf('Older conversation'));
    expect(html.includes('>Current<')).toBe(true);
    expect(html.includes('Close all browser lanes for Current conversation')).toBe(true);
    expect(html.includes('Close Current lane')).toBe(true);
    expect(html.includes('queue #2')).toBe(true);
  });

  test('renders each lane tab as a nested child with title, URL, active, and crashed state', () => {
    const firstLane = lane({
      lane_id: 'lane-a',
      lane_name: 'Lane A',
      active_tab_id: 'lane-a-active',
      tabs: [
        {
          tab_id: 'lane-a-active',
          title: 'Lane A active tab',
          url: 'https://lane-a.example/active',
        },
        {
          tab_id: 'lane-a-crashed',
          title: 'Lane A crashed tab',
          url: 'https://lane-a.example/crashed',
          crashed: true,
        },
      ],
    });
    const secondLane = lane({
      lane_id: 'lane-b',
      lane_name: 'Lane B',
      active_tab_id: 'lane-b-active',
      tabs: [
        {
          tab_id: 'lane-b-active',
          title: 'Lane B active tab',
          url: 'https://lane-b.example/active',
          active: true,
        },
      ],
    });
    const html = renderBrowser(
      <BrowserInventoryTree
        groups={[group('conversation-tabs', 'Tabs conversation', [firstLane, secondLane])]}
        selectedLaneId='lane-a'
        currentConversationId='conversation-tabs'
        onSelectLane={() => undefined}
        onCloseLane={() => undefined}
        onCloseConversation={() => undefined}
      />
    );

    const laneAStart = html.indexOf('data-browser-lane-id="lane-a"');
    const laneBStart = html.indexOf('data-browser-lane-id="lane-b"');
    const laneAHtml = html.slice(laneAStart, laneBStart);
    const laneBHtml = html.slice(laneBStart);

    expect(laneAStart).toBeGreaterThan(-1);
    expect(laneBStart).toBeGreaterThan(laneAStart);
    expect(laneAHtml.includes('data-browser-lane-tabs="lane-a"')).toBe(true);
    expect(laneAHtml.includes('Lane A active tab')).toBe(true);
    expect(laneAHtml.includes('https://lane-a.example/active')).toBe(true);
    expect(
      laneAHtml.includes(
        'data-browser-tab-id="lane-a-active" data-browser-tab-active="true" data-browser-tab-crashed="false"'
      )
    ).toBe(true);
    expect(
      laneAHtml.includes(
        'data-browser-tab-id="lane-a-crashed" data-browser-tab-active="false" data-browser-tab-crashed="true"'
      )
    ).toBe(true);
    expect(laneAHtml.includes('>Current<')).toBe(true);
    expect(laneAHtml.includes('>crashed<')).toBe(true);
    expect(laneAHtml.includes('Lane B active tab')).toBe(false);
    expect(laneBHtml.includes('data-browser-lane-tabs="lane-b"')).toBe(true);
    expect(laneBHtml.includes('Lane B active tab')).toBe(true);
    expect(laneBHtml.includes('Lane A active tab')).toBe(false);
  });

  test('keeps tab rows presentational so lane selection remains scoped to the parent lane', () => {
    const tabMapStart = inventoryTreeSource.indexOf('{lane.tabs.map((tab) => {');
    const tabMapEnd = inventoryTreeSource.indexOf(
      '                          })}',
      tabMapStart
    );
    const tabMarkup = inventoryTreeSource.slice(tabMapStart, tabMapEnd);

    expect(tabMapStart).toBeGreaterThan(-1);
    expect(tabMapEnd).toBeGreaterThan(tabMapStart);
    expect(tabMarkup.includes("role='listitem'")).toBe(true);
    expect(tabMarkup.includes('onClick')).toBe(false);
    expect(tabMarkup.includes('onSelectLane')).toBe(false);
  });

  test('renders queued pressure metadata while preserving the lane close action', () => {
    const html = renderBrowser(
      <BrowserLaneDetails
        lane={lane({
          lane_id: 'queued-lane',
          lane_name: 'Queued lane',
          lifecycle_state: 'queued',
          queue: {
            position: 7,
            reason_code: 'system_memory_pressure',
            recommended_concurrency: 2,
          },
        })}
        closing={false}
        onClose={() => undefined}
        onInventoryRefresh={async () => undefined}
      />
    );

    expect(html.includes('Waiting for browser capacity')).toBe(true);
    expect(html.includes('queue position 7')).toBe(true);
    expect(html.includes('system_memory_pressure')).toBe(true);
    expect(html.includes('recommended concurrency 2')).toBe(true);
    expect(html.includes('>Close lane<')).toBe(true);
    expect(html.includes('viewer will become available')).toBe(true);
  });

  test('renders critical pressure without disabling close-all when lanes still exist', () => {
    const html = renderBrowser(
      <BrowserPageHeader
        runningCount={3}
        queuedCount={5}
        pressureState='critical'
        refreshing={false}
        closingAll={false}
        hasLanes
        onRefresh={() => undefined}
        onCloseAll={() => undefined}
      />
    );

    expect(html.includes('Critical pressure')).toBe(true);
    expect(html.includes('3 running')).toBe(true);
    expect(html.includes('5 queued')).toBe(true);
    expect(html.includes('>Close all<')).toBe(true);
    expect(html.includes('disabled')).toBe(false);
  });

  test('keeps lane management visible when the embedded stream has failed', () => {
    const html = renderBrowser(
      <BrowserLaneDetails
        lane={lane({
          lane_id: 'failed-viewer-lane',
          lane_name: 'Recoverable lane',
          viewer_state: 'failed',
          error_code: 'viewer_stream_failed',
          error_message: 'The embedded viewer disconnected.',
        })}
        closing={false}
        onClose={() => undefined}
        onInventoryRefresh={async () => undefined}
      />
    );

    expect(html.includes('viewer_stream_failed: The embedded viewer disconnected.')).toBe(true);
    expect(html.includes('>Close lane<')).toBe(true);
  });
});
