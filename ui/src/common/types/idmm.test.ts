import { expect, test } from 'bun:test';
import { normalizeIdmmDecisionExplanation, normalizeIdmmDecisionNotice } from './idmm';

const explanation = {
  intervention_id: '0190f5fe-7c00-7a00-8000-000000000081', source: 'bypass_model',
  reason_code: 'bypass_model_decision', rationale: 'Prefer a bounded cache.',
  model: { provider_id: '0190f5fe-7c00-7a00-8000-000000000082', model: 'step-3.5' },
  question: { message_id: '0190f5fe-7c00-7a00-8000-000000000083', sequence: 5, fingerprint: 'a'.repeat(64) },
};

test('decision metadata retains only the typed explanation and exact question reference', () => {
  expect(normalizeIdmmDecisionExplanation({ ...explanation, confidence: 0.99 })).toEqual(explanation);
  expect(normalizeIdmmDecisionExplanation({ origin: 'idmm', content: 'step-3.5 selected 2' })).toBeUndefined();
  for (const invalid of [
    { ...explanation, source: 'guessed' },
    { ...explanation, rationale: 'a'.repeat(41) },
    { ...explanation, rationale: 'line\nbreak' },
    { ...explanation, intervention_id: 'guessed' },
    { ...explanation, source: 'rule' },
    { ...explanation, model: undefined },
    { ...explanation, model: { provider_id: 'bad-id', model: 'step-3.5' } },
    { ...explanation, question: { ...explanation.question, sequence: 1.5 } },
    { ...explanation, question: { ...explanation.question, message_id: 'bad-id' } },
  ]) expect(normalizeIdmmDecisionExplanation(invalid)).toBeUndefined();
});

test('notice status and time are validated without promoting a Session pause state', () => {
  const notice = { decision: explanation, status: 'waiting_for_human', created_at: 1234 };
  expect(normalizeIdmmDecisionNotice(notice)).toEqual(notice);
  expect(normalizeIdmmDecisionNotice({ ...notice, status: 'session_paused' })).toBeUndefined();
  expect(normalizeIdmmDecisionNotice({ ...notice, created_at: 1e100 })).toBeUndefined();
  expect(normalizeIdmmDecisionNotice({ ...notice, decision: { ...explanation, question: undefined } })).toBeUndefined();
});
