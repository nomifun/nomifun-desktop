/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import type { IBrowserLane } from '@/common/browser/browserTypes';
import type { BrowserConversationGroup } from './browserInventoryModel';
import {
  browserInstallationWideCloseCopy,
  browserClosePartialFailureMessage,
  browserCloseResultIsUnconfirmed,
  canForegroundBrowserLane,
  browserLaneHasActiveWork,
  createBrowserManagementMutationGate,
  requestBrowserCloseAll,
  requestBrowserConversationClose,
  requestBrowserLaneClose,
  runBrowserCloseAll,
  runBrowserConversationClose,
  runBrowserLaneBackground,
  runBrowserLaneForeground,
  runBrowserLaneClose,
  type BrowserConfirmationRequest,
} from './browserManagementActions';

const lane = (overrides: Partial<IBrowserLane> = {}): IBrowserLane => ({
  lane_id: 'lane-1',
  lifecycle_state: 'running',
  tabs: [],
  ...overrides,
});

const conversationGroup = (
  overrides: Partial<BrowserConversationGroup> = {}
): BrowserConversationGroup => ({
  conversationId: 'conversation-1',
  key: 'conversation-1',
  label: 'Conversation one',
  owners: [],
  lanes: [lane()],
  runningCount: 1,
  queuedCount: 0,
  lastActiveAt: 1,
  ...overrides,
});

const confirmationCopy = {
  title: 'Confirm close',
  content: 'This keeps the conversation alive.',
  okText: 'Close',
  cancelText: 'Keep open',
};

const browserPageSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');

const deferred = <T>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
};

