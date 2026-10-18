import { expect, test } from 'bun:test';
import { normalizeAgentStreamError } from './chatLib';
import { normalizeSystemExecutionFailure } from './systemExecutionError';

test('canonical native failures preserve OS and dispatch facts without reading message text', () => {
  const diagnostic = { code: 'SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED', message: 'OS cancelled the runas request',
    kind: 'Uncategorized', raw_os_error: 1223, user_code_not_started: true, cleanup_proven: true };
  expect(normalizeAgentStreamError({ message: 'native failure', code: diagnostic.code, systemExecutionError: diagnostic })?.systemExecutionError).toEqual(diagnostic);
  const unknown = { code: 'SYSTEM_READ_RESULT_UNKNOWN', message: '1223 authentication cancelled is only prose',
    kind: null, raw_os_error: null, user_code_not_started: false, cleanup_proven: false };
  expect(normalizeAgentStreamError({ message: unknown.message, code: unknown.code, systemExecutionError: unknown })?.systemExecutionError).toEqual(unknown);
  expect(normalizeSystemExecutionFailure({ code: 'SYSTEM_EXECUTION_NOT_AVAILABLE', message: 'os error 5' })).toEqual({
    code: 'SYSTEM_EXECUTION_NOT_AVAILABLE', message: 'os error 5', kind: null, raw_os_error: null, user_code_not_started: null, cleanup_proven: null,
  });
  expect(normalizeAgentStreamError({ code: 'NOMIFUN_INTERNAL_ERROR', message: JSON.stringify(diagnostic) })?.systemExecutionError).toBeUndefined();
  for (const invalid of [{ ...diagnostic, raw_os_error: '1223' }, { ...diagnostic, user_code_not_started: 'true' },
    { ...diagnostic, kind: 'PermissionDenied os error 5' }]) expect(normalizeSystemExecutionFailure(invalid)).toBeUndefined();
});
