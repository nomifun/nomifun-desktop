import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { derivePresentation, deriveSeed, generatedBlocks, generatedDomain, verifyInputs } from './generate-contracts.mjs';

const directory = new URL('../../crates/backend/nomifun-agent-contracts/contracts/', import.meta.url);
const inventory = JSON.parse(await readFile(new URL('computer-tools.v1.json', directory), 'utf8'));
const overrides = JSON.parse(await readFile(new URL('computer-tools.overrides.v1.json', directory), 'utf8'));

test('generator rejects an inventory produced with a different compact profile', () => {
  verifyInputs(inventory, overrides);
  const changed = structuredClone(overrides);
  changed.tools[0].descriptions['en-US'] += ' changed';
  assert.throws(() => verifyInputs(inventory, changed), /profile drift/);
});

test('every frozen native action has a distinct exact typed operation and host projection', () => {
  const blocks = generatedBlocks(inventory);
  const domain = generatedDomain(inventory);
  assert.equal(blocks.variants.split('\n').length, 22);
  for (const tool of inventory.tools) {
    assert.ok(blocks.actions.includes(JSON.stringify(tool.action_id)));
    assert.ok(domain.includes(`Some((${JSON.stringify(tool.action_id)}, input))`));
    assert.ok(domain.includes(`EffectClass::${tool.effect_class}`));
    assert.ok(domain.includes(`Some(${JSON.stringify(tool.resource_operation)})`));
  }
});

test('presentation regeneration preserves other domains and derives exact Computer names', () => {
  const other = { capability_id: 'workspace.files', action_id: 'workspace.files/read', name: 'read_file', titles: { 'zh-CN': '读文件', 'en-US': 'Read file' }, target_fields: ['path'] };
  const result = derivePresentation([other], inventory);
  assert.deepEqual(result[0], other);
  assert.deepEqual(result.slice(1).map(tool => tool.name), inventory.tools.map(tool => tool.model_tool));
  assert.deepEqual(result.slice(1).map(tool => tool.titles), inventory.tools.map(tool => tool.titles));
});

test('official default grants derive all 22 actions without changing other templates or capabilities', () => {
  const selection = { capability: { id: 'computer' }, action_allowlist: [] };
  const other = { capability: { id: 'workspace.files' }, action_allowlist: ['workspace.files/read'] };
  const original = { templates: {
    'assistant.general': { enabled_capabilities: [selection, other] },
    'companion.default': { enabled_capabilities: [selection, other] },
    'coding.codex': { enabled_capabilities: [other] },
  } };
  const result = deriveSeed(original, inventory);
  assert.deepEqual(result.templates['coding.codex'], original.templates['coding.codex']);
  assert.deepEqual(result.templates['assistant.general'].enabled_capabilities[1], other);
  for (const key of ['assistant.general', 'companion.default']) assert.deepEqual(result.templates[key].enabled_capabilities[0].action_allowlist, inventory.tools.map(tool => tool.action_id).sort());
  assert.deepEqual(original.templates['assistant.general'].enabled_capabilities[0].action_allowlist, []);
});
