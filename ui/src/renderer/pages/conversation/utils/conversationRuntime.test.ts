import { describe, expect, test } from 'bun:test';
import type { TChatConversation } from '@/common/config/storage';
import { parseMessageId } from '@/common/types/ids';
import {
  getConversationRuntimeAuthority,
  isConversationProcessing,
  getConversationPauseNotice,
} from './conversationRuntime';

const activeTurnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000081');

describe('conversation runtime authority', () => {
  test('pause presentation requires the exact nonprocessing owner and never grants idle authority', () => {
    const snapshot = {
      status: 'running', extra: { workspace: '/fixture', execution_phase: 'paused', execution_pause: {
        reason: 'EXECUTION_MODEL_PROVIDER_UNAVAILABLE', cleanup_proven: true, paused_at_ms: 123,
      } },
      runtime: { state: 'idle', has_runtime: false, is_processing: false, can_send_message: false, active_turn_id: activeTurnId },
    } as TChatConversation;
    expect(getConversationPauseNotice(snapshot)).toEqual({ turnId: activeTurnId,
      reason: 'EXECUTION_MODEL_PROVIDER_UNAVAILABLE', cleanupProven: true, pausedAt: 123 });
    expect(getConversationRuntimeAuthority(snapshot)).toBe('unknown');
    for (const change of [
      { status: 'finished' },
      { extra: { ...snapshot.extra, execution_phase: 'running' } },
      ...[{ is_processing: true }, { active_turn_id: undefined }, { can_send_message: true }, { state: 'running' }]
        .map(runtime => ({ runtime: { ...snapshot.runtime, ...runtime } })),
    ]) expect(getConversationPauseNotice({ ...snapshot, ...change } as TChatConversation)).toBeNull();
    expect(getConversationPauseNotice({ ...snapshot, extra: { ...snapshot.extra,
      execution_pause: { reason: 'untrusted detail', cleanup_proven: false, paused_at_ms: Number.NaN },
    } })).toEqual({ turnId: activeTurnId, reason: undefined, cleanupProven: false, pausedAt: undefined });
  });

  test('Finished cannot be promoted by a stale processing bit', () => {
    const snapshot = {
      status: 'finished',
      runtime: {
        is_processing: true,
        active_turn_id: activeTurnId,
      },
    } as TChatConversation;

    expect(getConversationRuntimeAuthority(snapshot)).toBe('idle');
    expect(isConversationProcessing(snapshot)).toBe(false);
  });

  test('Running requires an exact active turn projection', () => {
    const incomplete = {
      status: 'running',
      runtime: { is_processing: true },
    } as TChatConversation;
    const exact = {
      status: 'running',
      runtime: {
        is_processing: true,
        active_turn_id: activeTurnId,
      },
    } as TChatConversation;

    expect(getConversationRuntimeAuthority(incomplete)).toBe('unknown');
    expect(isConversationProcessing(incomplete)).toBe(false);
    expect(getConversationRuntimeAuthority(exact)).toBe('processing');
    expect(isConversationProcessing(exact)).toBe(true);
  });

  test('a paused canonical turn never grants idle authority to the command queue', () => {
    const snapshot = {
      status: 'running', extra: { execution_phase: 'paused' },
      runtime: { state: 'running', has_runtime: true, is_processing: false, can_send_message: false, active_turn_id: activeTurnId },
    } satisfies Pick<TChatConversation, 'status' | 'runtime'> & { extra: { execution_phase: 'paused' } };
    expect(getConversationRuntimeAuthority(snapshot)).toBe('unknown');
    expect(isConversationProcessing(snapshot)).toBe(false);
  });

  test('Pending is idle only when no processing projection exists', () => {
    expect(
      getConversationRuntimeAuthority({
        status: 'pending',
        runtime: { is_processing: false },
      } as TChatConversation)
    ).toBe('idle');
    expect(
      getConversationRuntimeAuthority({
        status: 'pending',
        runtime: { is_processing: true },
      } as TChatConversation)
    ).toBe('unknown');
    expect(
      getConversationRuntimeAuthority({
        status: 'pending',
        runtime: {
          is_processing: false,
          active_turn_id: activeTurnId,
        },
      } as TChatConversation)
    ).toBe('unknown');
  });
});
