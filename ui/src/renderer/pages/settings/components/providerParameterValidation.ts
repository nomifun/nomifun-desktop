import type { ModelTask } from '@/common/protocolBindings/ModelTask';

export interface ProviderParameterIssue {
  code: 'provider_param_scope' | 'provider_param_type' | 'provider_param_owned' |
    'provider_param_output_limit' | 'invalid_max_tokens_field';
  parameter: string;
}

// Immediate editor preflight for known controls. Native provider options remain
// open-ended; save-time backend validation is authoritative.
const LOCAL_TRANSPORT_KEYS = [
  'base_url', 'base_url_override', 'allow_cross_origin_credentials', 'endpoint',
  'poll_endpoint', 'content_endpoint', 'realtime_endpoint', 'request_defaults',
  'request_body', 'protocol', 'connection', 'connection_id', 'connection_role',
  'auth', 'auth_scheme', 'credentials', 'api_key', 'api_keys', 'headers',
];
const OWNED_CHAT_FIELDS = [
  'model', 'messages', 'input', 'instructions', 'tools', 'tool_choice', 'system', 'stream',
  'previous_response_id', 'conversation', 'store',
];
const RESERVED_OUTPUT_FIELDS = new Set([
  ...LOCAL_TRANSPORT_KEYS, ...OWNED_CHAT_FIELDS,
  'include', 'reasoning', 'stream_options', 'text', 'response_format', 'output_config',
  'generationConfig', 'temperature', 'top_p', 'reasoning_effort', 'max_tokens_field',
  'require_reasoning_content', 'chain_rounds', '_nomifun_context_limit_kind',
  '_nomifun_gateway_catalog_baseline', 'systemInstruction', 'contents', 'toolConfig',
  'anthropic_version', 'parallel_tool_calls',
]);
const OUTPUT_LIMIT_FIELDS = ['max_tokens', 'max_completion_tokens', 'maxOutputTokens', 'max_output_tokens'];

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);
const owns = (value: Record<string, unknown>, key: string): boolean =>
  Object.prototype.hasOwnProperty.call(value, key);

export const isUnsafeMaxTokensField = (value: unknown): boolean =>
  typeof value !== 'string' || !value.trim() || /\s/u.test(value.trim()) || RESERVED_OUTPUT_FIELDS.has(value.trim());

/** Validate only local controls and serializer-owned fields, leaving native provider options open-ended. */
export const validateProviderParameterControls = (
  protocol: string,
  task: ModelTask,
  params: Record<string, unknown>,
): ProviderParameterIssue[] => {
  const issues: ProviderParameterIssue[] = [];
  const chat = task === 'chat';
  const openaiChat = chat && protocol === 'openai.chat_text';
  const responses = chat && protocol === 'openai.responses';
  const scope = (parameter: string, supported: boolean): boolean => {
    if (!owns(params, parameter)) return false;
    if (!supported) issues.push({ code: 'provider_param_scope', parameter });
    return supported;
  };
  if (scope('chain_rounds', responses) && typeof params.chain_rounds !== 'boolean') {
    issues.push({ code: 'provider_param_type', parameter: 'chain_rounds' });
  }
  if (scope('require_reasoning_content', openaiChat) && typeof params.require_reasoning_content !== 'boolean') {
    issues.push({ code: 'provider_param_type', parameter: 'require_reasoning_content' });
  }
  if (scope('stream_options', openaiChat)) {
    const options = params.stream_options;
    if (options !== null && (!isObject(options) ||
      (owns(options, 'include_usage') && typeof options.include_usage !== 'boolean'))) {
      issues.push({ code: 'provider_param_type', parameter: isObject(options)
        ? 'stream_options.include_usage' : 'stream_options' });
    }
  }
  if (scope('max_tokens_field', openaiChat || responses) && isUnsafeMaxTokensField(params.max_tokens_field)) {
    issues.push({ code: 'invalid_max_tokens_field', parameter: 'max_tokens_field' });
  }
  const localField = LOCAL_TRANSPORT_KEYS.find(key => owns(params, key));
  if (localField) issues.push({ code: 'provider_param_owned', parameter: localField });
  if (openaiChat || responses) {
    const ownedField = OWNED_CHAT_FIELDS.find(key => owns(params, key));
    if (ownedField) issues.push({ code: 'provider_param_owned', parameter: ownedField });
  }
  if (chat) {
    const maxField = typeof params.max_tokens_field === 'string' ? params.max_tokens_field.trim() : '';
    const outputField = OUTPUT_LIMIT_FIELDS.find(key => owns(params, key)) ||
      (maxField && owns(params, maxField) ? maxField : undefined);
    if (outputField) issues.push({ code: 'provider_param_output_limit', parameter: outputField });
    if (isObject(params.generationConfig) && owns(params.generationConfig, 'maxOutputTokens')) {
      issues.push({ code: 'provider_param_output_limit', parameter: 'generationConfig.maxOutputTokens' });
    }
  }
  return issues;
};
