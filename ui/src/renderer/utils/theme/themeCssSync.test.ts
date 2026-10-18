/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ICssTheme } from '@/common/config/storage';
import {
  BACKGROUND_BLOCK_END,
  BACKGROUND_BLOCK_START,
  backgroundCssBlockNeedsUpgrade,
} from '@renderer/pages/settings/DisplaySettings/backgroundUtils';
import { DEFAULT_THEME_ID, PRESET_THEMES } from '@renderer/pages/settings/DisplaySettings/presets';
import { computeCssSyncDecision, normalizeCssTheme, resolveCssThemeState } from './themeCssSync';

const classic = PRESET_THEMES.find((theme) => theme.id === DEFAULT_THEME_ID)!;
const frosted = PRESET_THEMES.find((theme) => theme.id === 'frosted-glass')!;
const image = 'data:image/png;base64,AAAA';
const customTheme = (overrides: Partial<ICssTheme> = {}): ICssTheme => ({
  id: '5b8e7860-5bd6-4d81-854e-1f162fb57231',
  name: '我的自定义主题',
  css: ':root { --color-primary: #123456; }',
  ...overrides,
});

describe('canonical CSS theme state', () => {
  test.each(['', 'missing-user-theme', 'rhythm-dark', 'neon-rainbow', 'sunset-afterglow'])(
    'replaces unavailable selection %j and stale CSS with Classic',
    (activeThemeId) => {
      const decision = computeCssSyncDecision({
        activeThemeId,
        savedCss: 'body { color: hotpink; }',
        savedThemes: [],
      });

      expect(decision.activeThemeId).toBe(DEFAULT_THEME_ID);
      expect(decision.effectiveCss).toBe(classic.css);
      expect(decision.configUpdates).toEqual({
        'css.activeThemeId': DEFAULT_THEME_ID,
        customCss: classic.css,
      });
    }
  );

  test('removes saved preset records so retired built-ins cannot return', () => {
    const user = customTheme();
    const stalePreset = customTheme({ id: 'rhythm-dark', name: '退役内置主题', is_preset: true });
    const decision = computeCssSyncDecision({
      activeThemeId: stalePreset.id,
      savedCss: stalePreset.css,
      savedThemes: [stalePreset, { ...frosted }, user],
    });

    expect(decision.themes).toEqual([...PRESET_THEMES, user]);
    expect(decision.activeThemeId).toBe(DEFAULT_THEME_ID);
    expect(decision.configUpdates).toEqual({
      'css.activeThemeId': DEFAULT_THEME_ID,
      customCss: classic.css,
      'css.themes': [user],
    });
  });

  test('keeps Frosted Glass and genuine user themes selected', () => {
    const user = customTheme({ name: '我的律动副本' });
    const savedThemes = [user];

    for (const theme of [frosted, user]) {
      const state = resolveCssThemeState(theme.id, savedThemes);
      const decision = computeCssSyncDecision({ activeThemeId: theme.id, savedCss: theme.css, savedThemes });

      expect(state.activeTheme).toBe(theme);
      expect(state.userThemes).toEqual([user]);
      expect(decision.activeThemeId).toBe(theme.id);
      expect(decision.effectiveCss).toBe(theme.css);
      expect(decision.configUpdates).toEqual({});
    }
    expect(resolveCssThemeState(user.id, savedThemes).activeTheme.name).toBe('我的律动副本');
  });

  test('uses catalog built-ins and the first user record for duplicate IDs', () => {
    const user = customTheme();
    const duplicate = { ...user, name: '重复记录', css: 'body { color: red; }' };
    const collision = customTheme({ id: DEFAULT_THEME_ID, name: '覆盖经典', css: 'body { color: blue; }' });
    const savedThemes = [collision, user, duplicate, customTheme({ id: '' })];
    const state = resolveCssThemeState(DEFAULT_THEME_ID, savedThemes);
    const decision = computeCssSyncDecision({ activeThemeId: user.id, savedCss: user.css, savedThemes });

    expect(state.activeTheme).toBe(classic);
    expect(state.themes).toEqual([...PRESET_THEMES, user]);
    expect(state.userThemes[0]).toBe(user);
    expect(decision.configUpdates).toEqual({ 'css.themes': [user] });
  });

  test('keeps an intentionally empty custom theme instead of replacing it with Classic', () => {
    const user = customTheme({ css: '' });
    const decision = computeCssSyncDecision({ activeThemeId: user.id, savedCss: '', savedThemes: [user] });

    expect(decision.activeThemeId).toBe(user.id);
    expect(decision.effectiveCss).toBe('');
    expect(decision.configUpdates).toEqual({});
  });
});

