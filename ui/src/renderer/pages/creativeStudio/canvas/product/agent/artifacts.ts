/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  CreativeStudioContractError,
  type CreativeStudioContractErrorCode,
} from '../../../domain';

export const CREATIVE_CANVAS_AGENT_ARTIFACT_KIND =
  'nomifun.creative-studio.canvas-ops/v1' as const;
export const MAX_CREATIVE_CANVAS_AGENT_ARTIFACT_JSON_BYTES = 262_144;

export interface CreativeCanvasAgentTextData {
  text: string;
  format: 'plain' | 'markdown';
  fontSize: number;
  textAlign: 'left' | 'center' | 'right';
}

export type CreativeCanvasAgentTextDataPatch = Partial<CreativeCanvasAgentTextData>;

/**
 * Closed, non-destructive Agent mutation wire. The server owns durable IDs,
 * z-order and lock state for newly added nodes.
 */
export type CreativeCanvasAgentOp =
  | {
      type: 'add_node';
      node_type: 'text';
      x: number;
      y: number;
      width?: number;
      height?: number;
      group_id?: string | null;
      data: CreativeCanvasAgentTextData;
    }
  | {
      type: 'update_node_data';
      node_id: string;
      patch: CreativeCanvasAgentTextDataPatch;
    }
  | { type: 'move_node'; node_id: string; x: number; y: number }
  | { type: 'resize_node'; node_id: string; width: number; height: number }
  | {
      type: 'connect';
      source_node_id: string;
      target_node_id: string;
      source_handle?: string | null;
      target_handle?: string | null;
    }
  | { type: 'disconnect'; connection_id: string };

export interface CreativeCanvasAgentArtifact {
  kind: typeof CREATIVE_CANVAS_AGENT_ARTIFACT_KIND;
  summary: string;
  ops: CreativeCanvasAgentOp[];
}

type UnknownRecord = Record<string, unknown>;

const UUID_V7 =
  /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const POSITIVE_DIMENSION = { min: 1 } as const;

const fail = (
  code: CreativeStudioContractErrorCode,
  path: string,
  expected: string
): never => {
  throw new CreativeStudioContractError(code, path, expected);
};

const asRecord = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): UnknownRecord => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return fail(code, path, 'object');
  }
  return value as UnknownRecord;
};

const hasOwn = (value: UnknownRecord, key: string): boolean =>
  Object.prototype.hasOwnProperty.call(value, key);

const exactKeys = (
  value: UnknownRecord,
  required: readonly string[],
  optional: readonly string[],
  path: string,
  code: CreativeStudioContractErrorCode
): void => {
  const allowed = new Set([...required, ...optional]);
  for (const key of required) {
    if (!hasOwn(value, key)) fail(code, `${path}.${key}`, 'present');
  }
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) fail(code, `${path}.${key}`, 'no unknown fields');
  }
};

const asFiniteNumber = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode,
  options: { min?: number; max?: number; minExclusive?: number } = {}
): number => {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return fail(code, path, 'finite number');
  }
  if (options.min !== undefined && value < options.min) {
    return fail(code, path, `number >= ${options.min}`);
  }
  if (options.max !== undefined && value > options.max) {
    return fail(code, path, `number <= ${options.max}`);
  }
  if (options.minExclusive !== undefined && value <= options.minExclusive) {
    return fail(code, path, `number > ${options.minExclusive}`);
  }
  return value;
};

const asString = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode,
  options: { allowEmpty?: boolean; maxLength: number; trimmed?: boolean }
): string => {
  if (typeof value !== 'string') return fail(code, path, 'string');
  for (let index = 0; index < value.length; index += 1) {
    const unit = value.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (!(next >= 0xdc00 && next <= 0xdfff)) {
        return fail(code, path, 'Unicode string without unpaired surrogates');
      }
      index += 1;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      return fail(code, path, 'Unicode string without unpaired surrogates');
    }
  }
  const length = Array.from(value).length;
  if ((!options.allowEmpty && length === 0) || length > options.maxLength) {
    return fail(
      code,
      path,
      `${options.allowEmpty ? '' : 'non-empty '}string <= ${options.maxLength} chars`
    );
  }
  if (options.trimmed && value !== value.trim()) {
    return fail(code, path, `trimmed string <= ${options.maxLength} chars`);
  }
  return value;
};

const asUuidV7 = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): string => {
  if (typeof value !== 'string' || !UUID_V7.test(value)) {
    return fail(code, path, 'canonical lowercase UUIDv7');
  }
  return value;
};

const asLiteral = <T extends string>(
  value: unknown,
  allowed: readonly T[],
  path: string,
  code: CreativeStudioContractErrorCode
): T => {
  if (typeof value !== 'string' || !allowed.includes(value as T)) {
    return fail(code, path, allowed.map((entry) => JSON.stringify(entry)).join(' | '));
  }
  return value as T;
};

