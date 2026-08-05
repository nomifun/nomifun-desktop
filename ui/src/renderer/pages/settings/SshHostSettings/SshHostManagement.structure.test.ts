/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('./SshHostManagement.tsx', import.meta.url), 'utf8');

describe('SshHostManagement structure', () => {
  test('uses Input.Password for secret fields', () => {
    expect(src.includes('Input.Password')).toBe(true);
  });

  test('renders the four auth methods conditionally via shouldUpdate noStyle', () => {
    expect(src.includes('shouldUpdate noStyle')).toBe(true);
    for (const opt of ["value='password'", "value='key'", "value='certificate'", "value='agent'"]) {
      expect(src.includes(opt)).toBe(true);
    }
  });

  test('goes through the masked-secret round-trip helper on update', () => {
    expect(src.includes('buildUpdatePayload')).toBe(true);
  });

  test('deletes via Modal.confirm (not a client-side hard delete)', () => {
    expect(src.includes('Modal.confirm')).toBe(true);
  });

  test('avoids the dead border-border-2 class (uses border-arco-2)', () => {
    expect(src.includes('border-border-2')).toBe(false);
    expect(src.includes('border-arco-2')).toBe(true);
  });

  test('uses semantic text tokens, not the brand-accent text-primary for copy', () => {
    expect(src.includes('text-t-primary')).toBe(true);
  });

  test('imports icons as bare named imports from @icon-park/react', () => {
    expect(/import \{[^}]*\} from '@icon-park\/react';/.test(src)).toBe(true);
    // no aliased icon imports (they break the build-time wrapper rewrite)
    expect(/from '@icon-park\/react';[\s\S]*\bas\b/.test(src.split('\n').slice(0, 20).join('\n'))).toBe(false);
  });
});
