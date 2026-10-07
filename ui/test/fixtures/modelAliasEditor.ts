import { parseProviderId } from '../../src/common/types/ids';
import type { ProviderModelCapabilityResponse } from '../../src/common/types/provider/providerModel';
import type { ModelProtocolManifest } from '../../src/renderer/pages/settings/components/providerModelAdvanced';

export const aliasProviderId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000211');
export const aliasProviderBaseUrl = 'https://example.invalid/v1';
export const aliasCapability: ProviderModelCapabilityResponse = {
  task: 'chat', traits: ['vision_input'], protocol: 'openai.chat_text', connection_role: 'default',
  endpoint: '/chat/completions', allow_cross_origin_credentials: false,
  provider_params: { temperature: 0.4 }, context_limit: 64000,
  created_at: 1, updated_at: 1,
};
export const aliasProtocolManifest: ModelProtocolManifest = {
  tasks: ['chat'], preset: 'preview', platform: 'preview', requested_task: 'chat',
  platform_default_base_url: aliasProviderBaseUrl, requires_user_input: false,
  default_auth_scheme: 'bearer', auth_schemes: [{ scheme: 'bearer', parameterized: false }],
  recommendation: {
    protocol_id: 'openai.chat_text', connection_role: 'default',
    default_base_url: aliasProviderBaseUrl, default_auth_scheme: 'bearer', base_url_override_required: false,
  },
  protocols: [{
    protocol_id: 'openai.chat_text', root_shape: 'versioned_root', supported_tasks: ['chat'],
    executor: 'model_invoke', transport: 'http', requires_output_ceiling: false,
    allowed_auth_schemes: ['bearer'], scopes: ['native'], platforms: ['preview'], default_connections: [],
    endpoints: [{
      task: 'chat', field: 'endpoint', purpose: 'submit', method: 'POST', default_value: '/chat/completions',
      root_shape: 'versioned_root', allowed_placeholders: [], required_placeholders: [], editable: true,
    }],
  }],
};
