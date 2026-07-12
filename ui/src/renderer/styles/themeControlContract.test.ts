/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { PRESET_THEMES } from '@renderer/pages/settings/DisplaySettings/presets';

const controlCss = readFileSync(new URL('./theme-control-contract.css', import.meta.url), 'utf8');
const showcaseSource = readFileSync(new URL('../pages/TestShowcase.tsx', import.meta.url), 'utf8');
const assistantTagPickerSource = readFileSync(
  new URL('../pages/settings/AssistantSettings/AssistantTagPicker.tsx', import.meta.url),
  'utf8'
);
const knowledgeTagFilterSource = readFileSync(new URL('../pages/knowledge/KnowledgeTagFilterBar.tsx', import.meta.url), 'utf8');
const requirementSourceCardSource = readFileSync(
  new URL('../pages/requirements/SourcesPage/SourceCard.tsx', import.meta.url),
  'utf8'
);
const knowledgeEmptyStateSource = readFileSync(new URL('../pages/knowledge/KnowledgeEmptyState.tsx', import.meta.url), 'utf8');

const CONTROL_TOKENS = [
  '--control-selected-bg',
  '--control-selected-fg',
  '--control-idle-bg',
  '--control-idle-border',
  '--control-hover-bg',
  '--control-disabled-selected-bg',
  '--control-focus-ring',
];

describe('theme control contract', () => {
  test('every built-in theme supplies the full control palette in light and dark modes', () => {
    for (const theme of PRESET_THEMES) {
      for (const token of CONTROL_TOKENS) {
        expect(theme.css?.match(new RegExp(`${token}:`, 'g'))?.length).toBe(2);
      }
    }
  });

  test('centralizes the visual states that must remain readable across themes', () => {
    for (const selector of [
      '.arco-checkbox-mask',
      '.arco-checkbox-checked .arco-checkbox-mask',
      '.arco-radio-mask',
      '.arco-radio-checked .arco-radio-mask',
      '.arco-switch',
      '.arco-switch-checked',
      '.arco-tag-checkable.arco-tag-checked',
      '.arco-tabs-nav-tab-active',
      ':focus-visible',
      '[disabled]',
    ]) {
      expect(controlCss.includes(selector)).toBe(true);
    }

    expect(controlCss.includes('rgb(var(--primary-6))')).toBe(false);
  });

  test('keeps a visual regression matrix for the core interactive controls', () => {
    for (const component of ['<Checkbox', '<Radio', '<Switch', '<Tag', '<Tabs']) {
      expect(showcaseSource.includes(component)).toBe(true);
    }
  });

  test('uses the control palette for custom selected chips and selected source tags', () => {
    for (const source of [
      assistantTagPickerSource,
      knowledgeTagFilterSource,
      requirementSourceCardSource,
      knowledgeEmptyStateSource,
    ]) {
      expect(source.includes('--control-selected-bg')).toBe(true);
      expect(source.includes('--control-selected-fg')).toBe(true);
    }
  });
});
