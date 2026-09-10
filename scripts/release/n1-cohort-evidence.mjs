#!/usr/bin/env bun

/**
 * Validate Plugin N1 / MiniApp M1 platform records and build one cohort lock.
 *
 * This tool never executes a platform check and cannot turn a failed or
 * missing required cell into PASS. It only validates already-produced native
 * records, hashes their exact bytes, and enforces the frozen three-cell
 * candidate + signed_rc promotion matrix.
 */

import { createHash } from 'node:crypto';
import {
  closeSync,
  existsSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  writeFileSync,
} from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SHA256 = /^[0-9a-f]{64}$/;
const SOURCE = /^[0-9a-f]{40}$/;
const MACHINE_KEY = /^[a-z0-9][a-z0-9._-]{0,127}$/;
const STAGES = Object.freeze(['candidate', 'signed_rc']);
const REQUIRED_CELLS = Object.freeze({
  windows_desktop_x64: 'x86_64-pc-windows-msvc',
  macos_desktop_arm64: 'aarch64-apple-darwin',
  linux_desktop_x64: 'x86_64-unknown-linux-gnu',
});
const OPTIONAL_CELLS = Object.freeze({
  macos_desktop_x64: 'x86_64-apple-darwin',
  linux_headless_x64: 'x86_64-unknown-linux-gnu',
});
const ALL_CELLS = Object.freeze({ ...REQUIRED_CELLS, ...OPTIONAL_CELLS });

export class N1CohortEvidenceError extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.name = 'N1CohortEvidenceError';
    this.code = code;
    this.details = details;
  }
}

function failure(code, message, details = {}) {
  throw new N1CohortEvidenceError(code, message, details);
}

function exactKeys(value, keys) {
  return (
    value &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value).sort()) === JSON.stringify([...keys].sort())
  );
}

function sortedObject(value) {
  if (Array.isArray(value)) return value.map(sortedObject);
  if (!value || typeof value !== 'object') return value;
  return Object.fromEntries(
    Object.keys(value)
      .sort()
      .map((key) => [key, sortedObject(value[key])]),
  );
}

function sha256Bytes(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

function readRegularJson(path, label) {
  const absolute = resolve(path);
  if (!existsSync(absolute)) failure('evidence_missing', `${label} is missing`, { path: absolute });
  const metadata = lstatSync(absolute);
  if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.size === 0) {
    failure('evidence_not_regular', `${label} must be a non-empty regular file`, { path: absolute });
  }
  let value;
  try {
    value = JSON.parse(readFileSync(absolute, 'utf8'));
  } catch (error) {
    failure('evidence_invalid_json', `${label} is not valid JSON`, {
      path: absolute,
      error: error instanceof Error ? error.message : String(error),
    });
  }
  return {
    path: absolute,
    value,
    sha256: sha256Bytes(readFileSync(absolute)),
  };
}

export function validatePlatformRecord(record) {
  const keys = [
    'artifact_digests',
    'cell_id',
    'check_ids',
    'cohort_id',
    'host_target',
    'requirement',
    'schema_version',
    'source_commit',
    'stage',
    'status',
  ];
  if (record?.not_delivered_reason !== undefined) keys.push('not_delivered_reason');
  if (!exactKeys(record, keys)) {
    failure('record_shape_invalid', 'Platform record contains missing or unknown fields');
  }
  if (
    record.schema_version !== '1.0.0' ||
    !MACHINE_KEY.test(record.cohort_id || '') ||
    !SOURCE.test(record.source_commit || '') ||
    !STAGES.includes(record.stage) ||
    !Object.hasOwn(ALL_CELLS, record.cell_id) ||
    record.host_target !== ALL_CELLS[record.cell_id]
  ) {
    failure('record_identity_invalid', 'Platform record identity is outside the frozen matrix');
  }
  const expectedRequirement = Object.hasOwn(REQUIRED_CELLS, record.cell_id)
    ? 'required'
    : 'optional';
  if (record.requirement !== expectedRequirement) {
    failure('record_requirement_invalid', 'Platform record requirement differs from its cell');
  }
  if (!['pass', 'fail', 'not_delivered'].includes(record.status)) {
    failure('record_status_invalid', 'Platform record status is invalid');
  }
  if (expectedRequirement === 'required' && record.status === 'not_delivered') {
    failure('required_record_not_delivered', 'A required platform cell cannot be not_delivered');
  }
  const hasReason = Object.hasOwn(record, 'not_delivered_reason');
  if (hasReason !== (record.status === 'not_delivered')) {
    failure('record_not_delivered_reason_invalid', 'Only not_delivered records carry a reason');
  }
  if (hasReason && record.not_delivered_reason !== 'not_in_release_scope') {
    failure('record_not_delivered_reason_invalid', 'Unsupported not_delivered reason');
  }
  if (
    !record.artifact_digests ||
    typeof record.artifact_digests !== 'object' ||
    Array.isArray(record.artifact_digests) ||
    !Array.isArray(record.check_ids)
  ) {
    failure('record_evidence_invalid', 'Platform record evidence collections are invalid');
  }
  const artifacts = Object.entries(record.artifact_digests);
  if (record.status === 'pass' && artifacts.length === 0) {
    failure('passing_record_has_no_artifacts', 'A passing platform record requires artifact digests');
  }
  if (record.status !== 'not_delivered' && record.check_ids.length === 0) {
    failure('executed_record_has_no_checks', 'An executed platform record requires check IDs');
  }
  if (
    artifacts.some(([key, digest]) => !MACHINE_KEY.test(key) || !SHA256.test(digest)) ||
    record.check_ids.some((id) => !MACHINE_KEY.test(id)) ||
    new Set(record.check_ids).size !== record.check_ids.length
  ) {
    failure('record_evidence_invalid', 'Platform record evidence contains invalid keys or digests');
  }
  return record;
}

