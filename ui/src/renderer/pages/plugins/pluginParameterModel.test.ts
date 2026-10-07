import { expect, test } from 'bun:test';
import { pluginParameterDefaults, pluginParameterFields, setPluginParameterValue } from './pluginParameterModel';

const schema = { type: 'object', properties: {
  title: { type: 'string', title: 'Title' }, count: { type: 'integer', default: 3 },
  ratio: { type: 'number', default: .25 }, enabled: { type: 'boolean', default: false },
  mode: { type: 'integer', enum: [1, 2], default: 2 },
}, required: ['count'] };

test('form defaults and edits preserve JSON types and omit untouched optional fields', () => {
  expect(pluginParameterDefaults(schema)).toEqual({ count: 3, ratio: .25, enabled: false, mode: 2 });
  const fields = pluginParameterFields(schema)!;
  let value = JSON.stringify({ count: 3, title: 'Temporary', extra: { kept: true } });
  value = setPluginParameterValue(value, fields.find(field => field.key === 'title')!, '');
  value = setPluginParameterValue(value, fields.find(field => field.key === 'ratio')!, 0);
  value = setPluginParameterValue(value, fields.find(field => field.key === 'enabled')!, false);
  expect(JSON.parse(value)).toEqual({ count: 3, ratio: 0, enabled: false, extra: { kept: true } });
  value = setPluginParameterValue(value, fields.find(field => field.key === 'enabled')!, undefined);
  expect(JSON.parse(value)).toEqual({ count: 3, ratio: 0, extra: { kept: true } });
});

test('nested, conditional, referenced, and constrained inputs retain the complete JSON editor', () => {
  for (const unsupported of [
    { ...schema, oneOf: [{ required: ['title'] }, { required: ['count'] }] },
    { ...schema, properties: { account: { type: 'object', properties: { token: { type: 'string' } }, required: ['token'] } }, required: ['account'] },
    { ...schema, properties: { account: { $ref: '#/$defs/account' } }, required: ['account'] },
    { ...schema, properties: { title: { type: 'string', minLength: 3 } }, required: [] },
    { ...schema, additionalProperties: { type: 'integer' } },
    { ...schema, required: ['missing-property'] },
    { type: 'object' },
  ]) expect(pluginParameterFields(unsupported)).toBeNull();
  expect(pluginParameterFields({ type: 'object', properties: {}, additionalProperties: false })).toEqual([]);
});

test('root defaults and homogeneous enums remain typed without converting numeric options to strings', () => {
  const supported = { type: 'object', properties: { mode: { enum: [1, 2], title: 'Mode' } }, default: { mode: 2 } };
  expect(pluginParameterDefaults(supported)).toEqual({ mode: 2 });
  expect(pluginParameterFields(supported)?.[0].options).toEqual([1, 2]);
  expect(pluginParameterFields({ ...supported, properties: { mode: { enum: [1, '2'] } } })).toBeNull();
});
