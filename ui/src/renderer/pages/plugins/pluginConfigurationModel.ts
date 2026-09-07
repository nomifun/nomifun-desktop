/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  ConfigurePluginRequest,
  PluginConfigValues,
  PluginCredentialSlotBinding,
  PluginDetail,
} from '@/common/types/pluginPlatform';

export type PluginConfigScalar = string | number | boolean;
export type PluginConfigFieldKind =
  | 'string'
  | 'number'
  | 'integer'
  | 'boolean'
  | 'enum';

export type PluginConfigSchemaIssueCode =
  | 'root_not_object'
  | 'root_type_unsupported'
  | 'properties_not_object'
  | 'required_invalid'
  | 'required_property_missing'
  | 'dynamic_properties_unsupported'
  | 'unsupported_keyword'
  | 'field_not_object'
  | 'field_type_unsupported'
  | 'field_enum_invalid'
  | 'field_constraint_invalid'
  | 'field_read_only'
  | 'secret_config_unsupported'
  | 'schema_digest_mismatch'
  | 'duplicate_credential_slot'
  | 'mount_target_missing';

export interface PluginConfigSchemaIssue {
  code: PluginConfigSchemaIssueCode;
  path: string;
  keyword?: string;
}

export interface PluginConfigEnumOption {
  id: string;
  label: string;
  value: PluginConfigScalar;
}

export interface PluginConfigFieldModel {
  key: string;
  label: string;
  description?: string;
  required: boolean;
  kind: PluginConfigFieldKind;
  enumOptions: PluginConfigEnumOption[];
  defaultValue?: PluginConfigScalar;
  initialValue?: PluginConfigScalar;
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
}

export interface PluginConfigurationEditorModel {
  title?: string;
  description?: string;
  fields: PluginConfigFieldModel[];
  schemaIssues: PluginConfigSchemaIssue[];
  canSubmit: boolean;
}

export type PluginCredentialDraftAction = 'keep' | 'bind' | 'unbind';

export interface PluginCredentialDraft {
  action: PluginCredentialDraftAction;
  credentialId: string;
}

export interface PluginConfigurationDraft {
  values: Record<string, PluginConfigScalar | undefined>;
  credentials: Record<string, PluginCredentialDraft>;
}

export type PluginConfigurationDraftIssueCode =
  | 'required'
  | 'invalid_type'
  | 'integer_required'
  | 'minimum'
  | 'maximum'
  | 'min_length'
  | 'max_length'
  | 'enum_value'
  | 'credential_required'
  | 'credential_id_required';

export interface PluginConfigurationDraftIssue {
  code: PluginConfigurationDraftIssueCode;
  scope: 'config' | 'credential';
  key: string;
  limit?: number;
}

const ROOT_KEYWORDS = new Set([
  '$id',
  '$schema',
  'additionalProperties',
  'description',
  'properties',
  'required',
  'title',
  'type',
]);

const FIELD_KEYWORDS = new Set([
  '$id',
  'default',
  'description',
  'enum',
  'format',
  'maxLength',
  'maximum',
  'minLength',
  'minimum',
  'readOnly',
  'secret',
  'title',
  'type',
  'writeOnly',
  'x-nomifun-secret',
  'x-secret',
]);

const isRecord = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);

const hasOwn = (value: Record<string, unknown>, key: string): boolean =>
  Object.prototype.hasOwnProperty.call(value, key);

const text = (value: unknown): string | undefined =>
  typeof value === 'string' && value.trim() ? value.trim() : undefined;

const finiteNumber = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isFinite(value) ? value : undefined;

const nonNegativeInteger = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isInteger(value) && value >= 0
    ? value
    : undefined;

const scalarType = (
  value: unknown
): Exclude<PluginConfigFieldKind, 'enum'> | undefined => {
  if (typeof value === 'string') return 'string';
  if (typeof value === 'boolean') return 'boolean';
  if (typeof value === 'number' && Number.isFinite(value)) {
    return Number.isInteger(value) ? 'integer' : 'number';
  }
  return undefined;
};

