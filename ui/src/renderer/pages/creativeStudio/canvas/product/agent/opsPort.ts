/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpRequest } from '@/common/adapter/httpBridge';
import { CANONICAL_UUID_V7 } from '@/common/types/ids';

import {
  CreativeStudioContractError,
  parseCreativeProjectSummary,
  type CreativeProjectSummary,
} from '../../../domain';
import {
  parseCreativeCanvasAgentOps,
  type CreativeCanvasAgentOp,
} from './artifacts';

export type CreativeCanvasAgentOpResult =
  | { type: 'node_added'; node_id: string }
  | { type: 'node_updated'; node_id: string }
  | { type: 'node_moved'; node_id: string }
  | { type: 'node_resized'; node_id: string }
  | { type: 'nodes_connected'; connection_id: string }
  | { type: 'nodes_disconnected'; connection_id: string };

export interface CreativeCanvasAgentOpsApplyInput {
  projectId: string;
  assistantMessageId: string;
  expectedRevision: string;
  ops: readonly CreativeCanvasAgentOp[];
}

export interface CreativeCanvasAgentOpsApplyResult {
  project: CreativeProjectSummary;
  ops: CreativeCanvasAgentOpResult[];
  replayed: boolean;
  appliedRevision: string;
}

export interface CreativeCanvasAgentOpsPort {
  apply(input: CreativeCanvasAgentOpsApplyInput): Promise<CreativeCanvasAgentOpsApplyResult>;
}

export type CreativeCanvasAgentOpsHttpRequest = (
  method: string,
  path: string,
  body?: unknown
) => Promise<unknown>;

type UnknownRecord = Record<string, unknown>;

const defaultRequest: CreativeCanvasAgentOpsHttpRequest = (method, path, body) =>
  httpRequest<unknown>(method, path, body);

const fail = (path: string, expected: string): never => {
  throw new CreativeStudioContractError('INVALID_RESPONSE', path, expected);
};

const inputFail = (path: string, expected: string): never => {
  throw new CreativeStudioContractError('INVALID_REQUEST', path, expected);
};

const asRecord = (value: unknown, path: string): UnknownRecord => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return fail(path, 'object');
  }
  return value as UnknownRecord;
};

const exactKeys = (
  value: UnknownRecord,
  expected: readonly string[],
  path: string
): void => {
  const allowed = new Set(expected);
  for (const key of expected) {
    if (!Object.prototype.hasOwnProperty.call(value, key)) {
      fail(`${path}.${key}`, 'present');
    }
  }
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) fail(`${path}.${key}`, 'no unknown fields');
  }
};

const asUuidV7 = (value: unknown, path: string): string => {
  if (typeof value !== 'string' || !CANONICAL_UUID_V7.test(value)) {
    return fail(path, 'canonical lowercase UUIDv7');
  }
  return value;
};

const assertInputUuidV7 = (value: unknown, path: string): string => {
  if (typeof value !== 'string' || !CANONICAL_UUID_V7.test(value)) {
    return inputFail(path, 'canonical lowercase UUIDv7');
  }
  return value;
};

const assertExpectedRevision = (value: unknown): string => {
  if (
    typeof value !== 'string' ||
    value.length > 19 ||
    !/^[1-9]\d*$/.test(value) ||
    BigInt(value) > 9_223_372_036_854_775_806n
  ) {
    return inputFail(
      '$.expectedRevision',
      'canonical decimal revision from 1 through 9223372036854775806'
    );
  }
  return value;
};

const asBoolean = (value: unknown, path: string): boolean => {
  if (typeof value !== 'boolean') return fail(path, 'boolean');
  return value;
};

const asI64Revision = (value: unknown, path: string): string => {
  if (
    typeof value !== 'string' ||
    !/^[1-9]\d{0,18}$/.test(value) ||
    BigInt(value) > 9_223_372_036_854_775_807n
  ) {
    return fail(path, 'canonical positive i64 decimal revision');
  }
  return value;
};

const parseResult = (value: unknown, path: string): CreativeCanvasAgentOpResult => {
  const record = asRecord(value, path);
  if (typeof record.type !== 'string') fail(`${path}.type`, 'operation result type');
  switch (record.type) {
    case 'node_added':
    case 'node_updated':
    case 'node_moved':
    case 'node_resized':
      exactKeys(record, ['type', 'node_id'], path);
      return { type: record.type, node_id: asUuidV7(record.node_id, `${path}.node_id`) };
    case 'nodes_connected':
    case 'nodes_disconnected':
      exactKeys(record, ['type', 'connection_id'], path);
      return {
        type: record.type,
        connection_id: asUuidV7(record.connection_id, `${path}.connection_id`),
      };
    default:
      return fail(
        `${path}.type`,
        'node_added | node_updated | node_moved | node_resized | nodes_connected | nodes_disconnected'
      );
  }
};

