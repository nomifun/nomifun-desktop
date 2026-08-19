/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * The shared URL-join contract, asserted from the TypeScript side.
 *
 * `crates/backend/nomifun-model-invoke/tests/url_join_contract.rs` asserts the
 * same fixture against the Rust original. A live per-keystroke preview cannot
 * round-trip to the backend, so `joinEndpointUrl` is a deliberate second
 * implementation and this fixture is what stops the two from drifting.
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

import { joinEndpointUrl, rootDeclaresVersion, rootMatchesShape } from './providerModelAdvanced';

interface JoinCase {
  why: string;
  base: string;
  endpoint: string;
  expected: string;
}

const fixture = JSON.parse(
  readFileSync(
    new URL(
      '../../../../../../crates/backend/nomifun-model-invoke/tests/fixtures/url_join_cases.json',
      import.meta.url
    ),
    'utf8'
  )
) as { cases: JoinCase[] };

describe('joinEndpointUrl mirrors the Rust url_algebra contract', () => {
  test('the shared fixture is actually loaded', () => {
    expect(fixture.cases.length).toBeGreaterThan(10);
  });

  for (const testCase of fixture.cases) {
    test(`${testCase.why} [${testCase.base} + ${testCase.endpoint}]`, () => {
      expect(joinEndpointUrl(testCase.base, testCase.endpoint)).toBe(testCase.expected);
    });
  }
});

describe('root shape classification', () => {
  test('a version anywhere in the path counts, not just the last segment', () => {
    expect(rootDeclaresVersion('https://api.openai.com/v1')).toBe(true);
    expect(rootDeclaresVersion('https://qianfan.baidubce.com/v2/coding')).toBe(true);
    expect(rootDeclaresVersion('https://open.bigmodel.cn/api/paas/v4')).toBe(true);
    expect(rootDeclaresVersion('https://api.anthropic.com')).toBe(false);
    expect(rootDeclaresVersion('https://api.example.com/videos')).toBe(false);
  });

  test('the two conventions are exact complements', () => {
    expect(rootMatchesShape('https://api.openai.com/v1', 'versioned_root')).toBe(true);
    expect(rootMatchesShape('https://api.openai.com', 'versioned_root')).toBe(false);
    expect(rootMatchesShape('https://api.anthropic.com', 'origin_root')).toBe(true);
    expect(rootMatchesShape('https://api.anthropic.com/v1', 'origin_root')).toBe(false);
  });

  test('an unparseable base URL declares no version rather than throwing', () => {
    expect(rootDeclaresVersion('not a url')).toBe(false);
    expect(rootDeclaresVersion('')).toBe(false);
  });
});
