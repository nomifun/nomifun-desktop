/**
 * Authentication type for a model provider.
 *
 * String values are wire/serialized identifiers — they MUST stay stable, as
 * they are persisted and compared across the app and backend. (Previously
 * re-exported from a third-party package; inlined here to drop the dependency.)
 */
export enum AuthType {
  LOGIN_WITH_GOOGLE = 'oauth-personal',
  USE_GEMINI = 'gemini-api-key',
  USE_VERTEX_AI = 'vertex-ai',
  LEGACY_CLOUD_SHELL = 'cloud-shell',
  COMPUTE_ADC = 'compute-default-credentials',
  USE_OPENAI = 'openai',
  USE_ANTHROPIC = 'anthropic',
  USE_BEDROCK = 'bedrock',
}
