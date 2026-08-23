/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./EditModeModal.tsx', import.meta.url), 'utf8');

describe('provider detail auth-scheme lifecycle', () => {
  test('hydrates only from the persisted provider and never from an edit-time default', () => {
    expect(source.includes('auth_scheme: data.auth_scheme')).toBe(true);
    expect(
      source.includes("form.setFieldValue('auth_scheme', providerManifest.default_auth_scheme)")
    ).toBe(false);
  });

  test('offers resilient candidates and sends auth_scheme only after an explicit edit', () => {
    expect(source.includes('buildAuthSchemeOptions(')).toBe(true);
    expect(source.includes('filterOption={false}')).toBe(true);
    expect(source.includes('setAuthSchemeDirty(true)')).toBe(true);
    expect(source.includes('buildAuthSchemeEditPatch(')).toBe(true);
  });

  test('loads saved API keys and echoes them in the plaintext editor', () => {
    expect(source.includes('ipcBridge.mode.getProviderApiKeys')).toBe(true);
    expect(source.includes("form.setFieldValue('api_key', apiKeys.join(','))")).toBe(true);
    expect(source.includes('<Input.TextArea')).toBe(true);
    expect(source.includes('disabled={apiKeysLoading}')).toBe(true);
  });
});
