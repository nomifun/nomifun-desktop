import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const schemas = JSON.parse(fs.readFileSync(path.join(root, 'crates/backend/nomifun-voice-contracts/contracts/schemas.json'), 'utf8'));
const names = Object.keys(schemas);
const definitions = new Map();
for (const name of names) {
  const schema = schemas[name];
  if (!schema?.title) throw new Error(`Missing generated voice schema ${name}; run voice-contract write first`);
  definitions.set(schema.title, schema);
  for (const [key, value] of Object.entries(schema.$defs ?? {})) {
    const existing = definitions.get(key);
    if (existing && typeOf(existing) !== typeOf(value)) throw new Error(`Conflicting schema ${key}`);
    definitions.set(key, value);
  }
}
function typeOf(schema) {
  if (schema === true) return 'unknown';
  if (schema === false) return 'never';
  if (schema.$ref) return schema.$ref.split('/').at(-1).replaceAll('~1', '/').replaceAll('~0', '~');
  if ('const' in schema) return JSON.stringify(schema.const);
  if (schema.enum) return schema.enum.map(value => JSON.stringify(value)).join(' | ');
  if (schema.anyOf || schema.oneOf) return (schema.anyOf ?? schema.oneOf).map(typeOf).join(' | ');
  if (schema.allOf) return schema.allOf.map(typeOf).join(' & ');
  if (Array.isArray(schema.type)) return schema.type.map(type => typeOf({ ...schema, type })).join(' | ');
  switch (schema.type) {
    case 'null': return 'null';
    case 'boolean': return 'boolean';
    case 'integer': case 'number': return 'number';
    case 'string': return 'string';
    case 'array': return `Array<${typeOf(schema.items ?? true)}>`;
    case 'object': {
      const required = new Set(schema.required ?? []);
      const properties = Object.entries(schema.properties ?? {}).sort(([a], [b]) => a.localeCompare(b));
      const fields = properties.map(([key, value]) => `  ${JSON.stringify(key)}${required.has(key) ? '' : '?'}: ${typeOf(value)};`);
      if (schema.additionalProperties && schema.additionalProperties !== false) fields.push(`  [key: string]: ${typeOf(schema.additionalProperties)};`);
      return fields.length ? `{\n${fields.join('\n')}\n}` : 'Record<string, unknown>';
    }
    default: return 'unknown';
  }
}
const output = '// Generated from independent optional voice JSON schemas by scripts/check-voice-contracts.mjs.\n// Do not edit; run bun scripts/check-voice-contracts.mjs --write after contract generation.\n\n' +
  [...definitions].sort(([a], [b]) => a.localeCompare(b)).map(([name, schema]) => `export type ${name} = ${typeOf(schema)};\n`).join('\n');
const target = path.resolve(root, '../nomifun-mobile/src/features/voice/contracts.generated.ts');
if (process.argv.includes('--write')) fs.mkdirSync(path.dirname(target), { recursive: true });
if (process.argv.includes('--write')) fs.writeFileSync(target, output);
else if (!fs.existsSync(target) || fs.readFileSync(target, 'utf8') !== output) {
  throw new Error('Voice TypeScript contracts are stale; run bun scripts/check-voice-contracts.mjs --write');
}
console.log('Canonical voice TypeScript contracts are current.');
