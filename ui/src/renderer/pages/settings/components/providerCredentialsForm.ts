/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IProvider } from '@/common/config/storage';
import type { ProviderCredentials } from '@/common/types/provider/providerApi';
import { parseApiKeyList } from '@/common/utils/apiKeys';

export type BedrockAuthMethod = NonNullable<IProvider['bedrock_config']>['auth_method'];
export type ProviderCredentialSaveMode = 'create' | 'update';

export interface ProviderCredentialsDraft {
  isBedrock: boolean;
  mode: ProviderCredentialSaveMode;
  hasStoredCredentials: boolean;
  apiKeysText?: string | null;
  bedrockAuthMethod?: BedrockAuthMethod | null;
  accessKeyId?: string | null;
  secretAccessKey?: string | null;
  sessionToken?: string | null;
}

export type ProviderCredentialsBuildResult =
  | {
      ok: true;
      /** Undefined is meaningful only for update: preserve the encrypted value. */
      credentials?: ProviderCredentials;
    }
  | {
      ok: false;
      error:
        | 'api_keys_required'
        | 'bedrock_auth_method_required'
        | 'bedrock_access_keys_required'
        | 'bedrock_access_keys_incomplete';
    };

/**
 * Map the provider credentials form to the backend's write-only typed JSON.
 * Existing secrets are never loaded into the form. In update mode, an empty
 * form preserves stored credentials only when the response says they exist.
 */
export const buildProviderCredentials = (
  draft: ProviderCredentialsDraft
): ProviderCredentialsBuildResult => {
  if (!draft.isBedrock) {
    const keys = parseApiKeyList(draft.apiKeysText);
    if (keys.length > 0) {
      return { ok: true, credentials: { api_keys: keys } };
    }
    if (draft.mode === 'update' && draft.hasStoredCredentials) {
      return { ok: true };
    }
    return { ok: false, error: 'api_keys_required' };
  }

  if (!draft.bedrockAuthMethod) {
    return { ok: false, error: 'bedrock_auth_method_required' };
  }
  if (draft.bedrockAuthMethod === 'profile' || draft.bedrockAuthMethod === 'defaultChain') {
    // Explicitly clear access-key material when changing to an AWS provider chain.
    return { ok: true, credentials: {} };
  }

  const accessKeyId = draft.accessKeyId?.trim() ?? '';
  const secretAccessKey = draft.secretAccessKey?.trim() ?? '';
  const sessionToken = draft.sessionToken?.trim() ?? '';
  const hasAnyInput = Boolean(accessKeyId || secretAccessKey || sessionToken);
  if (!hasAnyInput) {
    if (draft.mode === 'update' && draft.hasStoredCredentials) {
      return { ok: true };
    }
    return { ok: false, error: 'bedrock_access_keys_required' };
  }
  if (!accessKeyId || !secretAccessKey) {
    return { ok: false, error: 'bedrock_access_keys_incomplete' };
  }
  return {
    ok: true,
    credentials: {
      access_key_id: accessKeyId,
      secret_access_key: secretAccessKey,
      ...(sessionToken ? { session_token: sessionToken } : {}),
    },
  };
};

/** Construct only the non-secret Bedrock metadata returned by provider APIs. */
export const buildBedrockConfig = (
  authMethod: BedrockAuthMethod,
  region: string,
  profile?: string | null
): NonNullable<IProvider['bedrock_config']> => ({
  auth_method: authMethod,
  region: region.trim(),
  ...(authMethod === 'profile' ? { profile: profile?.trim() ?? '' } : {}),
});

