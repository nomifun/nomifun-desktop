import { describe, expect, test } from 'bun:test';
import { projectionCard } from './model';

describe('AgentSession projection cards', () => {
  test('renders canonical message content without a legacy chat-container fallback', () => {
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
        events: [{ seq: 3, kind: 'message/content-part', kind_version: 1, payload: null }],
      },
    });
    expect(card.kind).toBe('message');
    expect(card.content).toBe('hello');
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
        events: [{ seq: 8, kind: 'effect/uncertain', kind_version: 1, payload: null }],
      },
    });
    expect(card.kind).toBe('effect');
    expect(card.state).toBe('uncertain');
  });
});
