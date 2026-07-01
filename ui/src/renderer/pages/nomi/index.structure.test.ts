/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

describe('Nomi companion tab order', () => {
  test('places remote connection immediately after overview', () => {
    const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
    const registry = source.match(/const COMPANION_TABS = \[(.*?)\] as const;/s)?.[1] ?? '';

    expect(registry.indexOf("'overview'")).toBeLessThan(registry.indexOf("'remote'"));
    expect(registry.indexOf("'remote'")).toBeLessThan(registry.indexOf("'memories'"));

    const overviewRadio = source.indexOf("<Radio value='overview'>");
    const remoteRadio = source.indexOf("<Radio value='remote'>");
    const memoriesRadio = source.indexOf("<Radio value='memories'>");

    expect(overviewRadio).toBeGreaterThan(-1);
    expect(remoteRadio).toBeGreaterThan(overviewRadio);
    expect(memoriesRadio).toBeGreaterThan(remoteRadio);
  });
});
