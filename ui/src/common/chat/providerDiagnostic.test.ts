import { expect, test } from 'bun:test';
import { normalizeAgentStreamError } from './chatLib';
import { diagnosticEndpoint, normalizeModelFailureDiagnostic } from './providerDiagnostic';

test('typed request evidence survives normalization while secrets and malformed fields do not', () => {
  const normalized = normalizeModelFailureDiagnostic({ reason: 'expired_key', httpStatus: 401,
    providerId: '0190f5fe-7c00-7a00-8000-000000000001', modelName: 'actual-model',
    providerCode: 'key_expired', providerType: 'authentication_error', providerParam: 'headers.Authorization',
    endpoint: 'https://user:secret@api.example.com/v1/chat?api_key=query-secret#fragment-secret',
    requestId: 'req_123', retryAfterMs: 0, protocol: 'openai_chat', authScheme: 'bearer', contentType: 'application/json',
    transportDetail: 'could not connect: https://user:secret@api.example.com/v1?token=token-secret\nAuthorization: Bearer auth-secret',
  });
  expect(normalized?.endpoint).toBe('https://api.example.com/v1/chat');
  expect(normalized?.httpStatus).toBe(401);
  expect(normalized?.retryAfterMs).toBe(0);
  expect(normalized?.providerParam).toBe('headers.Authorization');
  expect(normalized?.modelName).toBe('actual-model');
  for (const secret of ['user:secret', 'query-secret', 'fragment-secret', 'token-secret', 'auth-secret']) {
    expect(JSON.stringify(normalized)).not.toContain(secret);
  }
  expect(normalizeModelFailureDiagnostic({ reason: 'invented' })).toBeUndefined();
  expect(normalizeModelFailureDiagnostic({ reason: 'network_failure', httpStatus: Infinity, retryAfterMs: -1,
    endpoint: 'javascript:alert(1)', requestId: 'Bearer secret', providerCode: 'sk-private', providerParam: 'api_key=secret',
  })).toEqual({ reason: 'network_failure' });
  expect(diagnosticEndpoint('https://api.example.com/v1?public=also-removed')).toBe('https://api.example.com/v1');
});

test('diagnostic prose never supplies or changes a provider reason', () => {
  const error = { message: '401 key expired quota insufficient', code: 'USER_LLM_PROVIDER_UNAVAILABLE',
    detail: 'invalid_api_key: token expired; permission denied' };
  expect(normalizeAgentStreamError(error)?.providerDiagnostic).toBeUndefined();
  const typed = { reason: 'auth_failed' as const, httpStatus: 401 };
  expect(normalizeAgentStreamError({ ...error, providerDiagnostic: typed })?.providerDiagnostic).toEqual(typed);
});

test('provider envelope detail remains sanitized evidence and cannot change the typed cause', () => {
  const providerDetail = 'Parameter temperature is not supported; quoted example says key expired. '
    + 'https://user:password-secret@api.example.com/v1?token=query-secret\nAuthorization: Bearer header-secret';
  const diagnostic = normalizeModelFailureDiagnostic({ reason: 'invalid_request', providerDetail });
  expect(diagnostic?.reason).toBe('invalid_request');
  expect(diagnostic?.providerDetail).toContain('Parameter temperature is not supported');
  for (const secret of ['password-secret', 'query-secret', 'header-secret']) {
    expect(diagnostic?.providerDetail).not.toContain(secret);
  }
  expect(normalizeModelFailureDiagnostic({ reason: 'invented', providerDetail })).toBeUndefined();
  expect(normalizeModelFailureDiagnostic({ reason: 'invalid_request', providerDetail: 'a'.repeat(2500) })?.providerDetail?.length).toBe(2000);
});