describe('browser management actions', () => {
  test('atomically gates delayed confirmation callbacks and releases in finally', async () => {
    const gate = createBrowserManagementMutationGate();
    const first = deferred<void>();
    const calls: string[] = [];
    const running = gate.run(async () => {
      calls.push('first:start');
      await first.promise;
      calls.push('first:end');
    });
    expect(gate.isBusy()).toBe(true);

    expect(
      await gate.run(
        async () => {
          calls.push('second:unexpected');
        },
        () => calls.push('second:busy')
      )
    ).toBe(false);
    first.resolve(undefined);
    expect(await running).toBe(true);
    expect(gate.isBusy()).toBe(false);
    expect(calls).toEqual(['first:start', 'second:busy', 'first:end']);

    let caught = false;
    try {
      await gate.run(async () => {
        throw new Error('operation failed');
      });
    } catch {
      caught = true;
    }
    expect(caught).toBe(true);
    expect(gate.isBusy()).toBe(false);
  });

  test('describes installation-wide close-all as a fully verified resource drain', () => {
    const en = browserInstallationWideCloseCopy('en-US');
    expect(en.title.includes('across this installation')).toBe(true);
    expect(en.warning.includes('installation-wide global')).toBe(true);
    expect(en.warning.includes("every user's browser lanes")).toBe(true);
    expect(en.warning.includes('drains pending cleanup')).toBe(true);
    expect(en.warning.includes('managed Browser Hosts/processes')).toBe(true);
    expect(en.warning.includes('three authoritative remaining counts')).toBe(true);
    expect(en.warning.includes('all zero')).toBe(true);
    expect(en.button.includes('globally')).toBe(true);
    expect(en.success.includes('lanes, pending cleanup, and managed Hosts/processes')).toBe(
      true
    );

    const zh = browserInstallationWideCloseCopy('zh-CN');
    expect(zh.title.includes('整个安装')).toBe(true);
    expect(zh.warning.includes('全局')).toBe(true);
    expect(zh.warning.includes('所有用户的浏览器通道')).toBe(true);
    expect(zh.warning.includes('排空待清理任务')).toBe(true);
    expect(zh.warning.includes('受管浏览器主机及进程')).toBe(true);
    expect(zh.warning.includes('三项权威剩余计数全部为 0')).toBe(true);
    expect(zh.success.includes('浏览器通道、待清理任务和受管主机及进程全部归零')).toBe(
      true
    );
  });

  test('wires visible page controls to the tested management action layer', () => {
    expect(browserPageSource.includes('runBrowserLaneClose(lane, {')).toBe(true);
    expect(
      browserPageSource.includes(
        'requestBrowserLaneClose(lane, closeLaneExclusively, confirmDanger'
      )
    ).toBe(true);
    expect(
      browserPageSource.includes('closeConversationExclusively')
    ).toBe(true);
    expect(
      browserPageSource.includes('requestBrowserCloseAll(closeAllExclusively, confirmDanger')
    ).toBe(true);
    expect(browserPageSource.includes('onCloseLane={handleCloseLane}')).toBe(true);
    expect(browserPageSource.includes('onCloseConversation={handleCloseConversation}')).toBe(true);
    expect(browserPageSource.includes('onCloseAll={handleCloseAll}')).toBe(true);
    expect(browserPageSource.includes('runBrowserLaneForeground(lane, {')).toBe(true);
    expect(
      browserPageSource.includes('ipcBridge.browserSession.foregroundLane.invoke(request)')
    ).toBe(true);
    expect(browserPageSource.includes('onForeground={handleForegroundLane}')).toBe(true);
    expect(browserPageSource.includes('runBrowserLaneBackground(lane, {')).toBe(true);
    expect(
      browserPageSource.includes('ipcBridge.browserSession.backgroundLane.invoke(request)')
    ).toBe(true);
    expect(browserPageSource.includes('onBackground={handleBackgroundLane}')).toBe(true);
  });

  test('foregrounds only running Primary lanes and reports success', async () => {
    const primary = lane({ identity: { mode: 'primary' } });
    expect(canForegroundBrowserLane(primary)).toBe(true);
    for (const unavailable of [
      lane({ lifecycle_state: 'queued', identity: { mode: 'primary' } }),
      lane({ lifecycle_state: 'failed', identity: { mode: 'primary' } }),
      lane({ identity: { mode: 'anonymous' } }),
      lane(),
    ]) {
      expect(canForegroundBrowserLane(unavailable)).toBe(false);
    }

    const calls: string[] = [];
    await runBrowserLaneForeground(primary, {
      invoke: async ({ lane_id }) => {
        calls.push(`foreground:${lane_id}`);
        return { foregrounded: true };
      },
      refresh: async () => {
        calls.push('refresh');
      },
      setChangingVisibilityLaneId: (value) => calls.push(`busy:${value}`),
      notifySuccess: (message) => calls.push(`success:${message}`),
      notifyError: (message) => calls.push(`error:${message}`),
      successMessage: 'opened',
    });

    expect(calls).toEqual([
      'busy:lane-1',
      'foreground:lane-1',
      'refresh',
      'success:opened',
      'busy:null',
    ]);
  });

  test('does not invoke foreground for queued, failed, or non-Primary lanes', async () => {
    for (const unavailable of [
      lane({ lifecycle_state: 'queued', identity: { mode: 'primary' } }),
      lane({ lifecycle_state: 'failed', identity: { mode: 'primary' } }),
      lane({ identity: { mode: 'isolated' } }),
    ]) {
      let invoked = false;
      let busy = false;
      await runBrowserLaneForeground(unavailable, {
        invoke: async () => {
          invoked = true;
        },
        refresh: async () => {},
        setChangingVisibilityLaneId: () => {
          busy = true;
        },
        notifySuccess: () => undefined,
        notifyError: () => undefined,
        successMessage: 'opened',
      });
      expect(invoked).toBe(false);
      expect(busy).toBe(false);
    }
  });

  test('reports foreground failures and always releases busy state', async () => {
    const calls: string[] = [];
    await runBrowserLaneForeground(lane({ identity: { mode: 'primary' } }), {
      invoke: async () => {
        throw new Error('foreground failed');
      },
      refresh: async () => {
        calls.push('refresh');
      },
      setChangingVisibilityLaneId: (value) => calls.push(`busy:${value}`),
      notifySuccess: (message) => calls.push(`success:${message}`),
      notifyError: (message) => calls.push(`error:${message}`),
      successMessage: 'opened',
    });

    expect(calls).toEqual([
      'busy:lane-1',
      'refresh',
      'error:foreground failed',
      'busy:null',
    ]);
  });

  test('returns a foreground Primary lane to silent headless mode', async () => {
    const calls: string[] = [];
    await runBrowserLaneBackground(lane({ identity: { mode: 'primary' } }), {
      invoke: async ({ lane_id }) => {
        calls.push(`background:${lane_id}`);
        return { backgrounded: true, lane_id };
      },
      refresh: async () => {
        calls.push('refresh');
      },
      setChangingVisibilityLaneId: (value) => calls.push(`busy:${value}`),
      notifySuccess: (message) => calls.push(`success:${message}`),
      notifyError: (message) => calls.push(`error:${message}`),
      successMessage: 'headless',
    });

    expect(calls).toEqual([
      'busy:lane-1',
      'background:lane-1',
      'refresh',
      'success:headless',
      'busy:null',
    ]);
  });

  test('does not report foreground success when the response is unconfirmed or refresh fails', async () => {
    for (const scenario of [
      {
        invoke: async () => ({}),
        refresh: async () => undefined,
        expected: 'error:foreground unconfirmed',
      },
      {
        invoke: async () => ({ foregrounded: true }),
        refresh: async () => {
          throw new Error('inventory offline');
        },
        expected: 'error:foreground refresh failed: inventory offline',
      },
    ]) {
      const calls: string[] = [];
      await runBrowserLaneForeground(lane({ identity: { mode: 'primary' } }), {
        invoke: scenario.invoke,
        refresh: scenario.refresh,
        setChangingVisibilityLaneId: (value) => calls.push(`busy:${value}`),
        notifySuccess: (message) => calls.push(`success:${message}`),
        notifyError: (message) => calls.push(`error:${message}`),
        successMessage: 'opened',
        unconfirmedMessage: 'foreground unconfirmed',
        formatRefreshFailure: (message) => `foreground refresh failed: ${message}`,
      });

      expect(calls).toEqual(['busy:lane-1', scenario.expected, 'busy:null']);
      expect(calls.some((call) => call.startsWith('success:'))).toBe(false);
    }
  });

  test('closes queued and failed lanes without hiding lifecycle management', async () => {
    for (const candidate of [
      lane({
        lane_id: 'queued-lane',
        lifecycle_state: 'queued',
        queue: { position: 4, reason_code: 'system_memory_pressure' },
      }),
      lane({
        lane_id: 'failed-lane',
        lifecycle_state: 'failed',
        error_code: 'browser_unavailable',
        error_message: 'The managed browser is unavailable.',
      }),
    ]) {
      const calls: string[] = [];
      const busy: Array<string | null> = [];
      await runBrowserLaneClose(candidate, {
        invoke: async ({ lane_id }) => {
          calls.push(`close:${lane_id}`);
          return { closed: 1, already_closed: false };
        },
        refresh: async () => {
          calls.push('refresh');
        },
        setBusyLaneId: (value) => busy.push(value),
        notifySuccess: (message) => calls.push(`success:${message}`),
        notifyError: (message) => calls.push(`error:${message}`),
        successMessage: 'closed',
      });

      expect(calls).toEqual([
        `close:${candidate.lane_id}`,
        'refresh',
        'success:closed',
      ]);
      expect(busy).toEqual([candidate.lane_id, null]);
    }
  });

  test('reports close failures and always releases lane busy state', async () => {
    const busy: Array<string | null> = [];
    const errors: string[] = [];
    let refreshed = false;

    await runBrowserLaneClose(lane(), {
      invoke: async () => {
        throw new Error('lane close failed');
      },
      refresh: async () => {
        refreshed = true;
      },
      setBusyLaneId: (value) => busy.push(value),
      notifySuccess: () => {
        throw new Error('success should not be reported');
      },
      notifyError: (message) => errors.push(message),
      successMessage: 'closed',
    });

    expect(errors).toEqual(['lane close failed']);
    expect(refreshed).toBe(true);
    expect(busy).toEqual(['lane-1', null]);
  });

  test('parses aggregate partial failures and reports them instead of false success', async () => {
    const calls: string[] = [];
    const result = {
      closed: 1,
      failed_count: 2,
      failures: [
        {
          lane_id: 'lane-timeout',
          code: 'browser_close_timeout',
          message: 'Cleanup is still pending.',
        },
        {
          lane_id: 'lane-crashed',
          error: 'The target crashed during close.',
        },
      ],
    };
    expect(browserClosePartialFailureMessage(result)).toBe(
      'Some browser lanes could not be closed: Lane lane-timeout (browser_close_timeout): Cleanup is still pending.; Lane lane-crashed: The target crashed during close.'
    );
    expect(
      browserClosePartialFailureMessage(result, {
        withoutDetails: 'localized partial',
        withDetails: (details) => `localized: ${details}`,
      })?.startsWith('localized: Lane lane-timeout')
    ).toBe(true);

    await runBrowserConversationClose('conversation-partial', {
      invoke: async () => result,
      refresh: async () => {
        calls.push('refresh');
      },
      setBusyConversationId: (value) => calls.push(`busy:${value}`),
      notifySuccess: (message) => calls.push(`success:${message}`),
      notifyError: (message) => calls.push(`error:${message}`),
      successMessage: 'conversation closed',
    });

    expect(calls).toEqual([
      'busy:conversation-partial',
      'refresh',
      'error:Some browser lanes could not be closed: Lane lane-timeout (browser_close_timeout): Cleanup is still pending.; Lane lane-crashed: The target crashed during close.',
      'busy:null',
    ]);
  });

  test('understands map-shaped partial failures and always refreshes close-all', async () => {
    const calls: string[] = [];
    await runBrowserCloseAll({
      invoke: async () => ({
        data: {
          status: 'partial',
          failures: {
            'lane-a': {
              code: 'lane_closed_by_user',
              message: 'Already being cleaned up.',
            },
          },
        },
      }),
      refresh: async () => {
        calls.push('refresh');
      },
      setClosingAll: (value) => calls.push(`loading:${value}`),
      notifySuccess: (message) => calls.push(`success:${message}`),
      notifyError: (message) => calls.push(`error:${message}`),
      successMessage: 'all closed',
    });

    expect(calls).toEqual([
      'loading:true',
      'refresh',
      'error:Some browser lanes could not be closed: Lane lane-a (lane_closed_by_user): Already being cleaned up.',
      'loading:false',
    ]);
  });

  test('requires confirmation only when a lane has active browser work', async () => {
    expect(browserLaneHasActiveWork(lane({ active_operation: true }))).toBe(true);
    expect(browserLaneHasActiveWork(lane({ active_operation_count: 2 }))).toBe(true);
    expect(browserLaneHasActiveWork(lane({ active_operation_count: 0 }))).toBe(false);

    let closeCount = 0;
    const confirmations: BrowserConfirmationRequest[] = [];
    const close = async () => {
      closeCount++;
    };

    await requestBrowserLaneClose(
      lane({ lifecycle_state: 'queued' }),
      close,
      (request) => confirmations.push(request),
      confirmationCopy
    );
    expect(closeCount).toBe(1);
    expect(confirmations).toHaveLength(0);

    requestBrowserLaneClose(
      lane({ active_operation_count: 1 }),
      close,
      (request) => confirmations.push(request),
      confirmationCopy
    );
    expect(closeCount).toBe(1);
    expect(confirmations).toHaveLength(1);
    await confirmations[0]!.onOk();
    expect(closeCount).toBe(2);
  });

  test('closes a conversation through the exact authoritative id after confirmation', async () => {
    const calls: string[] = [];
    let confirmation: BrowserConfirmationRequest | undefined;

    requestBrowserConversationClose(
      conversationGroup({ conversationId: 'conversation-authoritative' }),
      (conversationId) =>
        runBrowserConversationClose(conversationId, {
          invoke: async ({ conversation_id }) => {
            calls.push(`close:${conversation_id}`);
            return { closed: 1, already_closed: false };
          },
          refresh: async () => {
            calls.push('refresh');
          },
          setBusyConversationId: (value) => calls.push(`busy:${value}`),
          notifySuccess: (message) => calls.push(`success:${message}`),
          notifyError: (message) => calls.push(`error:${message}`),
          successMessage: 'conversation closed',
        }),
      (request) => {
        confirmation = request;
      },
      confirmationCopy
    );

    expect(confirmation?.content).toBe(confirmationCopy.content);
    await confirmation?.onOk();
    expect(calls).toEqual([
      'busy:conversation-authoritative',
      'close:conversation-authoritative',
      'refresh',
      'success:conversation closed',
      'busy:null',
    ]);
  });

  test('does not offer conversation close for the unassigned group', () => {
    let confirmed = false;
    let closed = false;
    requestBrowserConversationClose(
      conversationGroup({ conversationId: null, key: '__browser_unassigned__' }),
      async () => {
        closed = true;
      },
      () => {
        confirmed = true;
      },
      confirmationCopy
    );
    expect(confirmed).toBe(false);
    expect(closed).toBe(false);
  });

  test('keeps close-all behind confirmation and clears loading after refresh failure', async () => {
    const calls: string[] = [];
    let confirmation: BrowserConfirmationRequest | undefined;
    requestBrowserCloseAll(
      () =>
        runBrowserCloseAll({
          invoke: async () => {
            calls.push('close-all');
            return {
              closed: 2,
              already_closed: false,
              remaining_lane_count: 0,
              remaining_cleanup_count: 0,
              remaining_managed_host_count: 0,
            };
          },
          refresh: async () => {
            throw new Error('refresh failed');
          },
          setClosingAll: (value) => calls.push(`loading:${value}`),
          notifySuccess: (message) => calls.push(`success:${message}`),
          notifyError: (message) => calls.push(`error:${message}`),
          successMessage: 'all closed',
        }),
      (request) => {
        confirmation = request;
      },
      confirmationCopy
    );

    expect(calls).toEqual([]);
    await confirmation?.onOk();
    expect(calls).toEqual([
      'loading:true',
      'close-all',
      'error:refresh failed',
      'loading:false',
    ]);
  });

  test('accepts explicit idempotent no-op but rejects malformed and zero-close responses', async () => {
    expect(browserCloseResultIsUnconfirmed({ closed: 1, already_closed: false })).toBe(false);
    expect(browserCloseResultIsUnconfirmed({ closed: 0, already_closed: true })).toBe(false);
    expect(browserCloseResultIsUnconfirmed({ closed: 0, already_closed: false })).toBe(true);
    expect(browserCloseResultIsUnconfirmed({})).toBe(true);
    expect(browserCloseResultIsUnconfirmed(undefined)).toBe(true);
    expect(
      browserCloseResultIsUnconfirmed(
        {
          closed: 1,
          remaining_lane_count: 0,
          remaining_cleanup_count: 0,
          remaining_managed_host_count: 0,
        },
        { requireFullyDrained: true }
      )
    ).toBe(false);
    expect(
      browserCloseResultIsUnconfirmed(
        {
          closed: 0,
          already_closed: false,
          remaining_lane_count: 0,
          remaining_cleanup_count: 0,
          remaining_managed_host_count: 0,
        },
        { requireFullyDrained: true }
      )
    ).toBe(false);
    for (const residual of [
      {
        closed: 1,
        remaining_lane_count: 1,
        remaining_cleanup_count: 0,
        remaining_managed_host_count: 0,
      },
      {
        closed: 1,
        remaining_lane_count: 0,
        remaining_cleanup_count: 1,
        remaining_managed_host_count: 0,
      },
      {
        closed: 1,
        remaining_lane_count: 0,
        remaining_cleanup_count: 0,
        remaining_managed_host_count: 1,
      },
      { closed: 1 },
    ]) {
      expect(
        browserCloseResultIsUnconfirmed(residual, {
          requireFullyDrained: true,
        })
      ).toBe(true);
    }

    const calls: string[] = [];
    await runBrowserCloseAll({
      invoke: async () => ({ closed: 0, already_closed: false }),
      refresh: async () => {
        calls.push('refresh');
      },
      setClosingAll: (value) => calls.push(`loading:${value}`),
      notifySuccess: (message) => calls.push(`success:${message}`),
      notifyError: (message) => calls.push(`error:${message}`),
      successMessage: 'all closed',
      unconfirmedMessage: 'close unconfirmed',
    });

    expect(calls).toEqual([
      'loading:true',
      'refresh',
      'error:close unconfirmed',
      'loading:false',
    ]);
  });

  test('never reports close-all success until every remaining resource count is zero', async () => {
    const calls: string[] = [];
    await runBrowserCloseAll({
      invoke: async () => ({
        closed: 1,
        already_closed: false,
        remaining_lane_count: 0,
        remaining_cleanup_count: 0,
        remaining_managed_host_count: 1,
      }),
      refresh: async () => {
        calls.push('refresh');
      },
      setClosingAll: (value) => calls.push(`loading:${value}`),
      notifySuccess: (message) => calls.push(`success:${message}`),
      notifyError: (message) => calls.push(`error:${message}`),
      successMessage: 'all resources closed',
      unconfirmedMessage: 'managed resources remain',
    });

    expect(calls).toEqual([
      'loading:true',
      'refresh',
      'error:managed resources remain',
      'loading:false',
    ]);
  });

  test('combines operation and refresh failures without reporting close success', async () => {
    const calls: string[] = [];
    await runBrowserLaneClose(lane(), {
      invoke: async () => {
        throw new Error('close failed');
      },
      refresh: async () => {
        throw new Error('refresh failed');
      },
      setBusyLaneId: (value) => calls.push(`busy:${value}`),
      notifySuccess: (message) => calls.push(`success:${message}`),
      notifyError: (message) => calls.push(`error:${message}`),
      successMessage: 'closed',
      formatRefreshFailure: (message) => `inventory failed: ${message}`,
    });

    expect(calls).toEqual([
      'busy:lane-1',
      'error:close failed; inventory failed: refresh failed',
      'busy:null',
    ]);
  });
});
