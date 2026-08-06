/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** Auth methods a saved SSH host can use. */
export type SshAuthType = 'password' | 'key' | 'certificate' | 'agent';

/** The masked sentinel the server returns for a stored secret. A field still
 *  equal to this on update means "unchanged; do not resend". */
export const SSH_SECRET_MASK = '***';

/** Editable form values for the add/edit-host form. */
export type SshHostFormValues = {
  name: string;
  host: string;
  port: number;
  username: string;
  authType: SshAuthType;
  password?: string | null;
  privateKey?: string | null;
  passphrase?: string | null;
  certificate?: string | null;
  sudoPassword?: string | null;
};

/** i18n keys for field-level validation errors (namespace `ssh`). */
export type SshHostValidationKey =
  | 'ssh.validation.nameRequired'
  | 'ssh.validation.hostRequired'
  | 'ssh.validation.portRange'
  | 'ssh.validation.usernameRequired'
  | 'ssh.validation.passwordRequired'
  | 'ssh.validation.privateKeyRequired';

const hasText = (v?: string | null): boolean => Boolean(v && v.trim().length > 0);

/** True when a credential value is present and NOT the masked sentinel. */
export const isRealSecret = (v?: string | null): boolean =>
  hasText(v) && v!.trim() !== SSH_SECRET_MASK;

/** True when a secret field still holds the masked sentinel (unchanged). */
export const isMaskedSecret = (v?: string | null): boolean =>
  (v ?? '').trim() === SSH_SECRET_MASK;

/**
 * Validate the host form. Returns the first blocking error key, or null.
 * `isEdit` relaxes credential-presence checks: on edit a masked value counts as
 * a present secret (it is kept server-side), so password/key need not be typed.
 */
export function validateSshHostForm(
  values: SshHostFormValues,
  isEdit: boolean = false
): SshHostValidationKey | null {
  if (!hasText(values.name)) return 'ssh.validation.nameRequired';
  if (!hasText(values.host)) return 'ssh.validation.hostRequired';
  if (!Number.isInteger(values.port) || values.port < 1 || values.port > 65535) {
    return 'ssh.validation.portRange';
  }
  if (!hasText(values.username)) return 'ssh.validation.usernameRequired';

  const secretPresent = (raw?: string | null): boolean =>
    isRealSecret(raw) || (isEdit && isMaskedSecret(raw));

  switch (values.authType) {
    case 'password':
      if (!secretPresent(values.password)) return 'ssh.validation.passwordRequired';
      break;
    case 'key':
    case 'certificate':
      if (!secretPresent(values.privateKey)) return 'ssh.validation.privateKeyRequired';
      break;
    case 'agent':
      // No stored secret — auth is delegated to the operator's ssh-agent.
      break;
  }
  return null;
}

/**
 * Strip credential fields still equal to the mask from an update payload, so an
 * unchanged secret is never resent (the server keeps the stored ciphertext).
 * Empty string is preserved (it explicitly clears the secret).
 */
export function buildUpdatePayload(values: Partial<SshHostFormValues>): Partial<SshHostFormValues> {
  const out: Partial<SshHostFormValues> = { ...values };
  for (const key of ['password', 'privateKey', 'passphrase', 'certificate', 'sudoPassword'] as const) {
    if (isMaskedSecret(out[key] as string | null | undefined)) {
      delete out[key];
    }
  }
  return out;
}