const matchesDeclaredType = (
  value: unknown,
  type: Exclude<PluginConfigFieldKind, 'enum'>
): value is PluginConfigScalar => {
  if (type === 'string') return typeof value === 'string';
  if (type === 'boolean') return typeof value === 'boolean';
  if (type === 'integer') return typeof value === 'number' && Number.isInteger(value);
  return typeof value === 'number' && Number.isFinite(value);
};

const unsupportedKeywords = (
  value: Record<string, unknown>,
  supported: Set<string>,
  path: string
): PluginConfigSchemaIssue[] =>
  Object.keys(value)
    .filter((keyword) => !supported.has(keyword))
    .map((keyword) => ({
      code: 'unsupported_keyword' as const,
      path,
      keyword,
    }));

const SUSPICIOUS_SECRET_KEY =
  /(^|[._-])(password|passwd|passphrase|secret|token|api[._-]?key|access[._-]?token|refresh[._-]?token|private[._-]?key|client[._-]?secret|credential)([._-]|$)/i;

const secretConfigMarker = (
  key: string,
  schema: Record<string, unknown>
): string | undefined => {
  if (schema.writeOnly === true) return 'writeOnly';
  if (schema.secret === true) return 'secret';
  if (schema['x-secret'] === true) return 'x-secret';
  if (schema['x-nomifun-secret'] === true) return 'x-nomifun-secret';
  if (schema.format === 'password') return 'format:password';
  if (SUSPICIOUS_SECRET_KEY.test(key)) return 'property-name';
  return undefined;
};

const enumDeclaredType = (
  schema: Record<string, unknown>,
  values: unknown[]
): Exclude<PluginConfigFieldKind, 'enum'> | undefined => {
  const declared = schema.type;
  if (
    declared === 'string' ||
    declared === 'number' ||
    declared === 'integer' ||
    declared === 'boolean'
  ) {
    return declared;
  }
  if (declared !== undefined || values.length === 0) return undefined;
  const inferred = values.map(scalarType);
  if (inferred.every((entry) => entry === 'integer')) return 'integer';
  if (inferred.every((entry) => entry === 'integer' || entry === 'number')) {
    return 'number';
  }
  const first = inferred[0];
  return first && inferred.every((entry) => entry === first) ? first : undefined;
};

