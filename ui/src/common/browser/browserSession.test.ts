/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  browserSession,
  normalizeBrowserLane,
  normalizeBrowserOverview,
} from './browserSession';
import { resolveBrowserOverviewCapabilities } from './browserTypes';

const realFetch = globalThis.fetch;

describe('browserSession foreground request', () => {
  test('posts a URL-encoded lane id without exposing a page-control body', async () => {
    let request: { url: string; method?: string; body?: BodyInit | null } | undefined;
    try {
      globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
        request = { url: String(input), method: init?.method, body: init?.body };
        return Promise.resolve(
          new Response(
            JSON.stringify({ success: true, data: { foregrounded: true } }),
            { status: 200, headers: { 'Content-Type': 'application/json' } }
          )
        );
      }) as typeof fetch;

      const result = await browserSession.foregroundLane.invoke({
        lane_id: 'primary/lane 1',
      });
      expect(result).toEqual({ foregrounded: true });
      expect(request?.url.endsWith('/api/browser/lanes/primary%2Flane%201/foreground')).toBe(
        true
      );
      expect(request?.method).toBe('POST');
      expect(request?.body).toBeUndefined();
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('posts the symmetric background route and requires backend confirmation', async () => {
    let request: { url: string; method?: string; body?: BodyInit | null } | undefined;
    try {
      globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
        request = { url: String(input), method: init?.method, body: init?.body };
        return Promise.resolve(
          new Response(
            JSON.stringify({
              success: true,
              data: { backgrounded: true, lane_id: 'primary/lane 1' },
            }),
            { status: 200, headers: { 'Content-Type': 'application/json' } }
          )
        );
      }) as typeof fetch;

      const result = await browserSession.backgroundLane.invoke({
        lane_id: 'primary/lane 1',
      });
      expect(result).toEqual({
        backgrounded: true,
        lane_id: 'primary/lane 1',
      });
      expect(request?.url.endsWith('/api/browser/lanes/primary%2Flane%201/background')).toBe(
        true
      );
      expect(request?.method).toBe('POST');
      expect(request?.body).toBeUndefined();
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});

describe('browserSession display-mode policy', () => {
  test('loads and updates the owner policy through the typed live API', async () => {
    const requests: Array<{ url: string; method?: string; body?: BodyInit | null }> = [];
    try {
      globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
        requests.push({ url: String(input), method: init?.method, body: init?.body });
        const displayMode = init?.method === 'PUT' ? 'external' : 'headless';
        return Promise.resolve(
          new Response(
            JSON.stringify({ success: true, data: { display_mode: displayMode } }),
            { status: 200, headers: { 'Content-Type': 'application/json' } }
          )
        );
      }) as typeof fetch;

      expect(await browserSession.displayMode.get.invoke()).toEqual({
        display_mode: 'headless',
      });
      expect(
        await browserSession.displayMode.put.invoke({ display_mode: 'external' })
      ).toEqual({ display_mode: 'external' });

      expect(requests[0]?.url.endsWith('/api/browser/display-mode')).toBe(true);
      expect(requests[0]?.method).toBe('GET');
      expect(requests[1]?.method).toBe('PUT');
      expect(JSON.parse(String(requests[1]?.body))).toEqual({
        display_mode: 'external',
      });
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('rejects malformed policy responses instead of claiming a fallback succeeded', async () => {
    try {
      globalThis.fetch = (() =>
        Promise.resolve(
          new Response(
            JSON.stringify({ success: true, data: { display_mode: 'embedded' } }),
            { status: 200, headers: { 'Content-Type': 'application/json' } }
          )
        )) as typeof fetch;

      let caught: unknown;
      try {
        await browserSession.displayMode.get.invoke();
      } catch (error) {
        caught = error;
      }
      expect(caught instanceof Error).toBe(true);
      expect((caught as Error).message.includes('invalid display mode')).toBe(true);
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});

describe('normalizeBrowserLane tab projection', () => {
  test('keeps public tab handles without retaining or falling back to raw CDP target ids', () => {
    const lane = normalizeBrowserLane({
      lane_id: 'lane-safe',
      lifecycle_state: 'running',
      browser_epoch: 12,
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
    expect(lane?.browser_epoch).toBe(12);
  });
});

describe('normalizeBrowserLane lifecycle fallback', () => {
  test('maps a missing or empty lifecycle state to unknown instead of failed', () => {
    for (const raw of [
      { lane_id: 'lane-no-state' },
      { lane_id: 'lane-empty-state', lifecycle_state: '' },
      { lane_id: 'lane-bad-state', lifecycle_state: 42 },
    ]) {
      expect(normalizeBrowserLane(raw)?.lifecycle_state).toBe('unknown');
    }
    expect(
      normalizeBrowserLane({ lane_id: 'lane-real', lifecycle_state: 'failed' })
        ?.lifecycle_state
    ).toBe('failed');
  });
});

describe('normalizeBrowserLane owner attribution', () => {
  test('lets an explicit owner-object null retract legacy lane-level attribution', () => {
    const lane = normalizeBrowserLane({
      lane_id: 'lane-owner-null',
      lifecycle_state: 'running',
      user_id: 'legacy-user',
      conversation_id: 'legacy-conversation',
      owner: {
        user_id: null,
        conversation_id: null,
        label: 'system maintenance',
      },
    });

    expect(lane?.owner?.user_id).toBeNull();
    expect(lane?.owner?.conversation_id).toBeNull();
    expect(lane?.owner?.label).toBe('system maintenance');
  });

  test('still falls back to lane-level attribution when the owner object omits the key', () => {
    const lane = normalizeBrowserLane({
      lane_id: 'lane-owner-fallback',
      lifecycle_state: 'running',
      user_id: 'lane-user',
      runtime_id: 'lane-runtime',
      owner: { label: 'labelled owner' },
    });

    expect(lane?.owner?.user_id).toBe('lane-user');
    expect(lane?.owner?.runtime_instance_id).toBe('lane-runtime');
    expect(lane?.owner?.label).toBe('labelled owner');
  });

  test('honors the runtime_id alias inside the owner object including explicit null', () => {
    const aliased = normalizeBrowserLane({
      lane_id: 'lane-owner-alias',
      lifecycle_state: 'running',
      runtime_instance_id: 'lane-runtime',
      owner: { runtime_id: null, label: 'maintenance' },
    });
    expect(aliased?.owner?.runtime_instance_id).toBeNull();
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
          headful: false,
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
        headful: false,
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
          is_headful: true,
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
        headful: true,
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
