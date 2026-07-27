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
import type { IBrowserLane, IBrowserOverview } from '@/common/browser/browserTypes';
import enBrowser from '../../services/i18n/locales/en-US/browser.json';
import BrowserHostDiagnostics from './BrowserHostDiagnostics';
import BrowserInventoryTree from './BrowserInventoryTree';
import BrowserLaneDetails from './BrowserLaneDetails';
import BrowserPageHeader from './BrowserPageHeader';
import type { BrowserConversationGroup } from './browserInventoryModel';

const browserPageSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const inventoryTreeSource = readFileSync(
  new URL('./BrowserInventoryTree.tsx', import.meta.url),
  'utf8'
);
const laneDetailsSource = readFileSync(
  new URL('./BrowserLaneDetails.tsx', import.meta.url),
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

  test('treats active_tab_id as authoritative over stale tab flags and lane URL', () => {
    const html = renderBrowser(
      <BrowserInventoryTree
        groups={[
          group('conversation-tabs', 'Tabs conversation', [
            lane({
              lane_id: 'lane-authoritative-tab',
              lane_name: 'Authoritative tab lane',
              url: 'https://stale-lane.example/old',
              active_tab_id: 'fresh-tab',
              tabs: [
                {
                  tab_id: 'stale-tab',
                  title: 'Stale active flag',
                  url: 'https://stale-tab.example/old',
                  active: true,
                },
                {
                  tab_id: 'fresh-tab',
                  title: 'Fresh authoritative tab',
                  url: 'https://fresh-tab.example/current',
                  active: false,
                },
              ],
            }),
          ]),
        ]}
        selectedLaneId='lane-authoritative-tab'
        onSelectLane={() => undefined}
        onCloseLane={() => undefined}
        onCloseConversation={() => undefined}
      />
    );

    const tabListStart = html.indexOf('data-browser-lane-tabs="lane-authoritative-tab"');
    const laneHeader = html.slice(0, tabListStart);
    expect(tabListStart).toBeGreaterThan(-1);
    expect(laneHeader.includes('fresh-tab.example')).toBe(true);
    expect(laneHeader.includes('stale-lane.example')).toBe(false);
    expect(
      html.includes(
        'data-browser-tab-id="stale-tab" data-browser-tab-active="false" data-browser-tab-crashed="false"'
      )
    ).toBe(true);
    expect(
      html.includes(
        'data-browser-tab-id="fresh-tab" data-browser-tab-active="true" data-browser-tab-crashed="false"'
      )
    ).toBe(true);
    expect(html.match(/>Current</g)).toHaveLength(1);
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
      />
    );

    expect(html.includes('Waiting for browser capacity')).toBe(true);
    expect(html.includes('queue position 7')).toBe(true);
    expect(html.includes('system_memory_pressure')).toBe(true);
    expect(html.includes('recommended concurrency 2')).toBe(true);
    expect(html.includes('>Close lane<')).toBe(true);
    expect(html.includes('Status only')).toBe(true);
    expect(html.includes('external managed window')).toBe(true);
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
        canCloseAll
        closeAllLabel='Close all globally'
        onRefresh={() => undefined}
        onCloseAll={() => undefined}
      />
    );

    expect(html.includes('Critical pressure')).toBe(true);
    expect(html.includes('3 running')).toBe(true);
    expect(html.includes('5 queued')).toBe(true);
    expect(html.includes('>Close all globally<')).toBe(true);
    expect(html.includes('disabled')).toBe(false);
  });

  test('hides installation-wide close-all unless overview grants it explicitly', () => {
    const renderHeader = (canCloseAll: boolean) =>
      renderBrowser(
        <BrowserPageHeader
          runningCount={1}
          queuedCount={0}
          refreshing={false}
          closingAll={false}
          hasLanes
          canCloseAll={canCloseAll}
          closeAllLabel='Owner-only close all'
          onRefresh={() => undefined}
          onCloseAll={() => undefined}
        />
      );

    expect(renderHeader(true).includes('Owner-only close all')).toBe(true);
    const deniedHtml = renderHeader(false);
    expect(deniedHtml.includes('Owner-only close all')).toBe(false);
    expect(deniedHtml.includes('Refresh')).toBe(true);
    expect(deniedHtml.includes('1 running')).toBe(true);
  });

  test('wires Browser page close-all visibility to normalized overview capability', () => {
    expect(browserPageSource.includes('resolveBrowserOverviewCapabilities(overview)')).toBe(true);
    expect(browserPageSource.includes('canCloseAll={canCloseAll}')).toBe(true);
  });

  test('does not read presentation settings or expose a viewer seam from the Browser page', () => {
    expect(browserPageSource.includes("from '@/common/browser/browserSettings'")).toBe(false);
    expect(browserPageSource.includes("useConfig('agent.browserUse.displayMode')")).toBe(false);
    expect(browserPageSource.includes('displayMode=')).toBe(false);
    expect(browserPageSource.includes('onInventoryRefresh=')).toBe(false);
  });

  test('keeps lane management visible when the lane reports an error', () => {
    const html = renderBrowser(
      <BrowserLaneDetails
        lane={lane({
          lane_id: 'failed-managed-lane',
          lane_name: 'Recoverable lane',
          error_code: 'managed_window_disconnected',
          error_message: 'The external managed browser disconnected.',
        })}
        closing={false}
        onClose={() => undefined}
      />
    );

    expect(
      html.includes(
        'managed_window_disconnected: The external managed browser disconnected.'
      )
    ).toBe(true);
    expect(html.includes('>Close lane<')).toBe(true);
  });

  test('renders a complete status-only lane surface without importing viewer code', () => {
    const html = renderBrowser(
      <BrowserLaneDetails
        lane={lane({
          lane_id: 'status-only-lane',
          lane_name: 'External managed lane',
          title: 'Fallback lane title',
          url: 'https://stale.example/old',
          active_tab_id: 'active-tab',
          tabs: [
            {
              tab_id: 'active-tab',
              title: 'Managed browser page',
              url: 'https://managed.example/current',
            },
            {
              tab_id: 'background-tab',
              title: 'Background page',
              url: 'https://managed.example/background',
            },
          ],
          identity: {
            mode: 'authenticated_replica',
            label: 'Signed-in replica',
            generation: 4,
          },
          owner: {
            agent_name: 'Research Agent',
            runtime_instance_id: 'runtime-status',
            execution_id: 'execution-status',
            attempt_id: 'attempt-status',
            cluster_node_id: 'node-status',
          },
          queue: {
            owner_active: 1,
            owner_queued: 2,
            global_active: 3,
            global_queued: 4,
          },
          active_operation_count: 2,
          resource_estimate_bytes: 64 * 1024 * 1024,
        })}
        closing={false}
        onClose={() => undefined}
      />
    );

    expect(html.includes('data-browser-lane-status-only="true"')).toBe(true);
    expect(html.includes('Managed browser page')).toBe(true);
    expect(html.includes('https://managed.example/current')).toBe(true);
    expect(html.includes('>2<')).toBe(true);
    expect(html.includes('Signed-in replica')).toBe(true);
    expect(html.includes('Research Agent')).toBe(true);
    expect(html.includes('64 MiB')).toBe(true);
    expect(html.includes('1 active · 2 queued')).toBe(true);
    expect(html.includes('3 active · 4 queued')).toBe(true);
    expect(html.includes('Take control')).toBe(false);
    expect(html.includes('Return to Agent')).toBe(false);

    expect(laneDetailsSource.includes('EmbeddedBrowserViewer')).toBe(false);
    expect(laneDetailsSource.includes('viewerToken')).toBe(false);
    expect(laneDetailsSource.includes('WebSocket')).toBe(false);
    expect(laneDetailsSource.includes('onInventoryRefresh')).toBe(false);
    expect(browserPageSource.includes('EmbeddedBrowserViewer')).toBe(false);
    expect(browserPageSource.includes('displayMode')).toBe(false);
    expect(browserPageSource.includes('viewerToken')).toBe(false);
  });

  test('renders Host diagnostics collapsed with safe resource metadata', () => {
    const overview: IBrowserOverview = {
      supported: true,
      enabled: true,
      running_lanes: 2,
      queued_lanes: 1,
      total_lanes: 3,
      pressure_state: 'pressured',
      capacity: {
        active: 2,
        queued: 1,
        max_active: 8,
        max_open_lanes: 32,
        recommended_concurrency: 4,
        reason_code: 'browser_memory_pressure',
      },
      hosts: [
        {
          host_id: 'host-primary',
          state: 'running',
          epoch: 7,
          identity_mode: 'primary',
          lane_count: 2,
          rss_bytes: 64 * 1024 * 1024,
        },
      ],
      updated_at: 1_700_000_000_000,
    };

    const html = renderBrowser(<BrowserHostDiagnostics overview={overview} />);

    expect(html.includes('<details')).toBe(true);
    expect(html.includes(' open')).toBe(false);
    expect(html.includes('Host diagnostics')).toBe(true);
    expect(html.includes('host-primary')).toBe(true);
    expect(html.includes('64 MiB')).toBe(true);
    expect(html.includes('Primary')).toBe(true);
    expect(html.includes('browser_memory_pressure')).toBe(true);
    expect(html.includes('cdp')).toBe(false);
    expect(html.includes('profile')).toBe(false);
    expect(html.includes('debugging')).toBe(false);
  });
});
