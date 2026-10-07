#!/usr/bin/env node
/** Opt-in diagnostic, not a task-quality evaluation. At most two provider
 * generations; no returned tool is executed. Only fixed shape categories are
 * printed. No credential, prompt, response text or argument value is logged.
 */
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const KEY_NAME = 'NOMIFUN_LIVE_STEPFUN_API_KEY';
const rawKey = process.env[KEY_NAME];
delete process.env[KEY_NAME];
const endpoint = 'https://api.stepfun.com/step_plan/v1/chat/completions';
const task = {
  type: 'object', additionalProperties: false,
  properties: { name: { type: 'string' }, prompt: { type: 'string' } },
  required: ['name', 'prompt'],
};
const parallel = {
  type: 'object', additionalProperties: false,
  properties: {
    strategy: { type: 'string', const: 'parallel' },
    tasks: { type: 'array', minItems: 1, maxItems: 16, items: task },
    synthesize: { type: 'boolean' },
  },
  required: ['strategy', 'tasks', 'synthesize'],
};
const cases = [
  ['flat', parallel],
  ['union', { type: 'object', oneOf: [
    { type: 'object', additionalProperties: false,
      properties: { strategy: { type: 'string', const: 'planned' }, goal: { type: 'string' } },
      required: ['strategy', 'goal'],
    }, parallel,
  ] }],
];
const kind = value => value === undefined ? 'missing' : value === null ? 'null'
  : Array.isArray(value) ? 'array' : typeof value;

// This diagnostic retains the raw argument JSON exactly; no schema repair or
// coercion is performed, so it can distinguish provider output from the app.
export function decodeStream(text) {
  if (text.length > 2 * 1024 * 1024) throw new Error('response budget exceeded');
  const calls = new Map();
  let finish;
  for (const frame of text.replaceAll('\r\n', '\n').split('\n\n')) {
    const data = frame.split('\n').filter(line => line.startsWith('data:'))
      .map(line => line.slice(5).trimStart()).join('\n');
    if (!data || data.trim() === '[DONE]') continue;
    const event = JSON.parse(data);
    const choice = event.choices?.[0];
    if (choice?.finish_reason) finish = choice.finish_reason;
    for (const delta of choice?.delta?.tool_calls ?? []) {
      const index = delta.index ?? 0;
      const call = calls.get(index) ?? { type: 'function', function: { name: '', arguments: '' } };
      if (delta.function?.name && call.function.name !== delta.function.name) call.function.name += delta.function.name;
      if (delta.function?.arguments !== undefined) {
        if (typeof delta.function.arguments !== 'string') throw new Error('unexpected raw argument type');
        call.function.arguments += delta.function.arguments;
      }
      calls.set(index, call);
    }
  }
  return { choices: [{ finish_reason: finish, message: { tool_calls: [...calls.values()] } }] };
}

async function main() {
  if (process.argv.length !== 2 || typeof rawKey !== 'string' || !rawKey.trim() || /[\r\n]/.test(rawKey)) {
    console.error('tool_schema_probe=not_run reason=invalid_input');
    process.exitCode = 2;
    return;
  }
  for (const [name, schema] of cases) {
    try {
      const response = await fetch(endpoint, {
        method: 'POST', signal: AbortSignal.timeout(45_000),
        headers: { 'Content-Type': 'application/json', Accept: 'text/event-stream', Authorization: `Bearer ${rawKey.trim()}` },
        body: JSON.stringify({
          model: 'step-3.7-flash', temperature: 0, max_tokens: 4096, stream: true,
          messages: [{ role: 'user', content: 'Call probe_tool exactly once with strategy="parallel", tasks=[{"name":"marker","prompt":"Reply OK"}], synthesize=false. Use a native tool call, no prose. The client will not execute this diagnostic proposal.' }],
          tools: [{ type: 'function', function: { name: 'probe_tool', description: 'Return a structured diagnostic proposal; it will not be executed.', parameters: schema } }],
          tool_choice: 'auto',
        }),
      });
      if (!response.ok) {
        let code = 'other';
        try {
          const error = (await response.json())?.error;
          const known = ['invalid_api_key', 'permission_error', 'usage_not_included', 'insufficient_quota', 'quota_exceeded', 'invalid_request_error', 'unsupported_parameter'];
          code = [error?.code, error?.type].find(value => known.includes(value)) ?? 'other';
        } catch { /* Do not log untrusted error diagnostics. */ }
        console.log(JSON.stringify({ probe: name, status: response.status, result: 'http_error', code }));
        process.exitCode = 1;
        return;
      }
      const data = decodeStream(await response.text());
      const calls = data?.choices?.[0]?.message?.tool_calls;
      const call = calls?.[0];
      let args = {};
      let parsed = false;
      try {
        args = typeof call?.function?.arguments === 'string'
          ? JSON.parse(call.function.arguments) : call?.function?.arguments;
        parsed = args !== null && typeof args === 'object' && !Array.isArray(args);
      } catch { /* Do not echo any provider argument or parser error. */ }
      if (!parsed) args = {};
      const exact = parsed && Array.isArray(calls) && calls.length === 1
        && call.function?.name === 'probe_tool' && data?.choices?.[0]?.finish_reason === 'tool_calls'
        && args.strategy === 'parallel' && args.synthesize === false
        && Array.isArray(args.tasks) && args.tasks.length === 1
        && args.tasks[0]?.name === 'marker' && args.tasks[0]?.prompt === 'Reply OK'
        && Object.keys(args.tasks[0]).every(key => ['name', 'prompt'].includes(key))
        && Object.keys(args).every(key => ['strategy', 'tasks', 'synthesize'].includes(key));
      console.log(JSON.stringify({
        probe: name, status: response.status, native_call: Boolean(call), parsed,
        strategy: args.strategy === 'parallel' ? 'parallel' : 'other',
        tasks_type: kind(args.tasks), synthesize_type: kind(args.synthesize), exact,
      }));
      if (!exact) process.exitCode = 1;
    } catch {
      console.log(JSON.stringify({ probe: name, result: 'request_failed' }));
      process.exitCode = 1;
      return;
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) await main();
