/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  BeginJavaScriptRuntimeSwitchRequest,
  DecideJavaScriptRuntimeSwitchRequest,
  JavaScriptRuntimeProbe,
  JavaScriptRuntimeRef,
  JavaScriptRuntimeStatus,
  RuntimeSwitchDecision,
} from '@/common/types/javascriptRuntime';

export interface RuntimeCandidateProjection {
  key: string;
  probe: JavaScriptRuntimeProbe;
  selected: boolean;
  pending: boolean;
  selectable: boolean;
}

const SOURCE_RANK: Record<JavaScriptRuntimeProbe['source'], number> = {
  manual_path: 0,
  process_path: 1,
  managed: 2,
};

const COMPATIBILITY_RANK: Record<
  JavaScriptRuntimeProbe['compatibility'],
  number
> = {
  recommended: 0,
  compatible: 1,
  incompatible: 2,
};

export function runtimeRefMatches(
  left: JavaScriptRuntimeRef | undefined,
  right: JavaScriptRuntimeRef | undefined
): boolean {
  return Boolean(
    left &&
      right &&
      left.runtime_installation_id === right.runtime_installation_id &&
      left.executable_digest === right.executable_digest
  );
}

export function probeForRuntime(
  status: JavaScriptRuntimeStatus,
  runtime: JavaScriptRuntimeRef | undefined
): JavaScriptRuntimeProbe | undefined {
  if (!runtime) return undefined;
  return status.probes.find((probe) =>
    runtimeRefMatches(probe.runtime, runtime)
  );
}

export function projectRuntimeCandidates(
  status: JavaScriptRuntimeStatus
): RuntimeCandidateProjection[] {
  return status.probes
    .map((probe, index) => {
      const selected = runtimeRefMatches(probe.runtime, status.selected);
      const pending = runtimeRefMatches(
        probe.runtime,
        status.pending_candidate
      );
      const runtimeIdentity =
        probe.runtime?.runtime_installation_id ?? `probe-${index}`;
      return {
        key: `${probe.source}:${probe.executable_path}:${runtimeIdentity}`,
        probe,
        selected,
        pending,
        selectable:
          Boolean(probe.runtime) &&
          probe.compatibility !== 'incompatible' &&
          status.download.state !== 'downloading' &&
          !status.pending_candidate &&
          !selected,
      };
    })
    .sort((left, right) => {
      if (left.selected !== right.selected) return left.selected ? -1 : 1;
      if (left.pending !== right.pending) return left.pending ? -1 : 1;
      return (
        SOURCE_RANK[left.probe.source] -
          SOURCE_RANK[right.probe.source] ||
        COMPATIBILITY_RANK[left.probe.compatibility] -
          COMPATIBILITY_RANK[right.probe.compatibility] ||
        left.probe.executable_path.localeCompare(
          right.probe.executable_path
        )
      );
    });
}

export function buildBeginRuntimeSwitchRequest(
  status: JavaScriptRuntimeStatus,
  candidate: JavaScriptRuntimeProbe,
  acknowledgeNonRecommendedRuntime: boolean
): BeginJavaScriptRuntimeSwitchRequest {
  if (!candidate.runtime || candidate.compatibility === 'incompatible') {
    throw new Error('A compatible Runtime probe is required');
  }

  return {
    expected_selection_revision: status.selection_revision,
    ...(status.selected
      ? {
          expected_selected_runtime_id:
            status.selected.runtime_installation_id,
          expected_selected_executable_digest:
            status.selected.executable_digest,
        }
      : {}),
    candidate_runtime_id: candidate.runtime.runtime_installation_id,
    expected_candidate_executable_digest:
      candidate.runtime.executable_digest,
    acknowledge_non_recommended_runtime:
      acknowledgeNonRecommendedRuntime,
  };
}

export function buildRuntimeSwitchDecisionRequest(
  status: JavaScriptRuntimeStatus,
  decision: RuntimeSwitchDecision
): DecideJavaScriptRuntimeSwitchRequest {
  const candidate = status.pending_candidate;
  if (!candidate || !status.requires_switch_decision) {
    throw new Error('A pending Runtime switch decision is required');
  }

  return {
    expected_selection_revision: status.selection_revision,
    candidate_runtime_id: candidate.runtime_installation_id,
    expected_candidate_executable_digest:
      candidate.executable_digest,
    decision,
  };
}

export function requiresNonRecommendedConfirmation(
  status: JavaScriptRuntimeStatus,
  candidate: JavaScriptRuntimeProbe
): boolean {
  const runtimeId = candidate.runtime?.runtime_installation_id;
  return Boolean(
    runtimeId &&
      candidate.compatibility === 'compatible' &&
      !status.non_recommended_warning_acknowledged.includes(runtimeId)
  );
}

export function probeForManagedOffer(
  status: JavaScriptRuntimeStatus
): JavaScriptRuntimeProbe | undefined {
  const offer = status.download_offer;
  if (!offer) return undefined;
  return status.probes.find(
    (probe) =>
      probe.source === 'managed' &&
      probe.runtime?.node_version === offer.node_version &&
      probe.runtime.runtime_target === offer.runtime_target &&
      probe.compatibility !== 'incompatible'
  );
}

export function runtimeStatusNeedsPolling(
  status: JavaScriptRuntimeStatus | null
): boolean {
  return status?.download.state === 'downloading';
}