const parseTextFields = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode,
  complete: boolean
): CreativeCanvasAgentTextData | CreativeCanvasAgentTextDataPatch => {
  const record = asRecord(value, path, code);
  const fields = ['text', 'format', 'fontSize', 'textAlign'] as const;
  exactKeys(record, complete ? fields : [], complete ? [] : fields, path, code);
  if (!complete && Object.keys(record).length === 0) {
    fail(code, path, 'at least one text data field');
  }

  const output: CreativeCanvasAgentTextDataPatch = {};
  if (hasOwn(record, 'text')) {
    output.text = asString(record.text, `${path}.text`, code, {
      allowEmpty: true,
      maxLength: 20_000,
    });
  }
  if (hasOwn(record, 'format')) {
    output.format = asLiteral(
      record.format,
      ['plain', 'markdown'] as const,
      `${path}.format`,
      code
    );
  }
  if (hasOwn(record, 'fontSize')) {
    output.fontSize = asFiniteNumber(record.fontSize, `${path}.fontSize`, code, {
      min: 8,
      max: 256,
    });
  }
  if (hasOwn(record, 'textAlign')) {
    output.textAlign = asLiteral(
      record.textAlign,
      ['left', 'center', 'right'] as const,
      `${path}.textAlign`,
      code
    );
  }
  return output as CreativeCanvasAgentTextData | CreativeCanvasAgentTextDataPatch;
};

const optionalDimension = (
  record: UnknownRecord,
  key: 'width' | 'height',
  path: string,
  code: CreativeStudioContractErrorCode
): number | undefined =>
  hasOwn(record, key)
    ? asFiniteNumber(record[key], `${path}.${key}`, code, POSITIVE_DIMENSION)
    : undefined;

const optionalGroupId = (
  record: UnknownRecord,
  path: string,
  code: CreativeStudioContractErrorCode
): string | null | undefined => {
  if (!hasOwn(record, 'group_id')) return undefined;
  return record.group_id === null
    ? null
    : asUuidV7(record.group_id, `${path}.group_id`, code);
};

const optionalHandle = (
  record: UnknownRecord,
  key: 'source_handle' | 'target_handle',
  path: string,
  code: CreativeStudioContractErrorCode
): string | null | undefined => {
  if (!hasOwn(record, key)) return undefined;
  return record[key] === null
    ? null
    : asString(record[key], `${path}.${key}`, code, {
        maxLength: 128,
        trimmed: true,
      });
};

const parseOp = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): CreativeCanvasAgentOp => {
  const record = asRecord(value, path, code);
  const type = asString(record.type, `${path}.type`, code, { maxLength: 64 });
  switch (type) {
    case 'add_node': {
      exactKeys(
        record,
        ['type', 'node_type', 'x', 'y', 'data'],
        ['width', 'height', 'group_id'],
        path,
        code
      );
      if (record.node_type !== 'text') {
        fail(code, `${path}.node_type`, JSON.stringify('text'));
      }
      const width = optionalDimension(record, 'width', path, code);
      const height = optionalDimension(record, 'height', path, code);
      const groupId = optionalGroupId(record, path, code);
      return {
        type: 'add_node',
        node_type: 'text',
        x: asFiniteNumber(record.x, `${path}.x`, code),
        y: asFiniteNumber(record.y, `${path}.y`, code),
        ...(width === undefined ? {} : { width }),
        ...(height === undefined ? {} : { height }),
        ...(groupId === undefined ? {} : { group_id: groupId }),
        data: parseTextFields(record.data, `${path}.data`, code, true) as CreativeCanvasAgentTextData,
      };
    }
    case 'update_node_data':
      exactKeys(record, ['type', 'node_id', 'patch'], [], path, code);
      return {
        type: 'update_node_data',
        node_id: asUuidV7(record.node_id, `${path}.node_id`, code),
        patch: parseTextFields(
          record.patch,
          `${path}.patch`,
          code,
          false
        ) as CreativeCanvasAgentTextDataPatch,
      };
    case 'move_node':
      exactKeys(record, ['type', 'node_id', 'x', 'y'], [], path, code);
      return {
        type: 'move_node',
        node_id: asUuidV7(record.node_id, `${path}.node_id`, code),
        x: asFiniteNumber(record.x, `${path}.x`, code),
        y: asFiniteNumber(record.y, `${path}.y`, code),
      };
    case 'resize_node':
      exactKeys(record, ['type', 'node_id', 'width', 'height'], [], path, code);
      return {
        type: 'resize_node',
        node_id: asUuidV7(record.node_id, `${path}.node_id`, code),
        width: asFiniteNumber(record.width, `${path}.width`, code, POSITIVE_DIMENSION),
        height: asFiniteNumber(record.height, `${path}.height`, code, POSITIVE_DIMENSION),
      };
    case 'connect': {
      exactKeys(
        record,
        ['type', 'source_node_id', 'target_node_id'],
        ['source_handle', 'target_handle'],
        path,
        code
      );
      const sourceHandle = optionalHandle(record, 'source_handle', path, code);
      const targetHandle = optionalHandle(record, 'target_handle', path, code);
      return {
        type: 'connect',
        source_node_id: asUuidV7(record.source_node_id, `${path}.source_node_id`, code),
        target_node_id: asUuidV7(record.target_node_id, `${path}.target_node_id`, code),
        ...(sourceHandle === undefined ? {} : { source_handle: sourceHandle }),
        ...(targetHandle === undefined ? {} : { target_handle: targetHandle }),
      };
    }
    case 'disconnect':
      exactKeys(record, ['type', 'connection_id'], [], path, code);
      return {
        type: 'disconnect',
        connection_id: asUuidV7(record.connection_id, `${path}.connection_id`, code),
      };
    default:
      return fail(
        code,
        `${path}.type`,
        'add_node | update_node_data | move_node | resize_node | connect | disconnect'
      );
  }
};