const parseField = (
  key: string,
  raw: unknown,
  required: boolean,
  currentValues: PluginConfigValues
): { field?: PluginConfigFieldModel; issues: PluginConfigSchemaIssue[] } => {
  const path = `properties.${key}`;
  if (!isRecord(raw)) {
    return { issues: [{ code: 'field_not_object', path }] };
  }

  const issues = unsupportedKeywords(raw, FIELD_KEYWORDS, path);
  if (raw.readOnly === true) {
    issues.push({ code: 'field_read_only', path });
  }
  if (
    (raw.writeOnly !== undefined && typeof raw.writeOnly !== 'boolean') ||
    (raw.secret !== undefined && typeof raw.secret !== 'boolean') ||
    (raw['x-secret'] !== undefined && typeof raw['x-secret'] !== 'boolean') ||
    (raw['x-nomifun-secret'] !== undefined &&
      typeof raw['x-nomifun-secret'] !== 'boolean') ||
    (raw.title !== undefined && typeof raw.title !== 'string') ||
    (raw.description !== undefined && typeof raw.description !== 'string')
  ) {
    issues.push({ code: 'field_constraint_invalid', path });
  }
  const secretMarker = secretConfigMarker(key, raw);
  if (secretMarker) {
    issues.push({
      code: 'secret_config_unsupported',
      path,
      keyword: secretMarker,
    });
  }
  if (
    raw.format !== undefined &&
    raw.format !== 'password'
  ) {
    issues.push({
      code: 'unsupported_keyword',
      path,
      keyword: `format:${String(raw.format)}`,
    });
  }

  const rawEnum = raw.enum;
  let declaredType:
    | Exclude<PluginConfigFieldKind, 'enum'>
    | undefined;
  let enumOptions: PluginConfigEnumOption[] = [];

  if (rawEnum !== undefined) {
    if (!Array.isArray(rawEnum)) {
      issues.push({ code: 'field_enum_invalid', path });
    } else {
      declaredType = enumDeclaredType(raw, rawEnum);
      const unique = new Set<string>();
      const valid =
        declaredType !== undefined &&
        rawEnum.length > 0 &&
        rawEnum.every((value) => {
          if (!matchesDeclaredType(value, declaredType!)) return false;
          const identity = `${typeof value}:${String(value)}`;
          if (unique.has(identity)) return false;
          unique.add(identity);
          return true;
        });
      if (!valid) {
        issues.push({ code: 'field_enum_invalid', path });
      } else {
        enumOptions = rawEnum.map((value, index) => ({
          id: String(index),
          label: String(value),
          value: value as PluginConfigScalar,
        }));
      }
    }
  } else if (
    raw.type === 'string' ||
    raw.type === 'number' ||
    raw.type === 'integer' ||
    raw.type === 'boolean'
  ) {
    declaredType = raw.type;
  } else {
    issues.push({ code: 'field_type_unsupported', path });
  }

  const minimum = raw.minimum === undefined ? undefined : finiteNumber(raw.minimum);
  const maximum = raw.maximum === undefined ? undefined : finiteNumber(raw.maximum);
  const minLength =
    raw.minLength === undefined ? undefined : nonNegativeInteger(raw.minLength);
  const maxLength =
    raw.maxLength === undefined ? undefined : nonNegativeInteger(raw.maxLength);
  if (
    (raw.minimum !== undefined && minimum === undefined) ||
    (raw.maximum !== undefined && maximum === undefined) ||
    (minimum !== undefined && maximum !== undefined && minimum > maximum) ||
    (raw.minLength !== undefined && minLength === undefined) ||
    (raw.maxLength !== undefined && maxLength === undefined) ||
    (minLength !== undefined && maxLength !== undefined && minLength > maxLength) ||
    ((minimum !== undefined || maximum !== undefined) &&
      declaredType !== 'number' &&
      declaredType !== 'integer') ||
    ((minLength !== undefined || maxLength !== undefined) &&
      declaredType !== 'string')
  ) {
    issues.push({ code: 'field_constraint_invalid', path });
  }

  if (!declaredType || issues.length > 0) {
    return { issues };
  }

  const defaultValue = matchesDeclaredType(raw.default, declaredType)
    ? raw.default
    : undefined;
  if (raw.default !== undefined && defaultValue === undefined) {
    issues.push({ code: 'field_constraint_invalid', path });
    return { issues };
  }

  const currentValue = currentValues[key];
  const initialValue = matchesDeclaredType(currentValue, declaredType)
    ? currentValue
    : required
      ? defaultValue
      : undefined;

  return {
    issues,
    field: {
      key,
      label: text(raw.title) ?? key,
      description: text(raw.description),
      required,
      kind: rawEnum !== undefined ? 'enum' : declaredType,
      enumOptions,
      defaultValue,
      initialValue,
      minimum,
      maximum,
      minLength,
      maxLength,
    },
  };
};

