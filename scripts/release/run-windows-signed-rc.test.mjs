import { describe, expect, test } from 'bun:test';

import {
  assertSelfTest,
  signedRcPaths,
} from './run-windows-signed-rc.mjs';

describe('Windows signed RC orchestrator', () => {
  test('uses one immutable source-keyed root and lock path', () => {
    const paths = signedRcPaths('a'.repeat(40), 'C:\\repo');
    expect(paths.root.endsWith('windows-signed-rc\\aaaaaaaaa')).toBe(true);
    expect(paths.lock.endsWith('artifacts\\NomiFun.release-lock.json')).toBe(true);
  });

  test('keeps the no-side-effect self-test runnable', () => {
    expect(assertSelfTest().status).toBe('pass');
  });
});
