/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const controls = readFileSync(new URL('./CreationControls.tsx', import.meta.url), 'utf8');
const resources = readFileSync(new URL('./CreationResourceDialog.tsx', import.meta.url), 'utf8');
const promptPicker = readFileSync(
  new URL('../pages/creativeStudio/components/CreativePromptPicker.tsx', import.meta.url),
  'utf8'
);
const resourceDialog = readFileSync(
  new URL('../pages/creativeStudio/components/CreativeResourceDialog.tsx', import.meta.url),
  'utf8'
);

describe('creation parameter resources', () => {
  test('opens prompts and the template workspace as dialogs without leaving the draft', () => {
    expect(controls.includes("openResourceDialog('prompts')")).toBe(true);
    expect(controls.includes("openResourceDialog('templates')")).toBe(true);
    expect(controls.includes("navigate('/asset-library/templates')")).toBe(false);
    expect(resources.includes('<CreativePromptPicker')).toBe(true);
    expect(promptPicker.includes('<PromptLibrarySidebar')).toBe(true);
    expect(resources.includes('<CreativeTemplateRoute />')).toBe(true);
    expect(resources.includes("scope='conversation'")).toBe(true);
    expect(resourceDialog.includes('data-creation-resource-dialog')).toBe(true);
  });

  test('keeps the prompt chooser scoped to image and video creation', () => {
    expect(controls.includes("mode !== 'music' && <button")).toBe(true);
    expect(controls.includes('>提示词</button>')).toBe(true);
  });
});