describe('CSS theme background normalization', () => {
  test('builds image CSS from an empty theme once without mutating the saved theme', () => {
    const saved = customTheme({ css: '', cover: image });
    const normalized = normalizeCssTheme(saved);

    expect(saved.css).toBe('');
    expect(normalized.css).toContain(`url("${image}")`);
    expect(normalized.css.split(BACKGROUND_BLOCK_START)).toHaveLength(2);
    expect(backgroundCssBlockNeedsUpgrade(normalized.css)).toBe(false);
    expect(normalizeCssTheme(normalized)).toBe(normalized);
    expect(resolveCssThemeState(saved.id, [saved]).activeTheme.css).toBe(normalized.css);
  });

  test('replaces a v1 background block once and persists the same normalized CSS', () => {
    const saved = customTheme({
      cover: image,
      css: `:root { --color-primary: #123456; }\n${BACKGROUND_BLOCK_START}\nbody { background-image: url("${image}"); }\n${BACKGROUND_BLOCK_END}`,
    });
    const normalized = normalizeCssTheme(saved);
    const decision = computeCssSyncDecision({ activeThemeId: saved.id, savedCss: saved.css, savedThemes: [saved] });

    expect(backgroundCssBlockNeedsUpgrade(saved.css)).toBe(true);
    expect(backgroundCssBlockNeedsUpgrade(normalized.css)).toBe(false);
    expect(normalized.css).toContain(':root { --color-primary: #123456; }');
    expect(normalized.css.split(BACKGROUND_BLOCK_START)).toHaveLength(2);
    expect(normalized.css.split(`url("${image}")`)).toHaveLength(2);
    expect(normalizeCssTheme(normalized)).toBe(normalized);
    expect(decision.configUpdates).toEqual({ customCss: normalized.css, 'css.themes': [normalized] });
    expect(computeCssSyncDecision({
      activeThemeId: normalized.id,
      savedCss: normalized.css,
      savedThemes: [normalized],
    }).configUpdates).toEqual({});
  });
});

describe('CSS theme synchronization decisions', () => {
  test('does not rewrite an already canonical state', () => {
    const decision = computeCssSyncDecision({ activeThemeId: classic.id, savedCss: classic.css, savedThemes: [] });

    expect(decision.configUpdates).toEqual({});
    expect(decision.shouldSkipApply).toBe(false);
    expect(decision.effectiveCss).toBe(classic.css);
  });

  test('keeps a recent UI update while persisted state is internally consistent', () => {
    const recentCss = 'body { color: blue; }';
    const decision = computeCssSyncDecision({
      activeThemeId: classic.id,
      savedCss: classic.css,
      savedThemes: [],
      currentUiCss: recentCss,
      lastUiCssUpdateAt: 1000,
      now: 2999,
    });

    expect(decision.configUpdates).toEqual({});
    expect(decision.shouldSkipApply).toBe(true);
    expect(decision.effectiveCss).toBe(recentCss);
  });

  test('applies persisted CSS once the recent update window expires', () => {
    const decision = computeCssSyncDecision({
      activeThemeId: classic.id,
      savedCss: classic.css,
      savedThemes: [],
      currentUiCss: 'body { color: blue; }',
      lastUiCssUpdateAt: 1000,
      now: 3000,
    });

    expect(decision.shouldSkipApply).toBe(false);
    expect(decision.effectiveCss).toBe(classic.css);
  });

  test('repairs an unavailable selection even during a recent UI update', () => {
    const decision = computeCssSyncDecision({
      activeThemeId: 'rhythm-dark',
      savedCss: classic.css,
      savedThemes: [],
      currentUiCss: 'body { color: blue; }',
      lastUiCssUpdateAt: 1000,
      now: 1001,
    });

    expect(decision.shouldSkipApply).toBe(false);
    expect(decision.effectiveCss).toBe(classic.css);
    expect(decision.configUpdates).toEqual({ 'css.activeThemeId': DEFAULT_THEME_ID });
  });

  test('repairs stale CSS even during a recent UI update', () => {
    const staleCss = 'body { color: blue; }';
    const decision = computeCssSyncDecision({
      activeThemeId: classic.id,
      savedCss: staleCss,
      savedThemes: [],
      currentUiCss: staleCss,
      lastUiCssUpdateAt: 1000,
      now: 1001,
    });

    expect(decision.shouldSkipApply).toBe(false);
    expect(decision.effectiveCss).toBe(classic.css);
    expect(decision.configUpdates).toEqual({ customCss: classic.css });
  });
});
