/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TurnDisclosureProcessState } from './turnDisclosureModel';

/**
 * The live-step strip under the latest AI reply. It is the primary "still
 * working" signal now that the turn header reads "processed" throughout the
 * turn lifecycle; it disappears as soon as the turn settles.
 */
export type TurnLiveStepPlan =
  | { kind: 'item'; itemId: string; state: 'running' }
  | { kind: 'composing'; state: 'running' }
  | { kind: 'analyzing'; state: 'running' }
  | { kind: 'preparing'; state: 'running' };

export interface TurnLiveStepInput {
  isProcessing: boolean;
  /** Tail turn disclosure with effective per-item states, when one exists. */
  disclosure?: {
    running: boolean;
    processItems: Array<{ id: string; state: TurnDisclosureProcessState }>;
  };
  /** True when the final assistant reply text is still streaming in. */
  hasStreamingReplyText: boolean;
}

export function planTurnLiveStep(input: TurnLiveStepInput): TurnLiveStepPlan | null {
  if (!input.isProcessing) return null;
  const disclosure = input.disclosure;
  if (!disclosure || !disclosure.running) return null;

  const runningItem = disclosure.processItems.findLast((entry) => entry.state === 'running');
  if (runningItem) return { kind: 'item', itemId: runningItem.id, state: 'running' };

  if (input.hasStreamingReplyText) return { kind: 'composing', state: 'running' };
  if (disclosure.processItems.length === 0) return { kind: 'analyzing', state: 'running' };
  return { kind: 'preparing', state: 'running' };
}
