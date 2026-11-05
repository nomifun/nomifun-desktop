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
  validateSshConnectionForm,
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

  test('certificate authentication requires both the private key and certificate', () => {
    const certificateHost = base({ authType: 'certificate', password: null, privateKey: 'private-key', certificate: '' });
    expect(validateSshConnectionForm(certificateHost)).toBe('ssh.validation.certificateRequired');
    expect(validateSshHostForm(certificateHost)).toBe('ssh.validation.certificateRequired');
    expect(validateSshConnectionForm({ ...certificateHost, certificate: 'signed-certificate' })).toBeNull();
    expect(validateSshConnectionForm({ ...certificateHost, privateKey: '', certificate: 'signed-certificate' })).toBe('ssh.validation.privateKeyRequired');
    expect(validateSshConnectionForm({ ...certificateHost, privateKey: '***', certificate: '***' }, true)).toBeNull();
    expect(validateSshConnectionForm({ ...certificateHost, privateKey: '***', certificate: '***' }, false)).toBe('ssh.validation.privateKeyRequired');
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

  test('connection testing validates credentials and keeps the stored-secret edit rule', () => {
    expect(validateSshConnectionForm(base({ password: '' }), false)).toBe(
      'ssh.validation.passwordRequired'
    );
    expect(validateSshConnectionForm(base(), false)).toBeNull();
    expect(validateSshConnectionForm(base({ password: '***' }), true)).toBeNull();
  });

  test('a display name is required for saving and optional for testing', () => {
    const unnamed = base({ name: '' });
    expect(validateSshConnectionForm(unnamed)).toBeNull();
    expect(validateSshHostForm(unnamed)).toBe('ssh.validation.nameRequired');
  });
});
