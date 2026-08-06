/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  buildUpdatePayload,
  isMaskedSecret,
  isRealSecret,
  validateSshHostForm,
  type SshHostFormValues,
} from './sshHostForm.validation';

const base = (overrides: Partial<SshHostFormValues> = {}): SshHostFormValues => ({
  name: 'prod',
  host: '10.0.0.1',
  port: 22,
  username: 'deploy',
  authType: 'password',
  password: 'hunter2',
  ...overrides,
});

describe('SSH host form validation', () => {
  test('accepts a complete password host', () => {
    expect(validateSshHostForm(base())).toBeNull();
  });

  test('requires name/host/username', () => {
    expect(validateSshHostForm(base({ name: '  ' }))).toBe('ssh.validation.nameRequired');
    expect(validateSshHostForm(base({ host: '' }))).toBe('ssh.validation.hostRequired');
    expect(validateSshHostForm(base({ username: '' }))).toBe('ssh.validation.usernameRequired');
  });

  test('rejects out-of-range ports', () => {
    expect(validateSshHostForm(base({ port: 0 }))).toBe('ssh.validation.portRange');
    expect(validateSshHostForm(base({ port: 70000 }))).toBe('ssh.validation.portRange');
    expect(validateSshHostForm(base({ port: 22 }))).toBeNull();
  });

  test('password auth needs a password on create', () => {
    expect(validateSshHostForm(base({ password: '' }))).toBe('ssh.validation.passwordRequired');
  });

  test('key auth needs a private key on create', () => {
    expect(
      validateSshHostForm(base({ authType: 'key', password: null, privateKey: '' }))
    ).toBe('ssh.validation.privateKeyRequired');
    expect(
      validateSshHostForm(base({ authType: 'key', password: null, privateKey: '-----BEGIN...' }))
    ).toBeNull();
  });

  test('agent auth needs no stored secret', () => {
    expect(validateSshHostForm(base({ authType: 'agent', password: null }))).toBeNull();
  });

  test('on edit, a masked secret counts as present', () => {
    // create would fail (no real secret) but edit accepts the mask
    const masked = base({ password: '***' });
    expect(validateSshHostForm(masked, false)).toBe('ssh.validation.passwordRequired');
    expect(validateSshHostForm(masked, true)).toBeNull();
  });

  test('mask detection helpers', () => {
    expect(isMaskedSecret('***')).toBe(true);
    expect(isMaskedSecret(' *** ')).toBe(true);
    expect(isMaskedSecret('realpw')).toBe(false);
    expect(isRealSecret('***')).toBe(false);
    expect(isRealSecret('realpw')).toBe(true);
    expect(isRealSecret('')).toBe(false);
  });

  test('buildUpdatePayload strips masked secrets but keeps empty (clear) and real', () => {
    const out = buildUpdatePayload({
      name: 'x',
      password: '***', // unchanged → stripped
      sudoPassword: '', // explicit clear → kept
      privateKey: 'new-key', // changed → kept
    });
    expect('password' in out).toBe(false);
    expect(out.sudoPassword).toBe('');
    expect(out.privateKey).toBe('new-key');
    expect(out.name).toBe('x');
  });

  test('Test Connection is gated by the same rule as Save', () => {
    // The button has no rule of its own: it calls `validateSshHostForm(values,
    // isEdit)` exactly like Save. What it depends on is the `isEdit` relaxation
    // below — an edit-mode host whose password is still the mask is testable,
    // because the server holds the real one.
    expect(validateSshHostForm(base({ password: '' }), false)).toBe(
      'ssh.validation.passwordRequired'
    );
    expect(validateSshHostForm(base(), false)).toBeNull();
    expect(validateSshHostForm(base({ password: '***' }), true)).toBeNull();
  });
});
