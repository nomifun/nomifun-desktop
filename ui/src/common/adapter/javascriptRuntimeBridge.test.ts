/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import type {
  BeginJavaScriptRuntimeSwitchRequest,
  ConfirmJavaScriptRuntimeDownloadRequest,
  DecideJavaScriptRuntimeSwitchRequest,
  JavaScriptRuntimeStatus,
  ProbeJavaScriptRuntimeRequest,
} from '../types/javascriptRuntime';
import { javascriptRuntime } from './javascriptRuntimeBridge';

const realFetch = globalThis.fetch;

const status: JavaScriptRuntimeStatus = {
  selection_revision: 7,
  probes: [],
  switch_participants: [],
  requires_switch_decision: false,
  non_recommended_warning_acknowledged: [],
  download: {
    download_revision: 2,
    state: 'not_installed',
  },
};

type RecordedCall = {
  method: string;
  path: string;
  body: unknown;
};

const calls: RecordedCall[] = [];

function installFetchFixture(): void {
  globalThis.fetch = (async (input, init) => {
    const path = new URL(String(input), 'http://127.0.0.1').pathname;
    calls.push({
      method: init?.method ?? 'GET',
      path,
      body:
        typeof init?.body === 'string'
          ? JSON.parse(init.body)
          : undefined,
    });
    return new Response(JSON.stringify({ success: true, data: status }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }) as typeof fetch;
}

afterEach(() => {
  globalThis.fetch = realFetch;
  calls.length = 0;
});

describe('JavaScript Runtime bridge', () => {
  test('maps the five Runtime Manager actions without changing CAS bodies', async () => {
    installFetchFixture();

    const probe: ProbeJavaScriptRuntimeRequest = {
      source: 'manual_path',
      expected_selection_revision: 7,
      executable_path: String.raw`C:\Program Files\nodejs\node.exe`,
    };
    const download: ConfirmJavaScriptRuntimeDownloadRequest = {
      expected_selection_revision: 7,
      expected_offer_digest: 'a'.repeat(64),
    };
    const begin: BeginJavaScriptRuntimeSwitchRequest = {
      expected_selection_revision: 7,
      expected_selected_runtime_id: 'node-old',
      expected_selected_executable_digest: 'b'.repeat(64),
      candidate_runtime_id: 'node-new',
      expected_candidate_executable_digest: 'c'.repeat(64),
      acknowledge_non_recommended_runtime: false,
    };
    const decision: DecideJavaScriptRuntimeSwitchRequest = {
      expected_selection_revision: 8,
      candidate_runtime_id: 'node-new',
      expected_candidate_executable_digest: 'c'.repeat(64),
      decision: 'commit_candidate',
    };

    expect(await javascriptRuntime.status.invoke()).toEqual(status);
    await javascriptRuntime.probe.invoke(probe);
    await javascriptRuntime.download.invoke(download);
    await javascriptRuntime.beginSwitch.invoke(begin);
    await javascriptRuntime.decideSwitch.invoke(decision);

    expect(calls).toEqual([
      {
        method: 'GET',
        path: '/api/javascript-runtime/status',
        body: undefined,
      },
      {
        method: 'POST',
        path: '/api/javascript-runtime/probe',
        body: probe,
      },
      {
        method: 'POST',
        path: '/api/javascript-runtime/download',
        body: download,
      },
      {
        method: 'POST',
        path: '/api/javascript-runtime/switch/begin',
        body: begin,
      },
      {
        method: 'POST',
        path: '/api/javascript-runtime/switch/decision',
        body: decision,
      },
    ]);
  });
});
