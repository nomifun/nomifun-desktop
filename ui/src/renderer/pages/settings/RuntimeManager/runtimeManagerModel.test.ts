/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type {
  JavaScriptRuntimeProbe,
  JavaScriptRuntimeRef,
  JavaScriptRuntimeStatus,
} from '@/common/types/javascriptRuntime';
import {
  buildBeginRuntimeSwitchRequest,
  buildRuntimeSwitchDecisionRequest,
  probeForManagedOffer,
  probeForRuntime,
  projectRuntimeCandidates,
  requiresNonRecommendedConfirmation,
  runtimeStatusNeedsPolling,
} from './runtimeManagerModel';

const runtime = (
  id: string,
  digest: string,
  version = '24.8.0'
): JavaScriptRuntimeRef => ({
  runtime_installation_id: id,
  node_version: version,
  runtime_target: 'x86_64-pc-windows-msvc',
  executable_digest: digest.repeat(64),
});

const probe = (
  source: JavaScriptRuntimeProbe['source'],
  path: string,
  compatibility: JavaScriptRuntimeProbe['compatibility'],
  value?: JavaScriptRuntimeRef
): JavaScriptRuntimeProbe => ({
  source,
  executable_path: path,
  compatibility,
  runtime: value,
  ...(value ? {} : { error_code: 'NODE_PROBE_FAILED' }),
});

const status = (
  overrides: Partial<JavaScriptRuntimeStatus> = {}
): JavaScriptRuntimeStatus => ({
  selection_revision: 3,
  probes: [],
  switch_participants: [],
  requires_switch_decision: false,
  non_recommended_warning_acknowledged: [],
  download: {
    download_revision: 0,
    state: 'not_installed',
  },
  ...overrides,
});

describe('Runtime Manager model', () => {
  test.each(['download', 'pending'] as const)('candidates cannot switch during %s', busy => {
    const value = runtime('candidate', 'a');
    const candidate = probe('manual_path', '/node', 'recommended', value);
    const snapshot = status({
      probes: [candidate],
      ...(busy === 'download'
        ? { download: { download_revision: 1, state: 'downloading' as const } }
        : { pending_candidate: runtime('other', 'b') }),
    });
    expect(projectRuntimeCandidates(snapshot)[0]?.selectable).toBe(false);
  });

  test('builds an exact switch CAS from selected and candidate identities', () => {
    const selected = runtime('node-selected', 'a');
    const candidate = probe(
      'manual_path',
      String.raw`C:\node24\node.exe`,
      'recommended',
      runtime('node-candidate', 'b')
    );

    expect(
      buildBeginRuntimeSwitchRequest(
        status({ selected, probes: [candidate] }),
        candidate,
        false
      )
    ).toEqual({
      expected_selection_revision: 3,
      expected_selected_runtime_id: 'node-selected',
      expected_selected_executable_digest: 'a'.repeat(64),
      candidate_runtime_id: 'node-candidate',
      expected_candidate_executable_digest: 'b'.repeat(64),
      acknowledge_non_recommended_runtime: false,
    });
  });

  test('projects selected, pending, compatible, and failed probes without guessing identities', () => {
    const selected = runtime('node-selected', 'a');
    const pending = runtime('node-pending', 'b');
    const probes = [
      probe('managed', String.raw`C:\managed\node.exe`, 'recommended', pending),
      probe('manual_path', String.raw`C:\old\node.exe`, 'incompatible'),
      probe('process_path', String.raw`C:\node\node.exe`, 'recommended', selected),
    ];
    const snapshot = status({
      selected,
      pending_candidate: pending,
      probes,
    });
    const projected = projectRuntimeCandidates(snapshot);

    expect(projected.map(({ selected, pending, selectable }) => ({
      selected,
      pending,
      selectable,
    }))).toEqual([
      { selected: true, pending: false, selectable: false },
      { selected: false, pending: true, selectable: false },
      { selected: false, pending: false, selectable: false },
    ]);
    expect(probeForRuntime(snapshot, pending)?.source).toBe('managed');
  });

  test('requires one explicit acknowledgement for a compatible non-recommended runtime', () => {
    const candidateRuntime = runtime('node-22', 'c', '22.14.0');
    const candidate = probe(
      'manual_path',
      String.raw`C:\node22\node.exe`,
      'compatible',
      candidateRuntime
    );

    expect(
      requiresNonRecommendedConfirmation(status(), candidate)
    ).toBe(true);
    expect(
      requiresNonRecommendedConfirmation(
        status({
          non_recommended_warning_acknowledged: ['node-22'],
        }),
        candidate
      )
    ).toBe(false);
  });

  test('builds an exact decision and recognizes managed download state', () => {
    const pending = runtime('node-managed', 'd');
    const snapshot = status({
      selection_revision: 9,
      pending_candidate: pending,
      requires_switch_decision: true,
      probes: [
        probe(
          'managed',
          String.raw`C:\managed\node.exe`,
          'recommended',
          pending
        ),
      ],
      download_offer: {
        offer_digest: 'e'.repeat(64),
        node_version: pending.node_version,
        runtime_target: pending.runtime_target,
        archive_file_name: 'node-v24.8.0-win-x64.zip',
      },
      download: {
        download_revision: 2,
        state: 'downloading',
      },
    });

    expect(
      buildRuntimeSwitchDecisionRequest(
        snapshot,
        'abort_and_restore_selected'
      )
    ).toEqual({
      expected_selection_revision: 9,
      candidate_runtime_id: 'node-managed',
      expected_candidate_executable_digest: 'd'.repeat(64),
      decision: 'abort_and_restore_selected',
    });
    expect(probeForManagedOffer(snapshot)).toBe(snapshot.probes[0]);
    expect(runtimeStatusNeedsPolling(snapshot)).toBe(true);
  });
});