export function buildCohortLock({ cohortId, sourceCommit, inputDigests, records, stable = false }) {
  if (!MACHINE_KEY.test(cohortId || '') || !SOURCE.test(sourceCommit || '')) {
    failure('cohort_identity_invalid', 'Cohort ID or source commit is invalid');
  }
  const inputs = Object.entries(inputDigests || {});
  if (
    inputs.length === 0 ||
    inputs.some(([key, digest]) => !MACHINE_KEY.test(key) || !SHA256.test(digest))
  ) {
    failure('cohort_inputs_invalid', 'Cohort lock requires canonical input digests');
  }
  const cells = {};
  const observed = new Set();
  for (const entry of records) {
    const record = validatePlatformRecord(entry.record);
    if (record.cohort_id !== cohortId || record.source_commit !== sourceCommit) {
      failure('record_cohort_mismatch', 'Platform record belongs to another source cohort');
    }
    if (record.status !== 'pass') {
      failure('record_not_passing', 'Only passing platform records may enter a cohort lock', {
        cell_id: record.cell_id,
        stage: record.stage,
        status: record.status,
      });
    }
    const identity = `${record.cell_id}:${record.stage}`;
    if (observed.has(identity)) failure('record_duplicate', 'Duplicate cell/stage platform record');
    observed.add(identity);
    cells[record.cell_id] ??= {
      requirement: record.requirement,
      validation_manifest_digests: {},
    };
    cells[record.cell_id].validation_manifest_digests[record.stage] = entry.sha256;
  }
  if (stable) {
    for (const cellId of Object.keys(REQUIRED_CELLS)) {
      for (const stage of STAGES) {
        if (!cells[cellId]?.validation_manifest_digests?.[stage]) {
          failure('stable_required_record_missing', 'Stable promotion lacks a required cell/stage', {
            cell_id: cellId,
            stage,
          });
        }
      }
    }
  }
  return {
    schema_version: '1.0.0',
    cohort_id: cohortId,
    source_commit: sourceCommit,
    input_digests: sortedObject(inputDigests),
    cells: sortedObject(cells),
  };
}

export function validateCohortLock(lock, { stable = false } = {}) {
  if (!exactKeys(lock, ['cells', 'cohort_id', 'input_digests', 'schema_version', 'source_commit'])) {
    failure('cohort_shape_invalid', 'Cohort lock contains missing or unknown fields');
  }
  const rebuilt = buildCohortLock({
    cohortId: lock.cohort_id,
    sourceCommit: lock.source_commit,
    inputDigests: lock.input_digests,
    records: Object.entries(lock.cells || {}).flatMap(([cellId, cell]) => {
      if (
        !Object.hasOwn(ALL_CELLS, cellId) ||
        !exactKeys(cell, ['requirement', 'validation_manifest_digests']) ||
        cell.requirement !== (Object.hasOwn(REQUIRED_CELLS, cellId) ? 'required' : 'optional') ||
        !cell.validation_manifest_digests ||
        typeof cell.validation_manifest_digests !== 'object' ||
        Array.isArray(cell.validation_manifest_digests)
      ) {
        failure('cohort_cell_invalid', 'Cohort lock cell is invalid', { cell_id: cellId });
      }
      return Object.entries(cell.validation_manifest_digests).map(([stage, digest]) => {
        if (!STAGES.includes(stage) || !SHA256.test(digest)) {
          failure('cohort_cell_digest_invalid', 'Cohort cell stage/digest is invalid');
        }
        return {
          record: {
            schema_version: '1.0.0',
            cohort_id: lock.cohort_id,
            source_commit: lock.source_commit,
            stage,
            cell_id: cellId,
            requirement: cell.requirement,
            host_target: ALL_CELLS[cellId],
            status: 'pass',
            artifact_digests: { validation_manifest: digest },
            check_ids: ['cohort_lock_shape'],
          },
          sha256: digest,
        };
      });
    }),
    stable,
  });
  if (JSON.stringify(sortedObject(rebuilt)) !== JSON.stringify(sortedObject(lock))) {
    failure('cohort_canonical_mismatch', 'Cohort lock is not the exact canonical structure');
  }
  return lock;
}

