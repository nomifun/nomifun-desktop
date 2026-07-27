/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  normalizeBrowserLane,
  normalizeBrowserOverview,
} from './browserSession';
import { resolveBrowserOverviewCapabilities } from './browserTypes';

describe('normalizeBrowserLane tab projection', () => {
  test('keeps public tab handles without retaining or falling back to raw CDP target ids', () => {
    const lane = normalizeBrowserLane({
      lane_id: 'lane-safe',
      lifecycle_state: 'running',
      tabs: [
        {
          tab_id: 'tab-safe',
          target_id: 'raw-cdp-target-secret',
          title: 'Safe tab',
          url: 'https://example.test/',
          active: true,
          crashed: false,
        },
        {
          target_id: 'raw-cdp-target-only',
          title: 'Must not become a renderer tab',
        },
      ],
    });

    expect(lane?.tabs).toEqual([
      {
        tab_id: 'tab-safe',
        title: 'Safe tab',
        url: 'https://example.test/',
        active: true,
        crashed: false,
      },
    ]);
    expect(lane?.tabs[0] && 'target_id' in lane.tabs[0]).toBe(false);
  });
});

describe('normalizeBrowserOverview Host projection', () => {
  test('keeps safe Host diagnostics and ignores sensitive backend fields', () => {
    const overview = normalizeBrowserOverview({
      running_lanes: 2,
      queued_lanes: 1,
      hosts: [
        {
          host_id: 'host-primary',
          state: 'running',
          epoch: 4,
          identity_mode: 'primary',
          lane_count: 2,
          rss_bytes: 32 * 1024 * 1024,
          process_id: 1234,
          debugging_port: 9222,
          cdp_endpoint: 'http://127.0.0.1:9222',
          profile_path: 'C:\\private\\profile',
        },
      ],
    });

    expect(overview.hosts).toEqual([
      {
        host_id: 'host-primary',
        state: 'running',
        epoch: 4,
        identity_mode: 'primary',
        lane_count: 2,
        rss_bytes: 32 * 1024 * 1024,
      },
    ]);
    expect(overview.hosts?.[0] && 'process_id' in overview.hosts[0]).toBe(false);
    expect(overview.hosts?.[0] && 'debugging_port' in overview.hosts[0]).toBe(false);
    expect(overview.hosts?.[0] && 'cdp_endpoint' in overview.hosts[0]).toBe(false);
    expect(overview.hosts?.[0] && 'profile_path' in overview.hosts[0]).toBe(false);
  });

  test('accepts legacy aliases while rejecting malformed Hosts', () => {
    const overview = normalizeBrowserOverview({
      counts: { running: 1, queued: 0 },
      hosts: [
        {
          id: 'host-alias',
          lifecycle_state: 'restarting',
          browser_epoch: 9,
          mode: 'anonymous',
          lanes: 1,
          memory_rss_bytes: 4096,
        },
        { state: 'running' },
        null,
        'not-a-host',
      ],
    });

    expect(overview.hosts).toEqual([
      {
        host_id: 'host-alias',
        state: 'restarting',
        epoch: 9,
        identity_mode: 'anonymous',
        lane_count: 1,
        rss_bytes: 4096,
      },
    ]);
  });
});

describe('normalizeBrowserOverview capability projection', () => {
  test('projects explicit owner capabilities', () => {
    const overview = normalizeBrowserOverview({
      running_lanes: 0,
      queued_lanes: 0,
      can_close_all: true,
      can_manage_browser_settings: true,
      can_manage_primary_identity: true,
    });

    expect(overview.can_close_all).toBe(true);
    expect(overview.can_manage_browser_settings).toBe(true);
    expect(overview.can_manage_primary_identity).toBe(true);
    expect(resolveBrowserOverviewCapabilities(overview)).toEqual({
      canCloseAll: true,
      canManageBrowserSettings: true,
      canManagePrimaryIdentity: true,
    });
  });

  test('fails closed when capability fields are missing, false, or malformed', () => {
    const missing = normalizeBrowserOverview({
      running_lanes: 0,
      queued_lanes: 0,
    });
    const denied = normalizeBrowserOverview({
      running_lanes: 0,
      queued_lanes: 0,
      can_close_all: false,
      can_manage_browser_settings: false,
      can_manage_primary_identity: false,
    });
    const malformed = normalizeBrowserOverview({
      running_lanes: 0,
      queued_lanes: 0,
      can_close_all: 'true',
      can_manage_browser_settings: 1,
      can_manage_primary_identity: {},
    });

    for (const overview of [missing, denied, malformed]) {
      expect(resolveBrowserOverviewCapabilities(overview)).toEqual({
        canCloseAll: false,
        canManageBrowserSettings: false,
        canManagePrimaryIdentity: false,
      });
    }
  });
});