export function pluginConfigurationEditorModel(
  detail: PluginDetail
): PluginConfigurationEditorModel {
  const rawSchema = detail.config_schema.schema;
  if (!isRecord(rawSchema)) {
    return {
      fields: [],
      schemaIssues: [{ code: 'root_not_object', path: '$' }],
      canSubmit: false,
    };
  }

  const issues = unsupportedKeywords(rawSchema, ROOT_KEYWORDS, '$');
  if (rawSchema.type !== undefined && rawSchema.type !== 'object') {
    issues.push({ code: 'root_type_unsupported', path: '$' });
  }
  if (rawSchema.additionalProperties !== false) {
    issues.push({ code: 'dynamic_properties_unsupported', path: '$' });
  }
  if (
    (rawSchema.title !== undefined && typeof rawSchema.title !== 'string') ||
    (rawSchema.description !== undefined &&
      typeof rawSchema.description !== 'string')
  ) {
    issues.push({ code: 'field_constraint_invalid', path: '$' });
  }
  if (detail.config.schema_digest !== detail.config_schema.schema_digest) {
    issues.push({ code: 'schema_digest_mismatch', path: '$' });
  }

  const rawProperties = rawSchema.properties ?? {};
  const properties = isRecord(rawProperties) ? rawProperties : {};
  if (!isRecord(rawProperties)) {
    issues.push({ code: 'properties_not_object', path: 'properties' });
  }

  const rawRequired = rawSchema.required ?? [];
  const requiredValues =
    Array.isArray(rawRequired) && rawRequired.every((entry) => typeof entry === 'string')
      ? rawRequired
      : [];
  if (
    !Array.isArray(rawRequired) ||
    rawRequired.some((entry) => typeof entry !== 'string')
  ) {
    issues.push({ code: 'required_invalid', path: 'required' });
  }
  const required = new Set(requiredValues as string[]);
  for (const key of required) {
    if (!hasOwn(properties, key)) {
      issues.push({
        code: 'required_property_missing',
        path: `required.${key}`,
      });
    }
  }

  const fields: PluginConfigFieldModel[] = [];
  for (const [key, rawField] of Object.entries(properties)) {
    const parsed = parseField(key, rawField, required.has(key), detail.config.values);
    issues.push(...parsed.issues);
    if (parsed.field) fields.push(parsed.field);
  }
  for (const key of Object.keys(detail.config.values)) {
    if (!hasOwn(properties, key)) {
      issues.push({
        code: 'dynamic_properties_unsupported',
        path: `values.${key}`,
      });
    }
  }

  const credentialKeys = new Set<string>();
  for (const slot of detail.credential_slots) {
    if (credentialKeys.has(slot.slot_key)) {
      issues.push({
        code: 'duplicate_credential_slot',
        path: `credential_slots.${slot.slot_key}`,
      });
    }
    credentialKeys.add(slot.slot_key);
  }
  if (!detail.summary.current) {
    issues.push({ code: 'mount_target_missing', path: 'summary.current' });
  }

  return {
    title: text(rawSchema.title),
    description: text(rawSchema.description),
    fields,
    schemaIssues: issues,
    canSubmit: issues.length === 0 && detail.summary.current !== undefined,
  };
}

const initialBoolean = (field: PluginConfigFieldModel): boolean | undefined => {
  if (typeof field.initialValue === 'boolean') return field.initialValue;
  return field.required ? false : undefined;
};

export function createPluginConfigurationDraft(
  detail: PluginDetail,
  editor = pluginConfigurationEditorModel(detail)
): PluginConfigurationDraft {
  const values: PluginConfigurationDraft['values'] = {};
  const credentials: PluginConfigurationDraft['credentials'] = {};

  for (const field of editor.fields) {
    values[field.key] =
      field.kind === 'boolean' ? initialBoolean(field) : field.initialValue;
  }

  for (const slot of detail.credential_slots) {
    credentials[slot.slot_key] = {
      action: slot.credential_id
        ? 'keep'
        : slot.required
          ? 'bind'
          : 'unbind',
      credentialId: '',
    };
  }

  return { values, credentials };
}

const enumContains = (
  field: PluginConfigFieldModel,
  value: PluginConfigScalar
): boolean => field.enumOptions.some((option) => Object.is(option.value, value));

