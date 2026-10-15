export interface ChatProtocolTransitionIssue {
  parameter: string;
  kind: 'conflict' | 'unsupported';
}
export interface ChatProtocolTransitionResult {
  params: Record<string, unknown>;
  suspended: Record<string, unknown>;
  issues: ChatProtocolTransitionIssue[];
}
export const isOpenAIChatProtocol = (protocol: string): boolean =>
  protocol === 'openai.chat_text' || protocol === 'openai.responses';
const object = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);
const owns = (value: Record<string, unknown>, key: string): boolean => Object.prototype.hasOwnProperty.call(value, key);
const stable = (value: unknown): unknown => Array.isArray(value) ? value.map(stable) : object(value)
  ? Object.fromEntries(Object.keys(value).sort().map(key => [key, stable(value[key])])) : value;
export const equalProtocolParamValue = (left: unknown, right: unknown): boolean => JSON.stringify(stable(left)) === JSON.stringify(stable(right));
const equal = equalProtocolParamValue;
const empty = (value: unknown): boolean => value === null || (object(value) && Object.keys(value).length === 0);

function convertFormat(value: unknown, toResponses: boolean): Record<string, unknown> | undefined {
  if (!object(value)) return undefined;
  if (value.type === 'text' || value.type === 'json_object') return { ...value };
  if (value.type !== 'json_schema') return undefined;
  if (toResponses) {
    if (!object(value.json_schema) || Object.keys(value).some(key => key !== 'type' && key !== 'json_schema') || owns(value.json_schema, 'type')) return undefined;
    return { type: 'json_schema', ...value.json_schema };
  }
  const { type: _type, ...schema } = value;
  return { type: 'json_schema', json_schema: schema };
}

/** Same behavior as the shared Rust transition fixture; never drops semantic conflicts. */
export function transitionChatProtocolParams(input: Record<string, unknown>, source: string, target: string): ChatProtocolTransitionResult {
  source = source.trim(); target = target.trim();
  const params = structuredClone(input);
  const suspended: Record<string, unknown> = {};
  const issues: ChatProtocolTransitionIssue[] = [];
  if (!isOpenAIChatProtocol(source) || !isOpenAIChatProtocol(target)) return { params, suspended, issues };
  const issue = (parameter: string, kind: ChatProtocolTransitionIssue['kind']) => issues.push({ parameter, kind });
  const suspend = (key: string) => { if (owns(params, key)) { suspended[key] = params[key]; delete params[key]; } };
  if (owns(params, 'reasoning') && empty(params.reasoning)) suspend('reasoning');
  if (object(params.reasoning) && owns(params.reasoning, 'effort')) {
    if (owns(params, 'reasoning_effort') && !equal(params.reasoning_effort, params.reasoning.effort)) issue('reasoning.effort', 'conflict');
    else {
      params.reasoning_effort = params.reasoning.effort;
      delete params.reasoning.effort;
      if (Object.keys(params.reasoning).length === 0) delete params.reasoning;
    }
  }
  if (source === target) return { params, suspended, issues };
  if (target === 'openai.responses') {
    for (const key of ['stream_options', 'require_reasoning_content']) suspend(key);
    if (typeof params.max_tokens_field === 'string' && ['max_tokens', 'max_completion_tokens'].includes(params.max_tokens_field.trim())) suspend('max_tokens_field');
    if (owns(params, 'text') && empty(params.text)) suspend('text');
    const setText = (key: string, value: unknown, sourceKey: string): boolean => {
      if (owns(params, 'text') && !object(params.text)) { issue(sourceKey, 'unsupported'); return false; }
      const text = object(params.text) ? params.text : {};
      if (owns(text, key) && !equal(text[key], value)) { issue(sourceKey, 'conflict'); return false; }
      text[key] = value; params.text = text; return true;
    };
    if (owns(params, 'response_format')) {
      if (empty(params.response_format)) suspend('response_format');
      else {
        const format = convertFormat(params.response_format, true);
        if (!format) issue('response_format', 'unsupported');
        else if (setText('format', format, 'response_format')) delete params.response_format;
      }
    }
    if (owns(params, 'verbosity') && setText('verbosity', params.verbosity, 'verbosity')) delete params.verbosity;
  } else {
    for (const key of ['include', 'chain_rounds']) suspend(key);
    if (typeof params.max_tokens_field === 'string' && params.max_tokens_field.trim() === 'max_output_tokens') suspend('max_tokens_field');
    if (owns(params, 'text')) {
      if (empty(params.text)) suspend('text');
      else if (!object(params.text)) issue('text', 'unsupported');
      else {
        const text = params.text;
        if (owns(text, 'format')) {
          if (empty(text.format)) { suspended.text = { format: text.format }; delete text.format; }
          else {
            const format = convertFormat(text.format, false);
            if (!format) issue('text.format', 'unsupported');
            else if (owns(params, 'response_format') && !equal(params.response_format, format)) issue('text.format', 'conflict');
            else { params.response_format = format; delete text.format; }
          }
        }
        if (owns(text, 'verbosity')) {
          if (owns(params, 'verbosity') && !equal(params.verbosity, text.verbosity)) issue('text.verbosity', 'conflict');
          else { params.verbosity = text.verbosity; delete text.verbosity; }
        }
        if (Object.keys(text).length === 0) delete params.text;
        else if (Object.keys(text).some(key => key !== 'format' && key !== 'verbosity')) issue('text', 'unsupported');
      }
    }
    if (owns(params, 'reasoning')) {
      if (empty(params.reasoning)) suspend('reasoning');
      else if (!object(params.reasoning) || Object.keys(params.reasoning).some(key => key !== 'effort')) issue('reasoning', 'unsupported');
    }
  }
  return { params, suspended, issues };
}
