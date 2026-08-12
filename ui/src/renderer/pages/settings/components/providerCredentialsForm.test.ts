import { describe, expect, test } from 'bun:test';
import { buildBedrockConfig, buildProviderCredentials } from './providerCredentialsForm';

describe('provider credentials form mapping', () => {
  test('normal providers write the typed api_keys object', () => {
    expect(
      buildProviderCredentials({
        isBedrock: false,
        mode: 'create',
        hasStoredCredentials: false,
        apiKeysText: ' sk-a,\n sk-b ',
      })
    ).toEqual({ ok: true, credentials: { api_keys: ['sk-a', 'sk-b'] } });
  });

  test('normal provider edit preserves stored credentials when the form is empty', () => {
    expect(
      buildProviderCredentials({
        isBedrock: false,
        mode: 'update',
        hasStoredCredentials: true,
        apiKeysText: '',
      })
    ).toEqual({ ok: true });
    expect(
      buildProviderCredentials({
        isBedrock: false,
        mode: 'update',
        hasStoredCredentials: false,
        apiKeysText: '',
      })
    ).toEqual({ ok: false, error: 'api_keys_required' });
  });

  test('Bedrock accessKey credentials are typed and session token is optional', () => {
    expect(
      buildProviderCredentials({
        isBedrock: true,
        mode: 'create',
        hasStoredCredentials: false,
        bedrockAuthMethod: 'accessKey',
        accessKeyId: ' AKIA ',
        secretAccessKey: ' secret ',
        sessionToken: ' session ',
      })
    ).toEqual({
      ok: true,
      credentials: {
        access_key_id: 'AKIA',
        secret_access_key: 'secret',
        session_token: 'session',
      },
    });
  });

  test('Bedrock accessKey edit preserves a stored secret but rejects partial replacement', () => {
    expect(
      buildProviderCredentials({
        isBedrock: true,
        mode: 'update',
        hasStoredCredentials: true,
        bedrockAuthMethod: 'accessKey',
      })
    ).toEqual({ ok: true });
    expect(
      buildProviderCredentials({
        isBedrock: true,
        mode: 'update',
        hasStoredCredentials: true,
        bedrockAuthMethod: 'accessKey',
        accessKeyId: 'AKIA',
      })
    ).toEqual({ ok: false, error: 'bedrock_access_keys_incomplete' });
  });

  test('Profile and DefaultChain explicitly use empty credentials and never put secrets in config', () => {
    expect(
      buildProviderCredentials({
        isBedrock: true,
        mode: 'update',
        hasStoredCredentials: true,
        bedrockAuthMethod: 'profile',
      })
    ).toEqual({ ok: true, credentials: {} });
    expect(
      buildProviderCredentials({
        isBedrock: true,
        mode: 'create',
        hasStoredCredentials: false,
        bedrockAuthMethod: 'defaultChain',
      })
    ).toEqual({ ok: true, credentials: {} });
    expect(buildBedrockConfig('profile', ' us-east-1 ', ' dev ')).toEqual({
      auth_method: 'profile',
      region: 'us-east-1',
      profile: 'dev',
    });
    expect(buildBedrockConfig('defaultChain', ' us-west-2 ', 'ignored')).toEqual({
      auth_method: 'defaultChain',
      region: 'us-west-2',
    });
  });
});
