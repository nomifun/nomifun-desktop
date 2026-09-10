import { describe, expect, test } from 'bun:test';

import {
  assertSelfTest,
  evaluateAuthenticode,
} from './run-windows-signed-rc-product.mjs';

describe('Windows signed RC product admission', () => {
  test('requires both a valid signer and a timestamp certificate', () => {
    expect(
      evaluateAuthenticode({
        status: 'Valid',
        signer_thumbprint: 'a'.repeat(40),
        timestamp_thumbprint: 'b'.repeat(40),
      }).status,
    ).toBe('pass');
    expect(
      evaluateAuthenticode({
        status: 'Valid',
        signer_thumbprint: 'a'.repeat(40),
        timestamp_thumbprint: null,
      }).status,
    ).toBe('fail');
    expect(
      evaluateAuthenticode({
        status: 'NotSigned',
        signer_thumbprint: null,
        timestamp_thumbprint: null,
      }).status,
    ).toBe('fail');
  });

  test('keeps the standalone self-test runnable', () => {
    expect(assertSelfTest().status).toBe('pass');
  });
});
