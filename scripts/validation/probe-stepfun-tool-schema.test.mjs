import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeStream } from './probe-stepfun-tool-schema.mjs';

const frame = value => `data: ${JSON.stringify(value)}\r\n\r\n`;
function stream(argumentsJson) {
  const split = Math.floor(argumentsJson.length / 2);
  return frame({ choices: [{ delta: { tool_calls: [{ index: 0, function: { name: 'probe_tool', arguments: argumentsJson.slice(0, split) } }] } }] })
    + frame({ choices: [{ delta: { tool_calls: [{ index: 0, function: { arguments: argumentsJson.slice(split) } }] }, finish_reason: 'tool_calls' }] })
    + 'data: [DONE]\r\n\r\n';
}

test('raw streaming probe preserves native arrays and booleans across chunks', () => {
  const args = { strategy: 'parallel', tasks: [{ name: 'marker', prompt: 'Reply OK' }], synthesize: false };
  const decoded = decodeStream(stream(JSON.stringify(args)));
  assert.equal(decoded.choices[0].finish_reason, 'tool_calls');
  assert.equal(decoded.choices[0].message.tool_calls[0].function.name, 'probe_tool');
  assert.deepEqual(JSON.parse(decoded.choices[0].message.tool_calls[0].function.arguments), args);
});

test('diagnostic does not repair stringified provider arguments', () => {
  const args = { strategy: 'parallel', tasks: '[]', synthesize: 'false' };
  const decoded = decodeStream(stream(JSON.stringify(args)));
  assert.deepEqual(JSON.parse(decoded.choices[0].message.tool_calls[0].function.arguments), args);
});
