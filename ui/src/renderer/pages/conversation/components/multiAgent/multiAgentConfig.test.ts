/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  defaultMultiAgentConfig,
  isMultiAgentConfigReady,
  normalizeMultiAgentConfig,
  type TMultiAgentConfig,
} from './multiAgentConfig';

describe('defaultMultiAgentConfig', () => {
  test('disabled, auto mode, empty roster', () => {
    expect(defaultMultiAgentConfig()).toEqual({ enabled: false, mode: 'auto', manual_agents: [] });
  });
  test('returns a fresh object each call (no shared roster reference)', () => {
    const a = defaultMultiAgentConfig();
    const b = defaultMultiAgentConfig();
    expect(a.manual_agents).not.toBe(b.manual_agents);
  });
});

describe('normalizeMultiAgentConfig', () => {
  test('undefined / null / non-object → default', () => {
    expect(normalizeMultiAgentConfig(undefined)).toEqual(defaultMultiAgentConfig());
    expect(normalizeMultiAgentConfig(null)).toEqual(defaultMultiAgentConfig());
    expect(normalizeMultiAgentConfig('x')).toEqual(defaultMultiAgentConfig());
    expect(normalizeMultiAgentConfig(42)).toEqual(defaultMultiAgentConfig());
  });

  test('enabled coerces strictly to boolean true', () => {
    expect(normalizeMultiAgentConfig({ enabled: true }).enabled).toBe(true);
    expect(normalizeMultiAgentConfig({ enabled: 'true' }).enabled).toBe(false);
    expect(normalizeMultiAgentConfig({ enabled: 1 }).enabled).toBe(false);
    expect(normalizeMultiAgentConfig({}).enabled).toBe(false);
  });

  test('invalid mode falls back to auto; valid modes preserved', () => {
    expect(normalizeMultiAgentConfig({ mode: 'manual' }).mode).toBe('manual');
    expect(normalizeMultiAgentConfig({ mode: 'auto' }).mode).toBe('auto');
    expect(normalizeMultiAgentConfig({ mode: 'bogus' }).mode).toBe('auto');
    expect(normalizeMultiAgentConfig({ mode: 123 }).mode).toBe('auto');
  });

  test('manual_agents: drops entries without a backend, keeps valid ones', () => {
    const cfg = normalizeMultiAgentConfig({
      mode: 'manual',
      manual_agents: [
        { backend: 'claude', model: 'claude-sonnet-4-5' },
        { backend: '   ', model: 'x' }, // blank backend → dropped
        { model: 'no-backend' }, // missing backend → dropped
        'garbage', // non-object → dropped
        { backend: 'nomi' }, // missing model → kept with empty model
      ],
    });
    expect(cfg.manual_agents).toEqual([
      { backend: 'claude', model: 'claude-sonnet-4-5' },
      { backend: 'nomi', model: '' },
    ]);
  });

  test('manual_agents trims backend and preserves optional name', () => {
    const cfg = normalizeMultiAgentConfig({
      mode: 'manual',
      manual_agents: [{ backend: '  gemini ', model: 'auto', name: 'Researcher' }],
    });
    expect(cfg.manual_agents).toEqual([{ backend: 'gemini', model: 'auto', name: 'Researcher' }]);
  });

  test('blank name is dropped (not persisted as empty string)', () => {
    const cfg = normalizeMultiAgentConfig({
      mode: 'manual',
      manual_agents: [{ backend: 'claude', model: 'm', name: '   ' }],
    });
    expect(cfg.manual_agents?.[0]).toEqual({ backend: 'claude', model: 'm' });
  });

  test('non-array manual_agents → empty array', () => {
    expect(normalizeMultiAgentConfig({ manual_agents: 'nope' }).manual_agents).toEqual([]);
    expect(normalizeMultiAgentConfig({ manual_agents: {} }).manual_agents).toEqual([]);
  });
});

describe('isMultiAgentConfigReady', () => {
  const cfg = (over: Partial<TMultiAgentConfig>): TMultiAgentConfig => ({
    ...defaultMultiAgentConfig(),
    ...over,
  });

  test('auto mode is always ready', () => {
    expect(isMultiAgentConfigReady(cfg({ mode: 'auto' }))).toBe(true);
    expect(isMultiAgentConfigReady(cfg({ mode: 'auto', manual_agents: [] }))).toBe(true);
  });

  test('manual mode with empty roster is not ready', () => {
    expect(isMultiAgentConfigReady(cfg({ mode: 'manual', manual_agents: [] }))).toBe(false);
    expect(isMultiAgentConfigReady(cfg({ mode: 'manual', manual_agents: undefined }))).toBe(false);
  });

  test('manual mode ready only when every agent has backend + model', () => {
    expect(
      isMultiAgentConfigReady(cfg({ mode: 'manual', manual_agents: [{ backend: 'claude', model: 'm' }] }))
    ).toBe(true);
    expect(
      isMultiAgentConfigReady(cfg({ mode: 'manual', manual_agents: [{ backend: 'claude', model: '' }] }))
    ).toBe(false);
    expect(
      isMultiAgentConfigReady(
        cfg({
          mode: 'manual',
          manual_agents: [
            { backend: 'claude', model: 'm' },
            { backend: 'nomi', model: '' },
          ],
        })
      )
    ).toBe(false);
  });
});
