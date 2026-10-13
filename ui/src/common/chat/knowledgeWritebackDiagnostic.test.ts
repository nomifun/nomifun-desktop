import { expect, test } from 'bun:test';
import { normalizeKnowledgeWritebackDiagnostic } from './knowledgeWritebackDiagnostic';
import { normalizeKnowledgeWritebackState } from './chatLib';

test('typed write-back observations preserve real zero counts and reuse sanitized provider evidence', () => {
  const diagnostic = normalizeKnowledgeWritebackDiagnostic({ stage: 'first_frame', code: 'USER_LLM_PROVIDER_TIMEOUT', detail: 'The first frame was not received.',
    elapsed_ms: 12000, first_frame_ms: 0, received_frames: 0, received_bytes: 0, output_bytes: 0,
    input_tokens_estimate: 3000, output_token_limit: 1000, output_tokens: 0,
    provider_diagnostic: { reason: 'request_timeout', modelName: 'actual-request-model', requestId: 'req_123',
      endpoint: 'https://user:password-secret@api.example.com/v1?token=query-secret#fragment-secret',
      transportDetail: 'connection failed at https://user:password-secret@api.example.com/v1?token=query-secret\nAuthorization: Bearer auth-secret' },
    prompt: 'private-input', response: 'private-output', authorization: 'private-token',
  });
  expect(diagnostic).toMatchObject({ stage: 'first_frame', code: 'USER_LLM_PROVIDER_TIMEOUT', detail: 'The first frame was not received.', elapsed_ms: 12000,
    first_frame_ms: 0, received_frames: 0, received_bytes: 0, output_bytes: 0, input_tokens_estimate: 3000,
    output_token_limit: 1000, output_tokens: 0, provider_diagnostic: { reason: 'request_timeout',
      endpoint: 'https://api.example.com/v1', modelName: 'actual-request-model' } });
  for (const secret of ['password-secret', 'query-secret', 'fragment-secret', 'auth-secret', 'private-input', 'private-output', 'private-token']) {
    expect(JSON.stringify(diagnostic)).not.toContain(secret);
  }
});

test('invalid observations remain unknown and arbitrary prose does not supply a stage or failure cause', () => {
  expect(normalizeKnowledgeWritebackDiagnostic({ stage: 'unknown', code: 'USER_LLM_PROVIDER_TIMEOUT' })).toBeUndefined();
  expect(normalizeKnowledgeWritebackDiagnostic({ stage: 'decode', code: 'expired key; 401', elapsed_ms: -1,
    first_frame_ms: Number.NaN, received_frames: 1.5, received_bytes: Number.POSITIVE_INFINITY,
    output_bytes: Number.MAX_SAFE_INTEGER + 1, output_token_limit: 0x100000000,
    provider_diagnostic: { reason: 'invented', transportDetail: 'expired key' },
  })).toEqual({ stage: 'decode' });
  expect(normalizeKnowledgeWritebackDiagnostic({ stage: 'stream', elapsed_ms: 1000, first_frame_ms: 2000 }))
    .toEqual({ stage: 'stream', elapsed_ms: 1000 });
  expect(normalizeKnowledgeWritebackDiagnostic({ stage: 'write' })).toEqual({ stage: 'write' });
  expect(normalizeKnowledgeWritebackDiagnostic({ stage: 'decode', detail: 'x'.repeat(2049) })).toEqual({ stage: 'decode' });
});

test('the canonical notice chooses structured diagnostics over old detail and safely bounds old evidence', () => {
  const base = { status: 'failed', event_seq: 12, written: [] };
  expect(normalizeKnowledgeWritebackState({ ...base, detail: 'private old prose',
    diagnostic: { stage: 'decode', code: 'USER_LLM_PROVIDER_INVALID_RESPONSE', elapsed_ms: 5000 } }))
    .toEqual({ ...base, diagnostic: { stage: 'decode', code: 'USER_LLM_PROVIDER_INVALID_RESPONSE', elapsed_ms: 5000 } });
  const old = normalizeKnowledgeWritebackState({ ...base,
    detail: 'Authorization: Bearer auth-secret\nhttps://user:password-secret@api.example.com/v1?token=query-secret#fragment-secret\n' + 'x'.repeat(2500) });
  expect(old?.detail?.length).toBeLessThanOrEqual(2000);
  for (const secret of ['auth-secret', 'password-secret', 'query-secret', 'fragment-secret']) expect(old?.detail).not.toContain(secret);
});
