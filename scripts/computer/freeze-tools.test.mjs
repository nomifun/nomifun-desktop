import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import {
  captureWorkerInventory, compactSchema, digest, freezeInventory,
  isolatedEnvironment, overridesPath, validateOverrides,
} from './freeze-tools.mjs';

const overrides = JSON.parse(await readFile(overridesPath, 'utf8'));
const metadata = {
  driver_version: '0.37.0', contract_version: '0.8.0', tools_list_schema_version: '1',
  capability_version: '1', mcp_protocol_version: '2025-06-18', pid: 1234,
};

function inventory() {
  return {
    schema_version: '1', capability_version: '1',
    tools: overrides.tools.map(tool => ({
      name: tool.driver_tool, description: `New driver definition for ${tool.driver_tool}`,
      annotations: { readOnlyHint: tool.resource_operation === 'observe' }, capabilities: [],
      inputSchema: tool.driver_tool === 'run_steps'
        ? { type: 'object', properties: { steps: { type: 'array', items: { type: 'object', properties: { tool: { type: 'string', enum: ['click', 'double_click', 'right_click', 'type_text', 'scroll'] } } } } } }
        : { type: 'object', properties: { session: { type: 'string' }, include_screenshot: { type: 'boolean' } }, additionalProperties: false },
    })),
  };
}

function freeze(list = inventory(), currentPlatform = 'windows', previous = null, profile = overrides) {
  return freezeInventory({ list, metadata, currentPlatform, previous, overrides: profile });
}

test('compact profile declares the 22 approved actions with complete localized metadata', () => {
  validateOverrides(overrides);
  assert.equal(overrides.tools.length, 22);
  assert.equal(overrides.tools.filter(tool => tool.loading === 'eager').length, 11);
  assert.equal(overrides.tools.filter(tool => tool.effect_class === 'ReadSensitive').length, 7);
  assert.equal(overrides.tools.find(tool => tool.driver_tool === 'verify_state').no_image, 'window_without_image');
  assert.equal(overrides.identity.effect_resource_key, null);
  assert.deepEqual(overrides.identity.resource_kinds, ['computer']);
  assert.equal(overrides.identity.default_resource_id, 'local-desktop');
  const duplicate = structuredClone(overrides);
  duplicate.tools[1].driver_tool = duplicate.tools[0].driver_tool;
  assert.throws(() => validateOverrides(duplicate), /duplicate/);
});

test('schema compaction removes trusted fields recursively while preserving structural constraints', () => {
  const original = {
    type: 'object', additionalProperties: false,
    properties: {
      target: { type: 'object', required: ['kind', 'session'], properties: { kind: { type: 'string', enum: ['window', 'desktop'] }, pid: { type: 'integer' }, session: { type: 'string' } } },
      scope: { type: 'string' }, pid: { type: 'integer' }, window_id: { type: 'integer' },
      session: { type: 'string' }, max_image_dimension: { type: 'integer' }, max_dimension: { type: 'integer' },
      screenshot_out_file: { type: 'string' }, capture_mode: { type: 'string' }, _hidden: { type: 'string' },
      count: { type: 'integer', minimum: 1, maximum: 3 },
      nested: { type: 'array', items: { type: 'object', properties: { _private: { type: 'string' }, text: { type: 'string' } } } },
    },
    required: ['pid', 'window_id', 'session', 'count'],
  };
  const before = structuredClone(original);
  const schema = compactSchema(original, overrides, overrides.tools.find(tool => tool.driver_tool === 'click'));
  assert.deepEqual(original, before);
  assert.deepEqual(Object.keys(schema.properties), ['count', 'nested', 'target']);
  assert.deepEqual(schema.required, ['count']);
  assert.deepEqual(schema.properties.target.required, ['kind']);
  assert.ok(schema.properties.target.properties.pid);
  assert.deepEqual(schema.properties.nested.items.properties, { text: { type: 'string' } });
  assert.deepEqual(schema.properties.count, { maximum: 3, minimum: 1, type: 'integer' });
  assert.equal(schema.additionalProperties, false);
  const steps = compactSchema({ type: 'object', properties: {
    steps: { type: 'integer', minimum: 1, maximum: 200, description: 'Number of intermediate WM_MOUSEMOVE events. Default: 20.' },
    nested: { type: 'object', properties: { steps: { type: 'array', items: { type: 'object' }, description: 'Upstream batch steps.' } } },
  } }, overrides, overrides.tools.find(tool => tool.driver_tool === 'drag'));
  assert.equal(steps.properties.steps.description, 'Number of intermediate WM_MOUSEMOVE events. Default: 20.');
  assert.equal(steps.properties.steps.minimum, 1);
  assert.equal(steps.properties.steps.maximum, 200);
  assert.equal(steps.properties.nested.properties.steps.description, overrides.parameter_descriptions.steps);
});

test('flat window identity remains present when a tool has no target object', () => {
  const schema = compactSchema({ type: 'object', properties: { pid: { type: 'integer' }, window_id: { type: 'integer' }, scope: { type: 'string' } }, required: ['pid', 'window_id'] }, overrides, overrides.tools[0]);
  assert.deepEqual(schema.required, ['pid', 'window_id']);
  assert.deepEqual(Object.keys(schema.properties), ['pid', 'scope', 'window_id']);
});

