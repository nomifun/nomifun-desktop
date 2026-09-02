import { describe, expect, test } from 'bun:test';
import { projectionCard } from './model';

describe('AgentSession projection cards', () => {
  test('renders canonical message content without a legacy events projection', () => {
    const card = projectionCard({
      session_id: 'session' as never,
      projection_id: 'message:m1',
      first_seq: 3,
      last_seq: 5,
      presentation_intent: 'message',
      semantic_digest: 'digest',
      projection: {
        projection_id: 'message:m1',
        correlation_id: 'm1',
        presentation_intent: 'message',
        state: 'completed',
        content: 'hello',
      } as never,
    });
    expect(card.kind).toBe('message');
    expect(card.role).toBe('assistant');
    expect(card.content).toBe('hello');
  });

  test('uses the bounded tool summary when the projection has no events', () => {
    const card = projectionCard({
      session_id: 'session' as never,
      projection_id: 'tool:t1',
      first_seq: 5,
      last_seq: 6,
      presentation_intent: 'tool',
      semantic_digest: 'digest',
      projection: {
        projection_id: 'tool:t1',
        correlation_id: 't1',
        presentation_intent: 'tool',
        state: 'recorded',
        tool_summary: {
          action_id: 'files.read',
          result_state: 'recorded',
          result_digest: 'digest',
        },
      } as never,
    });
    expect(card.title).toBe('files.read');
    expect(card.details).toEqual({
      action_id: 'files.read',
      result_state: 'recorded',
      result_digest: 'digest',
    });
  });

  test('keeps uncertain effects explicit', () => {
    const card = projectionCard({
      session_id: 'session' as never,
      projection_id: 'effect:e1',
      first_seq: 7,
      last_seq: 8,
      presentation_intent: 'effect',
      semantic_digest: 'digest',
      projection: {
        projection_id: 'effect:e1',
        correlation_id: 'e1',
        presentation_intent: 'effect',
        state: 'uncertain',
        terminal_effect: { effect: 'workspace.write', state: 'uncertain' },
      } as never,
    });
    expect(card.kind).toBe('effect');
    expect(card.state).toBe('uncertain');
    expect(card.title).toBe('workspace.write');
  });

  test('reads legacy events only as a compatibility fallback', () => {
    const card = projectionCard({
      session_id: 'session' as never,
      projection_id: 'message:legacy',
      first_seq: 2,
      last_seq: 3,
      presentation_intent: 'message',
      semantic_digest: 'digest',
      projection: {
        projection_id: 'message:legacy',
        correlation_id: 'legacy',
        presentation_intent: 'message',
        state: 'streaming',
        events: [
          {
            kind: 'message/user-accepted',
            payload: { content: 'hello' },
          },
        ],
      } as never,
    });
    expect(card.role).toBe('user');
    expect(card.content).toBe('hello');
  });
});