function parseCreate(tokens) {
  const values = { inputs: {}, records: [], stable: false };
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (token === '--stable') {
      values.stable = true;
      continue;
    }
    const value = tokens[++index];
    if (!value) throw new Error(`${token} requires a value`);
    if (token === '--cohort') values.cohortId = value;
    else if (token === '--source-commit') values.sourceCommit = value;
    else if (token === '--output') values.output = value;
    else if (token === '--record') values.records.push(value);
    else if (token === '--input') {
      const separator = value.indexOf('=');
      if (separator <= 0) throw new Error('--input must use key=digest');
      values.inputs[value.slice(0, separator)] = value.slice(separator + 1);
    } else throw new Error(`unknown argument: ${token}`);
  }
  if (!values.cohortId || !values.sourceCommit || !values.output || values.records.length === 0) {
    throw new Error('create requires --cohort, --source-commit, --input, --record, and --output');
  }
  return values;
}

function writeExclusive(path, value) {
  const absolute = resolve(path);
  mkdirSync(dirname(absolute), { recursive: true });
  const descriptor = openSync(absolute, 'wx');
  try {
    writeFileSync(descriptor, `${JSON.stringify(sortedObject(value), null, 2)}\n`);
  } finally {
    closeSync(descriptor);
  }
  return absolute;
}

function main(argv) {
  const [command, ...tokens] = argv;
  if (command === '--self-test') return assertSelfTest();
  if (command === 'verify-record' && tokens.length === 2 && tokens[0] === '--record') {
    const loaded = readRegularJson(tokens[1], 'platform record');
    validatePlatformRecord(loaded.value);
    return { status: 'pass', record: loaded.path, sha256: loaded.sha256 };
  }
  if (command === 'verify-lock' && (tokens.length === 2 || tokens.length === 3)) {
    const stable = tokens.includes('--stable');
    const pathIndex = tokens.indexOf('--lock');
    if (pathIndex < 0 || !tokens[pathIndex + 1]) throw new Error('verify-lock requires --lock');
    const loaded = readRegularJson(tokens[pathIndex + 1], 'cohort lock');
    validateCohortLock(loaded.value, { stable });
    return { status: 'pass', lock: loaded.path, sha256: loaded.sha256, stable };
  }
  if (command === 'create') {
    const options = parseCreate(tokens);
    const records = options.records.map((path) => {
      const loaded = readRegularJson(path, 'platform record');
      return { record: loaded.value, sha256: loaded.sha256 };
    });
    const lock = buildCohortLock({
      cohortId: options.cohortId,
      sourceCommit: options.sourceCommit,
      inputDigests: options.inputs,
      records,
      stable: options.stable,
    });
    const output = writeExclusive(options.output, lock);
    return { status: 'pass', lock: output, sha256: sha256Bytes(readFileSync(output)) };
  }
  throw new Error(
    'usage: --self-test | verify-record --record <path> | verify-lock --lock <path> [--stable] | create --cohort <id> --source-commit <sha> --input key=digest --record <path>... --output <path> [--stable]',
  );
}

export function assertSelfTest() {
  const base = {
    schema_version: '1.0.0',
    cohort_id: 'cohort-test',
    source_commit: 'a'.repeat(40),
    requirement: 'required',
    status: 'pass',
    artifact_digests: { package: 'b'.repeat(64) },
    check_ids: ['native_product'],
  };
  const records = [];
  for (const [cellId, hostTarget] of Object.entries(REQUIRED_CELLS)) {
    for (const stage of STAGES) {
      records.push({
        record: { ...base, cell_id: cellId, host_target: hostTarget, stage },
        sha256: 'c'.repeat(64),
      });
    }
  }
  const lock = buildCohortLock({
    cohortId: base.cohort_id,
    sourceCommit: base.source_commit,
    inputDigests: { cargo_lock: 'd'.repeat(64) },
    records,
    stable: true,
  });
  validateCohortLock(lock, { stable: true });
  let rejected = false;
  try {
    buildCohortLock({
      cohortId: base.cohort_id,
      sourceCommit: base.source_commit,
      inputDigests: { cargo_lock: 'd'.repeat(64) },
      records: records.slice(0, -1),
      stable: true,
    });
  } catch (error) {
    rejected = error instanceof N1CohortEvidenceError && error.code === 'stable_required_record_missing';
  }
  if (!rejected) throw new Error('missing required signed RC record was not rejected');
  return { status: 'pass', checks: ['record-shape', 'stable-matrix', 'missing-required-rejection'] };
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  try {
    console.log(JSON.stringify(main(process.argv.slice(2)), null, 2));
  } catch (error) {
    console.error(JSON.stringify({
      status: 'fail',
      code: error instanceof N1CohortEvidenceError ? error.code : 'usage_or_io_error',
      reason: error instanceof Error ? error.message : String(error),
      ...(error instanceof N1CohortEvidenceError ? error.details : {}),
    }, null, 2));
    process.exitCode = 1;
  }
}