/** Validate typed or decoded operations before they cross the HTTP boundary. */
export function parseCreativeCanvasAgentOps(
  value: unknown,
  code: CreativeStudioContractErrorCode = 'INVALID_REQUEST',
  path = '$.ops'
): CreativeCanvasAgentOp[] {
  if (!Array.isArray(value) || value.length < 1 || value.length > 64) {
    return fail(code, path, 'array with 1 to 64 operations');
  }
  return value.map((op, index) => parseOp(op, `${path}[${index}]`, code));
}

/**
 * A lexical JSON pass that rejects duplicate decoded object keys. JSON.parse
 * is intentionally insufficient here because it silently keeps the last key.
 */
class StrictJsonScanner {
  private index = 0;

  constructor(private readonly source: string) {}

  scan(): void {
    this.skipWhitespace();
    this.scanValue(0);
    this.skipWhitespace();
    if (this.index !== this.source.length) throw new SyntaxError('unexpected trailing JSON');
  }

  private scanValue(depth: number): void {
    if (depth > 64) throw new SyntaxError('JSON nesting exceeds 64 levels');
    const current = this.source[this.index];
    if (current === '{') return this.scanObject(depth + 1);
    if (current === '[') return this.scanArray(depth + 1);
    if (current === '"') {
      this.scanString();
      return;
    }
    if (current === 't') return this.scanLiteral('true');
    if (current === 'f') return this.scanLiteral('false');
    if (current === 'n') return this.scanLiteral('null');
    this.scanNumber();
  }

  private scanObject(depth: number): void {
    this.index += 1;
    this.skipWhitespace();
    if (this.source[this.index] === '}') {
      this.index += 1;
      return;
    }
    const keys = new Set<string>();
    while (true) {
      if (this.source[this.index] !== '"') throw new SyntaxError('object key must be a string');
      const key = this.scanString();
      if (keys.has(key)) throw new SyntaxError(`duplicate JSON key ${JSON.stringify(key)}`);
      keys.add(key);
      this.skipWhitespace();
      if (this.source[this.index] !== ':') throw new SyntaxError('object key must be followed by colon');
      this.index += 1;
      this.skipWhitespace();
      this.scanValue(depth);
      this.skipWhitespace();
      const delimiter = this.source[this.index];
      if (delimiter === '}') {
        this.index += 1;
        return;
      }
      if (delimiter !== ',') throw new SyntaxError('object entries must be comma separated');
      this.index += 1;
      this.skipWhitespace();
    }
  }

  private scanArray(depth: number): void {
    this.index += 1;
    this.skipWhitespace();
    if (this.source[this.index] === ']') {
      this.index += 1;
      return;
    }
    while (true) {
      this.scanValue(depth);
      this.skipWhitespace();
      const delimiter = this.source[this.index];
      if (delimiter === ']') {
        this.index += 1;
        return;
      }
      if (delimiter !== ',') throw new SyntaxError('array entries must be comma separated');
      this.index += 1;
      this.skipWhitespace();
    }
  }

