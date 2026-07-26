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
  browserLaneHasActiveWork,
  requestBrowserCloseAll,
  requestBrowserConversationClose,
  requestBrowserLaneClose,
  runBrowserCloseAll,
  runBrowserConversationClose,
  runBrowserLaneClose,
  type BrowserConfirmationRequest,
} from './browserManagementActions';

const lane = (overrides: Partial<IBrowserLane> = {}): IBrowserLane => ({
  lane_id: 'lane-1',
  lifecycle_state: 'running',
  control_state: 'agent',
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

describe('browser management actions', () => {
  test('labels close-all as installation-wide instead of current-user scoped', () => {
    const en = browserInstallationWideCloseCopy('en-US');
    expect(en.title.includes('across this installation')).toBe(true);
    expect(en.warning.includes('installation-wide global')).toBe(true);
    expect(en.warning.includes("every user's")).toBe(true);
    expect(en.button.includes('globally')).toBe(true);

    const zh = browserInstallationWideCloseCopy('zh-CN');
    expect(zh.title.includes('整个安装')).toBe(true);
    expect(zh.warning.includes('全局')).toBe(true);
    expect(zh.warning.includes('所有用户')).toBe(true);
  });

  test('wires the visible page controls to the tested close action layer', () => {
    expect(browserPageSource.includes('runBrowserLaneClose(lane, {')).toBe(true);
    expect(browserPageSource.includes('requestBrowserLaneClose(lane, closeLane, confirmDanger')).toBe(
      true
    );
    expect(browserPageSource.includes('requestBrowserConversationClose(group, closeConversation')).toBe(
      true
    );
    expect(browserPageSource.includes('requestBrowserCloseAll(closeAll, confirmDanger')).toBe(true);
    expect(browserPageSource.includes('onCloseLane={handleCloseLane}')).toBe(true);
    expect(browserPageSource.includes('onCloseConversation={handleCloseConversation}')).toBe(true);
    expect(browserPageSource.includes('onCloseAll={handleCloseAll}')).toBe(true);
  });

  test('closes queued and stream-failed lanes without hiding management behind viewer state', async () => {
    for (const candidate of [
      lane({
        lane_id: 'queued-lane',
        lifecycle_state: 'queued',
        queue: { position: 4, reason_code: 'system_memory_pressure' },
      }),
      lane({
        lane_id: 'stream-failed-lane',
        viewer_state: 'failed',
        error_code: 'viewer_stream_failed',
        error_message: 'The embedded viewer failed.',
      }),
    ]) {
      const calls: string[] = [];
      const busy: Array<string | null> = [];
      await runBrowserLaneClose(candidate, {
        invoke: async ({ lane_id }) => {
          calls.push(`close:${lane_id}`);
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
        'success:closed',
        'refresh',
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
      'error:Some browser lanes could not be closed: Lane lane-timeout (browser_close_timeout): Cleanup is still pending.; Lane lane-crashed: The target crashed during close.',
      'refresh',
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
      'error:Some browser lanes could not be closed: Lane lane-a (lane_closed_by_user): Already being cleaned up.',
      'refresh',
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
      'success:conversation closed',
      'refresh',
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
      'success:all closed',
      'error:refresh failed',
      'loading:false',
    ]);
  });
});
