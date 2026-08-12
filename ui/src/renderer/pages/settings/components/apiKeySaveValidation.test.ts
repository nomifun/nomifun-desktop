/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('API key save validation wiring', () => {
  test('keeps explicit key tests but does not block provider creation on a catalog probe', () => {
    const addSource = readSource(new URL('./AddPlatformModal.tsx', import.meta.url));
    const editSource = readSource(new URL('./EditModeModal.tsx', import.meta.url));
    const editorSource = readSource(new URL('./ApiKeyEditorModal.tsx', import.meta.url));

    expect(addSource.includes('validateApiKeysForSave')).toBe(false);
    expect(addSource.includes('removeInvalidApiKeysBeforeSave')).toBe(false);
    expect(addSource.includes('buildProviderCredentials')).toBe(true);

    for (const source of [addSource, editSource]) {
      expect(source.includes('validateApiKeysForSave')).toBe(false);
      expect(source.includes('removeInvalidApiKeysBeforeSave')).toBe(false);
      expect(source.includes('buildProviderCredentials')).toBe(true);
    }

    // Provider persistence is independent of the optional model catalog probe.
    expect(addSource.includes('credentials: credentialBuild.credentials')).toBe(true);
    expect(editSource.includes('const patch: EditProviderPatch')).toBe(true);
    const providerPatch = editSource.slice(editSource.indexOf('const patch: EditProviderPatch'));
    expect(providerPatch.includes('...data,')).toBe(false);
    expect(providerPatch.includes('auth_scheme: String(values.auth_scheme).trim()')).toBe(true);
    expect(providerPatch.includes("credentials === undefined")).toBe(true);
    // The standalone explicit-key tester still normalizes its textarea input.
    expect(editorSource.includes('normalizeApiKeyList')).toBe(true);
    expect(editorSource.includes('onTestKey')).toBe(true);
    expect(editorSource.includes('onSave(normalized)')).toBe(true);
  });

  test('provider base URL and explicit authentication remain editable', () => {
    const editSource = readSource(new URL('./EditModeModal.tsx', import.meta.url));
    expect(editSource.includes("field='base_url'")).toBe(true);
    expect(editSource.includes("field='auth_scheme'")).toBe(true);
    expect(editSource.includes('<AutoComplete')).toBe(true);
    expect(editSource.includes('providerManifest?.auth_schemes')).toBe(true);
  });

  test('Bedrock exposes all three auth modes while keeping secrets out of metadata', () => {
    const addSource = readSource(new URL('./AddPlatformModal.tsx', import.meta.url));
    const editSource = readSource(new URL('./EditModeModal.tsx', import.meta.url));
    const credentialSource = readSource(new URL('./providerCredentialsForm.ts', import.meta.url));

    for (const source of [addSource, editSource]) {
      expect(source.includes("value='accessKey'")).toBe(true);
      expect(source.includes("value='profile'")).toBe(true);
      expect(source.includes("value='defaultChain'")).toBe(true);
      expect(source.includes("field='bedrockSessionToken'")).toBe(true);
      expect(source.includes('buildBedrockConfig')).toBe(true);
    }
    expect(credentialSource.includes('access_key_id: accessKeyId')).toBe(true);
    expect(credentialSource.includes('secret_access_key: secretAccessKey')).toBe(true);
    const configBuilder = credentialSource.slice(
      credentialSource.indexOf('export const buildBedrockConfig')
    );
    expect(configBuilder.includes('auth_method: authMethod')).toBe(true);
    expect(configBuilder.includes('access_key_id')).toBe(false);
    expect(configBuilder.includes('secret_access_key')).toBe(false);
    expect(configBuilder.includes('session_token')).toBe(false);
  });
});
