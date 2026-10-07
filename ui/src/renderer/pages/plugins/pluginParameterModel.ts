export type PluginParameterValue = string | number | boolean;
export type PluginParameterField = {
  key: string;
  label: string;
  description?: string;
  type: 'string' | 'number' | 'integer' | 'boolean';
  required: boolean;
  options?: PluginParameterValue[];
  defaultValue?: PluginParameterValue;
};

const object = (value: unknown): Record<string, unknown> | null =>
  value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null;
const primitive = (value: unknown): value is PluginParameterValue =>
  typeof value === 'string' || typeof value === 'boolean' || typeof value === 'number' && Number.isFinite(value);
const ROOT_KEYS = new Set(['$schema', '$id', 'type', 'title', 'description', 'properties', 'required', 'additionalProperties', 'default']);
const FIELD_KEYS = new Set(['type', 'title', 'description', 'enum', 'default']);
const matches = (field: PluginParameterField, value: unknown) =>
  primitive(value) && (field.type === 'integer' ? typeof value === 'number' && Number.isInteger(value) : typeof value === field.type)
  && (!field.options || field.options.includes(value));

/** Only use fields when every declared parameter can be represented without
 * dropping a constraint. Everything else keeps its original JSON schema. */
export function pluginParameterFields(schema: unknown): PluginParameterField[] | null {
  const root = object(schema);
  if (!root || root.type !== 'object' || Object.keys(root).some(key => !ROOT_KEYS.has(key))) return null;
  if (root.additionalProperties !== undefined && typeof root.additionalProperties !== 'boolean') return null;
  const properties = object(root.properties);
  if (!properties) return null;
  if (!Object.keys(properties).length && root.additionalProperties !== false) return null;
  const required = root.required ?? [];
  if (!Array.isArray(required) || required.some(key => typeof key !== 'string' || !Object.hasOwn(properties, key))) return null;
  const fields: PluginParameterField[] = [];
  for (const [key, source] of Object.entries(properties)) {
    const property = object(source);
    if (!property || Object.keys(property).some(key => !FIELD_KEYS.has(key))) return null;
    if (property.title !== undefined && typeof property.title !== 'string'
      || property.description !== undefined && typeof property.description !== 'string') return null;
    let type = property.type;
    let options: PluginParameterValue[] | undefined;
    if (property.enum !== undefined) {
      if (!Array.isArray(property.enum) || !property.enum.length || !property.enum.every(primitive)) return null;
      options = property.enum;
      if (type === undefined && options.every(value => typeof value === typeof options![0])) type = typeof options[0];
    }
    if (type !== 'string' && type !== 'number' && type !== 'integer' && type !== 'boolean') return null;
    const field: PluginParameterField = { key, label: typeof property.title === 'string' ? property.title : key,
      ...(typeof property.description === 'string' ? { description: property.description } : {}), type, required: required.includes(key), ...(options ? { options } : {}) };
    if (options?.some(value => !matches({ ...field, options: undefined }, value))) return null;
    if (Object.hasOwn(property, 'default')) {
      if (!matches(field, property.default)) return null;
      field.defaultValue = property.default as PluginParameterValue;
    }
    fields.push(field);
  }
  if (Object.hasOwn(root, 'default')) {
    const defaults = object(root.default);
    if (!defaults || Object.entries(defaults).some(([key, value]) => {
      const field = fields.find(field => field.key === key);
      return field ? !matches(field, value) : root.additionalProperties === false || !primitive(value);
    })) return null;
  }
  return fields;
}

export function pluginParameterDefaults(schema: unknown): Record<string, PluginParameterValue> {
  const fields = pluginParameterFields(schema);
  if (!fields) return {};
  const defaults = object(object(schema)?.default) ?? {};
  return { ...Object.fromEntries(fields.filter(field => field.defaultValue !== undefined).map(field => [field.key, field.defaultValue!])),
    ...defaults as Record<string, PluginParameterValue> };
}

export function parsePluginParameterObject(value: string): Record<string, unknown> | null {
  try { return object(JSON.parse(value)); } catch { return null; }
}

/** Preserve all other values, including optional data supplied through JSON. */
export function setPluginParameterValue(value: string, field: PluginParameterField, next: PluginParameterValue | undefined): string {
  const current = parsePluginParameterObject(value);
  if (!current) return value;
  const updated = { ...current };
  if (next === undefined || !field.required && next === '') delete updated[field.key];
  else Object.defineProperty(updated, field.key, { value: next, enumerable: true, configurable: true, writable: true });
  return JSON.stringify(updated, null, 2);
}

export function pluginParameterValuesRepresentable(fields: PluginParameterField[], value: Record<string, unknown>): boolean {
  return fields.every(field => !Object.hasOwn(value, field.key) || matches(field, value[field.key]));
}