test('batch tool vocabulary derives from the driver enum intersected with exposed actions', () => {
  const manifest = freeze();
  const batch = manifest.tools.find(tool => tool.driver_tool === 'run_steps');
  assert.deepEqual(batch.platforms.windows.input_schema.properties.steps.items.properties.tool.enum,
    ['computer_click', 'computer_type_text', 'computer_scroll']);
  const list = inventory();
  delete list.tools.find(tool => tool.name === 'run_steps').inputSchema.properties.steps.items.properties.tool.enum;
  assert.throws(() => freeze(list), /batchable tool enum/);
});

test('freezing rejects missing actions, duplicate driver definitions and unsupported list versions', () => {
  const missing = inventory();
  missing.tools.pop();
  assert.throws(() => freeze(missing), /clipboard_write/);
  const duplicate = inventory();
  duplicate.tools.push(duplicate.tools[0]);
  assert.throws(() => freeze(duplicate), /duplicate tool/);
  assert.throws(() => freeze({ ...inventory(), schema_version: '2' }), /list schema/);
});

test('platform captures retain actual platform differences without inventing unexecuted platforms', () => {
  const windows = freeze();
  const linuxList = inventory();
  linuxList.tools[0].inputSchema.properties.linux_specific = { type: 'boolean' };
  const combined = freeze(linuxList, 'linux', windows);
  assert.deepEqual(Object.keys(combined.captures), ['linux', 'windows']);
  assert.deepEqual(combined.tools[0].platforms.windows, windows.tools[0].platforms.windows);
  assert.ok(combined.tools[0].platforms.linux.input_schema.properties.linux_specific);
  assert.equal(combined.tools[0].platforms.macos, undefined);
  assert.notEqual(combined.captures.windows.list_sha256, combined.captures.linux.list_sha256);
});

test('ephemeral process metadata cannot cause frozen inventory drift', () => {
  const list = inventory();
  const first = freeze(list);
  const second = freezeInventory({ list, metadata: { ...metadata, pid: 9876, host_bundle_id: 'different-test-host' }, currentPlatform: 'windows', overrides });
  assert.equal(digest(first), digest(second));
  assert.equal(first.captures.windows.metadata.pid, undefined);
});

test('profile changes cannot silently reuse other platforms from an older profile', () => {
  const previous = freeze();
  const changed = structuredClone(overrides);
  changed.tools[0].titles['en-US'] = 'Updated app title';
  assert.throws(() => freeze(inventory(), 'linux', previous, changed), /recapture all platforms/);
});

test('discovery environment isolates all writable homes and omits unrelated credentials or desktop routing', () => {
  const root = path.resolve('isolated-discovery-root');
  const env = isolatedEnvironment(root);
  for (const key of ['HOME', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'TEMP', 'TMP', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME', 'XDG_CACHE_HOME', 'CUA_DRIVER_RS_HOME', 'CUA_DRIVER_CURSOR_THEME_DIR']) {
    const relative = path.relative(root, env[key]);
    assert.ok(!relative.startsWith('..') && !path.isAbsolute(relative), `${key} must remain isolated`);
  }
  assert.equal(env.NO_AT_BRIDGE, '1');
  assert.equal(env.OPENAI_API_KEY, undefined);
  assert.equal(env.DISPLAY, undefined);
  assert.equal(env.CUA_DRIVER_RS_ENABLE_WAYLAND, undefined);
  const linux = isolatedEnvironment(root, 'linux');
  assert.equal(linux.XDG_RUNTIME_DIR, root);
  assert.equal(linux.DBUS_SESSION_BUS_ADDRESS, `unix:path=${path.join(root, 'no-session-bus')}`);
  assert.notEqual(linux.DBUS_SESSION_BUS_ADDRESS, process.env.DBUS_SESSION_BUS_ADDRESS);
});

const fakeWorker = `
  const readline = require('node:readline');
  const seen = [];
  readline.createInterface({input:process.stdin}).on('line', line => {
    const r = JSON.parse(line); seen.push(r.operation);
    if (!['initialize','metadata','list','shutdown'].includes(r.operation)) process.exit(2);
    if (r.operation === 'initialize' && (r.arguments.configured_driver.authorization.unrestricted_acknowledged || r.arguments.configured_driver.authorization.compatibility_mode !== 'standard')) process.exit(3);
    const result = r.operation === 'metadata' ? {test:true} : r.operation === 'list' ? {seen, isolated: process.env.HOME === process.env.USERPROFILE, hasDesktop: Boolean(process.env.DISPLAY)} : {};
    process.stdout.write(JSON.stringify({protocol_version:1,request_id:r.request_id,generation:r.generation,ok:true,completion:'completed',result})+'\\n');
    if(r.operation === 'shutdown') process.exit(0);
  });
`;

test('metadata capture uses only initialize/metadata/list/shutdown with standard authorization', async () => {
  const result = await captureWorkerInventory(process.execPath, { prefixArguments: ['-e', fakeWorker], timeoutMs: 5000 });
  assert.deepEqual(result.metadata, { test: true });
  assert.deepEqual(result.list.seen, ['initialize', 'metadata', 'list']);
  assert.equal(result.list.isolated, true);
  assert.equal(result.list.hasDesktop, false);
});

test('metadata capture fails closed on a partial response followed by worker exit', async () => {
  await assert.rejects(captureWorkerInventory(process.execPath, {
    prefixArguments: ['-e', "process.stdin.once('data',()=>{process.stdout.write('{');process.exit(0)})"], timeoutMs: 5000,
  }), /exited before completing/);
});

test('metadata capture enforces a deadline for an unresponsive worker', async () => {
  await assert.rejects(captureWorkerInventory(process.execPath, {
    prefixArguments: ['-e', 'setInterval(()=>{},1000)'], timeoutMs: 100,
  }), /discovery exceeded/);
});
