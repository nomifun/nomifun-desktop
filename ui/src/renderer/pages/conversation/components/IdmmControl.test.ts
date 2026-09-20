/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import { defaultIdmmConfig } from './IdmmControl';

const source = readFileSync(new URL('./IdmmControl.tsx', import.meta.url), 'utf8');

describe('IDMM control defaults', () => {
  test('is opt-in and keeps both deterministic recovery paths enabled', () => {
    const config = defaultIdmmConfig();
    expect(config.mode).toBe('off');
    expect(config.recover_provider_failures).toBe(true);
    expect(config.recover_stalled_turns).toBe(true);
    expect(config.auto_select_options).toBe(true);
    expect(config.bypass_model.provider_id).toBeNull();
  });

  test('uses the bounded capability-panel layout without horizontal model overflow', () => {
    expect(source.includes("className='idmm-control-popover'")).toBe(true);
    expect(source.includes('maxHeight: \'min(500px, calc(100vh - 32px))\'')).toBe(true);
    expect(source.includes('overflow-x-hidden overflow-y-auto')).toBe(true);
    expect(source.includes("layout='stacked'")).toBe(true);
  });

  test('can render the same policy editor directly inside Agent Workbench', () => {
    expect(source.includes("presentation?: 'popover' | 'embedded'")).toBe(true);
    expect(source.includes('if (embedded) return panel')).toBe(true);
    expect(source.includes("embedded ? 'min-w-0'")).toBe(true);
  });
});
