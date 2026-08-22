/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(
  new URL('./CreativeCanvasAgentPanel.tsx', import.meta.url),
  'utf8'
);
const proposalSource = readFileSync(
  new URL('./proposalProjection.ts', import.meta.url),
  'utf8'
);

describe('Creative Canvas Agent product integration', () => {
  test('uses the owner-only resolver, real Nomi transport, and canonical CAS callbacks', () => {
    for (const token of [
      'createNomiCreativeStudioAgentSessionHttpPort()',
      'createNomiCreativeStudioAgentChatPort',
      'const operation = props.onPersist',
      'documentMutationRef',
      'pending.idempotencyKey',
      'pending.modelInput',
      'pending.skillIds',
      'serializeCreativeCanvasAgentModelInput',
      'selectCreativeCanvasAgentContextNodes',
      'CREATIVE_STUDIO_PLANNING_SKILLS',
      'projectCreativeCanvasAgentProposals',
      'proposalProjection.artifacts',
      'proposalApplyRef.current',
      'resolution.appliedProposalMessageIds',
      'refreshAuthority()',
      'props.onApplyCanvasOps(messageId, artifact.ops)',
      'onApplyProposal={handleApplyProposal}',
      "event.type === 'history-reconciled'",
      'creativeCanvasAgentSessionWithAuthoritativeHistory',
      'creativeCanvasAgentSessionWithoutPendingTurn',
    ]) {
      expect(source.includes(token)).toBe(true);
    }
    expect(source.includes('localStorage')).toBe(false);
    expect(source.includes('sessionStorage')).toBe(false);
    expect(source.includes('setTimeout')).toBe(false);
    expect(source.includes('Math.random')).toBe(false);
    expect(source.includes('inject_skills')).toBe(false);
  });

  test('fences admission and settles the exclusive turn before route exit', () => {
    expect(source.includes('await persistSession(pending)')).toBe(true);
    expect(source.includes('leaveEpochRef.current += 1')).toBe(true);
    expect(source.includes('admittedSend ?? Promise.resolve(true)')).toBe(true);
    expect(source.includes('controller.stop()')).toBe(true);
    expect(source.includes('await currentRunRef.current')).toBe(true);
    expect(source.includes('proposalApply?.then(')).toBe(true);
    expect(source.includes('input.skillIds.length > 3')).toBe(true);
    expect(source.includes('input.contextNodeIds')).toBe(true);
    expect(proposalSource.includes("state: 'invalid'")).toBe(true);
    expect(source.includes("state: 'failed'")).toBe(true);
  });
});
