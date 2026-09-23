import { describe, expect, test } from 'bun:test';

import { evaluateAuthenticode } from './windows-authenticode.mjs';

describe('Windows Authenticode admission', () => {
  test('requires both a valid signer and a timestamp certificate', () => {
    expect(evaluateAuthenticode({
      status: 'Valid',
      signer_thumbprint: 'a'.repeat(40),
      timestamp_thumbprint: 'b'.repeat(40),
    }).status).toBe('pass');
    expect(evaluateAuthenticode({
      status: 'Valid',
      signer_thumbprint: 'a'.repeat(40),
      timestamp_thumbprint: null,
    }).status).toBe('fail');
    expect(evaluateAuthenticode({
      status: 'NotSigned',
      signer_thumbprint: null,
      timestamp_thumbprint: null,
    }).status).toBe('fail');
  });
});