const validateConfigField = (
  field: PluginConfigFieldModel,
  draft: PluginConfigurationDraft
): PluginConfigurationDraftIssue[] => {
  const issues: PluginConfigurationDraftIssue[] = [];
  const value = draft.values[field.key];
  if (value === undefined) {
    if (field.required) {
      issues.push({ code: 'required', scope: 'config', key: field.key });
    }
    return issues;
  }
  if (field.kind === 'enum') {
    if (!enumContains(field, value)) {
      issues.push({ code: 'enum_value', scope: 'config', key: field.key });
    }
    return issues;
  }
  if (!matchesDeclaredType(value, field.kind)) {
    issues.push({ code: 'invalid_type', scope: 'config', key: field.key });
    return issues;
  }
  if (field.kind === 'integer' && !Number.isInteger(value)) {
    issues.push({ code: 'integer_required', scope: 'config', key: field.key });
  }
  if (typeof value === 'number') {
    if (field.minimum !== undefined && value < field.minimum) {
      issues.push({
        code: 'minimum',
        scope: 'config',
        key: field.key,
        limit: field.minimum,
      });
    }
    if (field.maximum !== undefined && value > field.maximum) {
      issues.push({
        code: 'maximum',
        scope: 'config',
        key: field.key,
        limit: field.maximum,
      });
    }
  }
  if (typeof value === 'string') {
    if (field.minLength !== undefined && value.length < field.minLength) {
      issues.push({
        code: 'min_length',
        scope: 'config',
        key: field.key,
        limit: field.minLength,
      });
    }
    if (field.maxLength !== undefined && value.length > field.maxLength) {
      issues.push({
        code: 'max_length',
        scope: 'config',
        key: field.key,
        limit: field.maxLength,
      });
    }
  }
  return issues;
};

const resolvedCredential = (
  slot: PluginCredentialSlotBinding,
  draft: PluginCredentialDraft | undefined
): string | null => {
  if (!draft || draft.action === 'keep') return slot.credential_id ?? null;
  if (draft.action === 'unbind') return null;
  return draft.credentialId.trim() || null;
};

export function validatePluginConfigurationDraft(
  detail: PluginDetail,
  editor: PluginConfigurationEditorModel,
  draft: PluginConfigurationDraft
): PluginConfigurationDraftIssue[] {
  if (!editor.canSubmit) return [];
  const issues = editor.fields.flatMap((field) => validateConfigField(field, draft));
  for (const slot of detail.credential_slots) {
    const credentialDraft = draft.credentials[slot.slot_key];
    if (credentialDraft?.action === 'bind' && credentialDraft.credentialId.trim() === '') {
      issues.push({
        code: 'credential_id_required',
        scope: 'credential',
        key: slot.slot_key,
      });
      continue;
    }
    if (slot.required && resolvedCredential(slot, credentialDraft) === null) {
      issues.push({
        code: 'credential_required',
        scope: 'credential',
        key: slot.slot_key,
      });
    }
  }
  return issues;
}

export function configurePluginRequest(
  detail: PluginDetail,
  editor: PluginConfigurationEditorModel,
  draft: PluginConfigurationDraft
): ConfigurePluginRequest {
  const current = detail.summary.current;
  if (!current) throw new Error('Plugin Mount has no current target');
  if (!editor.canSubmit || editor.schemaIssues.length > 0) {
    throw new Error('Plugin config schema is not editable');
  }
  const issues = validatePluginConfigurationDraft(detail, editor, draft);
  if (issues.length > 0) {
    throw new Error('Plugin configuration draft is invalid');
  }

  const values: PluginConfigValues = {};
  for (const field of editor.fields) {
    const value = draft.values[field.key];
    if (value !== undefined) values[field.key] = value;
  }

  const credentialBindings: Record<string, string | null> = {};
  for (const slot of detail.credential_slots) {
    credentialBindings[slot.slot_key] = resolvedCredential(
      slot,
      draft.credentials[slot.slot_key]
    );
  }

  return {
    mount_id: detail.summary.mount_id,
    expected_mount_revision: detail.summary.mount_revision,
    expected_current_target_digest: current.artifact_digest,
    expected_config_revision: detail.config.config_revision,
    expected_schema_digest: detail.config_schema.schema_digest,
    values,
    credential_bindings: credentialBindings,
    expected_credential_bindings_revision: detail.credential_bindings_revision,
  };
}
