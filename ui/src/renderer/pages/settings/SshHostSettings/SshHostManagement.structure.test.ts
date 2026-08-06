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

describe('SshHostManagement ~/.ssh/config import', () => {
  test('the empty state leads with the import, and says how many were found', () => {
    expect(src.includes("t('ssh.import.cta')")).toBe(true);
    expect(src.includes("t('ssh.import.detected', { count: cta.count })")).toBe(true);
  });

  test('the CTA choice goes through the shared pure rule, not an inline count', () => {
    // `hostBookPrimaryCta` is what guarantees a dead import button is never
    // rendered (no scan / no candidates falls back to Add).
    expect(src.includes('hostBookPrimaryCta')).toBe(true);
    expect(src.includes("cta.kind === 'import'")).toBe(true);
  });

  test('a non-empty book keeps an unobtrusive import entry', () => {
    expect(src.includes("t('ssh.import.entry')")).toBe(true);
    expect(src.includes("type='text'")).toBe(true);
  });

  test('the request carries aliases only — never a path the client made up', () => {
    expect(src.includes('aliases: candidates.map((candidate) => candidate.alias)')).toBe(true);
    // No identityFile / host / port in the import payload: the server re-reads
    // its own config, so this screen cannot name a file for it to open.
    expect(/importHosts\.invoke\(\{[^}]*identityFile/.test(src)).toBe(false);
  });

  test('reports what the import actually did, via the summary helper', () => {
    expect(src.includes('summarizeImport')).toBe(true);
    expect(src.includes('Message.warning(text)')).toBe(true);
    expect(src.includes("t('ssh.import.failed'")).toBe(true);
  });

  test('shows why the scan could not offer more', () => {
    expect(src.includes('scanNotes')).toBe(true);
  });

  test('does not re-derive an endpoint string by hand', () => {
    expect(src.includes('candidateEndpoint(candidate)')).toBe(true);
  });
});
