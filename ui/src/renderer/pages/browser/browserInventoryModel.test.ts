import { describe, expect, test } from 'bun:test';
import type { IBrowserHost, IBrowserLane } from '@/common/browser/browserTypes';
import {
  browserConversationSearchParamsForLane,
  browserLaneCounts,
  buildBrowserInventoryTree,
  matchBrowserLaneHost,
  pickDefaultBrowserLaneId,
  resolveBrowserConversationId,
  type BrowserInventoryLabels,
} from './browserInventoryModel';

const lane = (overrides: Partial<IBrowserLane>): IBrowserLane => ({
  lane_id: 'lane-default',
  lifecycle_state: 'running',
  tabs: [],
  ...overrides,
});

const labels: BrowserInventoryLabels = {
  clusterNode: (id) => `节点 ${id}`,
  attempt: (id) => `尝试 ${id}`,
  runtime: (id) => `运行时 ${id}`,
  execution: (id) => `执行 ${id}`,
  owner: '所有者',
  laneOwner: '通道所有者',
  conversation: (id) => `对话 ${id}`,
  unassigned: '未分配',
};

describe('browser inventory model', () => {
  test('prioritizes the requested conversation and groups its runtime owners', () => {
    const groups = buildBrowserInventoryTree(
      [
        lane({ lane_id: 'older', conversation_id: 'conversation-old', last_active_at: 20 }),
        lane({
          lane_id: 'current-a',
          conversation_id: 'conversation-current',
          runtime_instance_id: 'runtime-a',
          last_active_at: 10,
        }),
        lane({
          lane_id: 'current-b',
          conversation_id: 'conversation-current',
          runtime_instance_id: 'runtime-a',
          last_active_at: 5,
        }),
      ],
      { 'conversation-current': 'Current work' },
      labels,
      'conversation-current'
    );

    expect(groups[0]?.label).toBe('Current work');
    expect(groups[0]?.owners).toHaveLength(1);
    expect(groups[0]?.owners[0]?.label).toBe('运行时 runtime-a');
    expect(groups[0]?.owners[0]?.lanes.map((item) => item.lane_id)).toEqual([
      'current-a',
      'current-b',
    ]);
    expect(pickDefaultBrowserLaneId(groups, 'conversation-current')).toBe('current-a');
  });

  test('uses caller-provided localized fallback labels', () => {
    const groups = buildBrowserInventoryTree(
      [
        lane({
          lane_id: 'unassigned',
          cluster_node_id: 'cluster-1234567890-long',
        }),
      ],
      {},
      labels
    );

    expect(groups[0]?.label).toBe('未分配');
    expect(groups[0]?.owners[0]?.label).toBe('节点 cluster-…long');
  });

  test('counts running capacity separately from queued lanes', () => {
    expect(
      browserLaneCounts([
        lane({ lane_id: 'one', lifecycle_state: 'running' }),
        lane({ lane_id: 'two', lifecycle_state: 'frozen' }),
        lane({ lane_id: 'three', lifecycle_state: 'queued' }),
        lane({ lane_id: 'four', lifecycle_state: 'failed' }),
      ])
    ).toEqual({ running: 2, queued: 1 });
  });

  test('prioritizes the current conversation before newer activity elsewhere', () => {
    const groups = buildBrowserInventoryTree(
      [
        lane({
          lane_id: 'newer-other',
          conversation_id: 'conversation-other',
          last_active_at: 500,
        }),
        lane({
          lane_id: 'current-older',
          conversation_id: 'conversation-current',
          last_active_at: 10,
        }),
        lane({
          lane_id: 'current-newer',
          conversation_id: 'conversation-current',
          last_active_at: 20,
        }),
      ],
      {
        'conversation-current': 'Current conversation',
        'conversation-other': 'Other conversation',
      },
      labels,
      'conversation-current'
    );

    expect(groups.map((group) => group.conversationId)).toEqual([
      'conversation-current',
      'conversation-other',
    ]);
    expect(groups[0]?.owners[0]?.lanes.map((item) => item.lane_id)).toEqual([
      'current-newer',
      'current-older',
    ]);
    expect(pickDefaultBrowserLaneId(groups, 'conversation-current')).toBe('current-newer');
  });

  test('uses the first available lane when the requested conversation has no inventory', () => {
    const groups = buildBrowserInventoryTree(
      [
        lane({
          lane_id: 'available',
          conversation_id: 'conversation-available',
          last_active_at: 1,
        }),
      ],
      {},
      labels,
      'conversation-missing'
    );

    expect(pickDefaultBrowserLaneId(groups, 'conversation-missing')).toBe('available');
    expect(pickDefaultBrowserLaneId([], 'conversation-missing')).toBeNull();
  });

  test('updates lane selection scope and clears a stale conversation for unassigned lanes', () => {
    expect(
      browserConversationSearchParamsForLane(
        new URLSearchParams('conversation_id=old&keep=yes'),
        lane({
          lane_id: 'owner-conversation',
          conversation_id: null,
          owner: { conversation_id: 'conversation-from-owner' },
        })
      ).toString()
    ).toBe('conversation_id=conversation-from-owner&keep=yes');

    const unassigned = browserConversationSearchParamsForLane(
      new URLSearchParams('conversation_id=old&keep=yes'),
      lane({
        lane_id: 'unassigned',
        conversation_id: null,
        owner: null,
      })
    );
    expect(unassigned.get('conversation_id')).toBeNull();
    expect(unassigned.get('keep')).toBe('yes');
  });

  test('uses explicit Browser query scope before router state', () => {
    expect(
      resolveBrowserConversationId({
        requestedConversationId: 'conversation-query',
        pathname: '/browser',
        locationState: { conversation_id: 'conversation-state' },
      })
    ).toBe('conversation-query');
  });

  test('reuses reliable router state when Browser is opened or refreshed without a query', () => {
    expect(
      resolveBrowserConversationId({
        pathname: '/browser',
        locationState: { conversation_id: 'conversation-state' },
      })
    ).toBe('conversation-state');
    expect(
      resolveBrowserConversationId({
        pathname: '/browser',
        locationState: { conversation: { id: 'conversation-nested' } },
      })
    ).toBe('conversation-nested');
  });

  test('accepts a canonical conversation route and otherwise leaves activity fallback intact', () => {
    const conversationId = '0190f5fe-7c00-7a00-8000-000000000011';
    expect(
      resolveBrowserConversationId({
        pathname: `/conversation/${conversationId}`,
      })
    ).toBe(conversationId);
    expect(resolveBrowserConversationId({ pathname: '/browser' })).toBeNull();
  });
});