  private scanString(): string {
    const start = this.index;
    this.index += 1;
    while (this.index < this.source.length) {
      const current = this.source.charCodeAt(this.index);
      if (current === 0x22) {
        this.index += 1;
        return JSON.parse(this.source.slice(start, this.index)) as string;
      }
      if (current < 0x20) throw new SyntaxError('unescaped control character in JSON string');
      if (current === 0x5c) {
        this.index += 1;
        const escape = this.source[this.index];
        if (escape === 'u') {
          const hex = this.source.slice(this.index + 1, this.index + 5);
          if (!/^[0-9a-fA-F]{4}$/.test(hex)) throw new SyntaxError('invalid unicode escape');
          this.index += 5;
          continue;
        }
        if (!escape || !'"\\/bfnrt'.includes(escape)) {
          throw new SyntaxError('invalid JSON string escape');
        }
      }
      this.index += 1;
    }
    throw new SyntaxError('unterminated JSON string');
  }

  private scanLiteral(literal: string): void {
    if (this.source.slice(this.index, this.index + literal.length) !== literal) {
      throw new SyntaxError('invalid JSON literal');
    }
    this.index += literal.length;
  }

  private scanNumber(): void {
    const match = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(
      this.source.slice(this.index)
    );
    if (!match) throw new SyntaxError('invalid JSON value');
    this.index += match[0].length;
  }

  private skipWhitespace(): void {
    while (/\s/.test(this.source[this.index] ?? '') && /[\t\n\r ]/.test(this.source[this.index]!)) {
      this.index += 1;
    }
  }
}

const artifactFromValue = (value: unknown): CreativeCanvasAgentArtifact | null => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record = value as UnknownRecord;
  if (record.kind !== CREATIVE_CANVAS_AGENT_ARTIFACT_KIND) return null;
  const code = 'INVALID_RESPONSE';
  exactKeys(record, ['kind', 'summary', 'ops'], [], '$', code);
  return {
    kind: CREATIVE_CANVAS_AGENT_ARTIFACT_KIND,
    summary: asString(record.summary, '$.summary', code, {
      maxLength: 500,
      trimmed: true,
    }),
    ops: parseCreativeCanvasAgentOps(record.ops, code),
  };
};

/**
 * Parse only a unique, final lowercase-json fenced artifact. Ordinary prose
 * and artifacts owned by another product remain assistant text and return null.
 */
export function parseCreativeCanvasAgentArtifact(
  text: string
): CreativeCanvasAgentArtifact | null {
  const candidate = text.trimEnd();
  const opening = '```json\n';
  const closing = '\n```';
  const openingIndex = candidate.lastIndexOf(opening);
  const hasCanonicalOpening =
    openingIndex >= 0 && (openingIndex === 0 || candidate[openingIndex - 1] === '\n');
  if (!hasCanonicalOpening || !candidate.endsWith(closing)) {
    if (text.includes('```') && text.includes(CREATIVE_CANVAS_AGENT_ARTIFACT_KIND)) {
      return fail('INVALID_RESPONSE', '$', 'one final canonical canvas-ops JSON fence');
    }
    return null;
  }

  const jsonText = candidate.slice(openingIndex + opening.length, -closing.length);
  if (
    jsonText.length > MAX_CREATIVE_CANVAS_AGENT_ARTIFACT_JSON_BYTES ||
    new TextEncoder().encode(jsonText).byteLength >
      MAX_CREATIVE_CANVAS_AGENT_ARTIFACT_JSON_BYTES
  ) {
    return fail(
      'INVALID_RESPONSE',
      '$',
      `canvas-ops JSON <= ${MAX_CREATIVE_CANVAS_AGENT_ARTIFACT_JSON_BYTES} UTF-8 bytes`
    );
  }
  let decoded: unknown;
  try {
    decoded = JSON.parse(jsonText) as unknown;
  } catch {
    if (jsonText.includes(CREATIVE_CANVAS_AGENT_ARTIFACT_KIND)) {
      return fail('INVALID_RESPONSE', '$', 'well-formed canvas-ops JSON');
    }
    return null;
  }

  let strictJson = true;
  try {
    new StrictJsonScanner(jsonText).scan();
  } catch {
    strictJson = false;
  }
  const decodedKind =
    decoded && typeof decoded === 'object' && !Array.isArray(decoded)
      ? (decoded as UnknownRecord).kind
      : undefined;
  if (!strictJson) {
    if (
      decodedKind === CREATIVE_CANVAS_AGENT_ARTIFACT_KIND ||
      jsonText.includes(CREATIVE_CANVAS_AGENT_ARTIFACT_KIND)
    ) {
      return fail('INVALID_RESPONSE', '$', 'JSON without duplicate object keys');
    }
    return null;
  }
  if (decodedKind !== CREATIVE_CANVAS_AGENT_ARTIFACT_KIND) return null;

  const fenceCount = candidate.match(/```/g)?.length ?? 0;
  if (fenceCount !== 2) {
    return fail('INVALID_RESPONSE', '$', 'one final canonical canvas-ops JSON fence');
  }
  return artifactFromValue(decoded);
}
