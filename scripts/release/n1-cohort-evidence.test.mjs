import { describe, expect, test } from 'bun:test';

import {
  N1CohortEvidenceError,
  assertSelfTest,
  buildCohortLock,
  validatePlatformRecord,
} from './n1-cohort-evidence.mjs';

const record = (cell_id, host_target, stage) => ({
  schema_version: '1.0.0',
  cohort_id: 'cohort-test',
  source_commit: 'a'.repeat(40),
  stage,
  cell_id,
  requirement: 'required',
  host_target,
  status: 'pass',
  artifact_digests: { package: 'b'.repeat(64) },
  check_ids: ['native_product'],
});

describe('N1/M1 cohort evidence', () => {
  test('validates a required native record', () => {
    expect(
      validatePlatformRecord(record('windows_desktop_x64', 'x86_64-pc-windows-msvc', 'candidate'))
        .status,
    ).toBe('pass');
  });

  test('rejects stable promotion with any required stage missing', () => {
    expect(() =>
      buildCohortLock({
        cohortId: 'cohort-test',
        sourceCommit: 'a'.repeat(40),
        inputDigests: { cargo_lock: 'c'.repeat(64) },
        records: [
          {
            record: record('windows_desktop_x64', 'x86_64-pc-windows-msvc', 'candidate'),
            sha256: 'd'.repeat(64),
          },
        ],
        stable: true,
      }),
    ).toThrow();
    try {
      buildCohortLock({
        cohortId: 'cohort-test',
        sourceCommit: 'a'.repeat(40),
        inputDigests: { cargo_lock: 'c'.repeat(64) },
        records: [],
        stable: true,
      });
    } catch (error) {
      expect(error instanceof N1CohortEvidenceError).toBe(true);
      expect(error.code).toBe('stable_required_record_missing');
    }
  });

  test('runs its complete matrix self-test', () => {
    expect(assertSelfTest().status).toBe('pass');
  });
});