describe('matchBrowserLaneHost', () => {
  const host = (overrides: Partial<IBrowserHost>): IBrowserHost => ({
    host_id: 'host-default',
    state: 'running',
    identity_mode: 'primary',
    ...overrides,
  });
  const primaryLane = (overrides: Partial<IBrowserLane>): IBrowserLane =>
    lane({ identity: { mode: 'primary' }, browser_epoch: 4, ...overrides });

  test('matches the serving host by browser epoch', () => {
    // Lane payloads carry no host_id (BrowserLaneDto serializes only
    // browser_epoch), so the epoch is the sole direct host linkage.
    const hosts = [
      host({ host_id: 'host-a', epoch: 3 }),
      host({ host_id: 'host-b', epoch: 4 }),
    ];
    expect(matchBrowserLaneHost(primaryLane({}), hosts)?.host_id).toBe('host-b');
  });

  test('tolerates snapshot skew by matching the sole primary host', () => {
    // After a display-mode restart the overview may already list the
    // new-epoch host while the lanes snapshot still carries the old epoch.
    const hosts = [
      host({ host_id: 'host-replacement', epoch: 9, headful: true }),
      host({ host_id: 'host-anonymous', epoch: 9, identity_mode: 'anonymous' }),
    ];
    expect(matchBrowserLaneHost(primaryLane({ browser_epoch: 8 }), hosts)?.host_id).toBe(
      'host-replacement'
    );

    // Ambiguity (two primary hosts mid-transition) must not guess.
    expect(
      matchBrowserLaneHost(primaryLane({ browser_epoch: 8 }), [
        host({ host_id: 'host-one', epoch: 9 }),
        host({ host_id: 'host-two', epoch: 10 }),
      ])
    ).toBeNull();
  });

  test('never matches non-primary lanes or empty inputs', () => {
    expect(
      matchBrowserLaneHost(lane({ identity: { mode: 'anonymous' }, browser_epoch: 4 }), [
        host({ epoch: 4 }),
      ])
    ).toBeNull();
    expect(matchBrowserLaneHost(null, [host({})])).toBeNull();
    expect(matchBrowserLaneHost(primaryLane({}), undefined)).toBeNull();
  });
});
