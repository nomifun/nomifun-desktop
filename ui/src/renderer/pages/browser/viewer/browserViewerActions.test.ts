/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import type { BrowserLaneControlState } from '@/common/browser/browserTypes';
import {
  requestBrowserViewerTakeover,
  returnBrowserViewerControl,
} from './browserViewerActions';

const viewerSource = readFileSync(new URL('./EmbeddedBrowserViewer.tsx', import.meta.url), 'utf8');

describe('browser viewer control actions', () => {
  test('wires takeover and return-control buttons to the tested control action layer', () => {
    expect(viewerSource.includes('requestBrowserViewerTakeover(sendCommand)')).toBe(true);
    expect(viewerSource.includes('returnBrowserViewerControl(lane.lane_id, {')).toBe(true);
    expect(viewerSource.includes('onClick={handleTakeControl}')).toBe(true);
    expect(viewerSource.includes('onClick={() => void handleReturnControl()}')).toBe(true);
  });

  test('keeps lane management and return-control independent from viewer retry state', () => {
    expect(viewerSource.includes("if (!canStream) {")).toBe(true);
    expect(viewerSource.includes('returnBrowserViewerControl(lane.lane_id, {')).toBe(true);
    expect(viewerSource.includes('disabled={connectionState !== \'streaming\'}\n            onClick={handleTakeControl}')).toBe(true);
    expect(viewerSource.includes("socketRef.current = null;\n                setRetryKey")).toBe(true);
  });

  test('sends an explicit lane-scoped takeover command', () => {
    const messages: Array<Record<string, unknown>> = [];
    expect(
      requestBrowserViewerTakeover((message) => {
        messages.push(message);
        return true;
      })
    ).toBe(true);
    expect(messages).toEqual([{ type: 'takeover' }]);
  });

  test('returns control through HTTP, updates local UI, notifies the socket, and refreshes inventory', async () => {
    const calls: string[] = [];
    let controlState: BrowserLaneControlState = 'user';

    await returnBrowserViewerControl('lane-user-controlled', {
      invoke: async ({ lane_id }) => {
        calls.push(`http:${lane_id}`);
      },
      send: (message) => {
        calls.push(`socket:${JSON.stringify(message)}`);
        return true;
      },
      refresh: async () => {
        calls.push('refresh');
      },
      setControlState: (state) => {
        controlState = state;
        calls.push(`control:${state}`);
      },
      setReturningControl: (returning) => calls.push(`loading:${returning}`),
      setViewerError: (error) => calls.push(`error:${error}`),
      formatError: (error) => String(error),
    });

    expect(controlState).toBe('agent');
    expect(calls).toEqual([
      'loading:true',
      'http:lane-user-controlled',
      'control:agent',
      'socket:{"type":"return_control"}',
      'refresh',
      'loading:false',
    ]);
  });

  test('keeps user control on failure, exposes a safe error, and releases loading', async () => {
    const calls: string[] = [];
    let controlState: BrowserLaneControlState = 'user';

    await returnBrowserViewerControl('lane-user-controlled', {
      invoke: async () => {
        throw new Error('profile C:\\secret\\Primary failed');
      },
      send: () => {
        calls.push('socket');
        return true;
      },
      refresh: async () => {
        calls.push('refresh');
      },
      setControlState: (state) => {
        controlState = state;
      },
      setReturningControl: (returning) => calls.push(`loading:${returning}`),
      setViewerError: (error) => calls.push(`error:${error}`),
      formatError: () => 'Unable to return control.',
    });

    expect(controlState).toBe('user');
    expect(calls).toEqual([
      'loading:true',
      'error:Unable to return control.',
      'loading:false',
    ]);
  });
});