const assertResultMatchesOp = (
  op: CreativeCanvasAgentOp,
  result: CreativeCanvasAgentOpResult,
  path: string
): void => {
  switch (op.type) {
    case 'add_node':
      if (result.type !== 'node_added') fail(`${path}.type`, JSON.stringify('node_added'));
      return;
    case 'update_node_data':
      if (result.type !== 'node_updated' || result.node_id !== op.node_id) {
        fail(path, `node_updated result for ${JSON.stringify(op.node_id)}`);
      }
      return;
    case 'move_node':
      if (result.type !== 'node_moved' || result.node_id !== op.node_id) {
        fail(path, `node_moved result for ${JSON.stringify(op.node_id)}`);
      }
      return;
    case 'resize_node':
      if (result.type !== 'node_resized' || result.node_id !== op.node_id) {
        fail(path, `node_resized result for ${JSON.stringify(op.node_id)}`);
      }
      return;
    case 'connect':
      if (result.type !== 'nodes_connected') {
        fail(`${path}.type`, JSON.stringify('nodes_connected'));
      }
      return;
    case 'disconnect':
      if (
        result.type !== 'nodes_disconnected' ||
        result.connection_id !== op.connection_id
      ) {
        fail(path, `nodes_disconnected result for ${JSON.stringify(op.connection_id)}`);
      }
  }
};

const parseApplyResponse = (
  value: unknown,
  projectId: string,
  expectedRevision: string,
  submittedOps: readonly CreativeCanvasAgentOp[]
): CreativeCanvasAgentOpsApplyResult => {
  const record = asRecord(value, '$');
  exactKeys(record, ['project', 'ops', 'replayed', 'appliedRevision'], '$');
  const project = parseCreativeProjectSummary(record.project);
  if (project.projectId !== projectId) {
    throw new CreativeStudioContractError(
      'PROJECT_MISMATCH',
      '$.project.projectId',
      JSON.stringify(projectId)
    );
  }
  const projectRevisionWire = asI64Revision(project.revision, '$.project.revision');
  const replayed = asBoolean(record.replayed, '$.replayed');
  const appliedRevisionWire = asI64Revision(
    record.appliedRevision,
    '$.appliedRevision'
  );
  const projectRevision = BigInt(projectRevisionWire);
  const appliedRevision = BigInt(appliedRevisionWire);
  if (!replayed) {
    const expectedNext = BigInt(expectedRevision) + 1n;
    if (appliedRevision !== expectedNext || projectRevision !== appliedRevision) {
      fail('$.appliedRevision', `first-apply revision ${expectedNext}`);
    }
  } else if (projectRevision < appliedRevision) {
    fail('$.project.revision', `revision >= replayed apply ${appliedRevision}`);
  }
  const wireOps: unknown[] = Array.isArray(record.ops)
    ? record.ops
    : fail('$.ops', `array with exactly ${submittedOps.length} operation results`);
  if (wireOps.length !== submittedOps.length) {
    fail('$.ops', `array with exactly ${submittedOps.length} operation results`);
  }
  const results = wireOps.map((entry, index) => parseResult(entry, `$.ops[${index}]`));
  results.forEach((result, index) =>
    assertResultMatchesOp(submittedOps[index]!, result, `$.ops[${index}]`)
  );
  return {
    project,
    ops: results,
    replayed,
    appliedRevision: appliedRevisionWire,
  };
};

/**
 * Create the shared-bridge HTTP adapter. A network response lost after the
 * server commits remains ambiguous to that request; callers receive the error
 * and this port never retries automatically. A later deliberate retry with the
 * same assistant message ID is resolved by the server receipt without a second
 * graph mutation.
 */
export function createCreativeCanvasAgentOpsPort(
  request: CreativeCanvasAgentOpsHttpRequest = defaultRequest
): CreativeCanvasAgentOpsPort {
  return {
    async apply(input) {
      const projectId = assertInputUuidV7(input.projectId, '$.projectId');
      const assistantMessageId = assertInputUuidV7(
        input.assistantMessageId,
        '$.assistantMessageId'
      );
      const expectedRevision = assertExpectedRevision(input.expectedRevision);
      const ops = parseCreativeCanvasAgentOps(input.ops, 'INVALID_REQUEST');
      const response = await request(
        'POST',
        `/api/creative-studio/projects/${encodeURIComponent(projectId)}/agent-ops`,
        { assistantMessageId, expectedRevision, ops }
      );
      return parseApplyResponse(response, projectId, expectedRevision, ops);
    },
  };
}

export const creativeCanvasAgentOpsPort = createCreativeCanvasAgentOpsPort();
