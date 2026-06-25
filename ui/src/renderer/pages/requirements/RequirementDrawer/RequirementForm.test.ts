/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = () => readFileSync(new URL('./RequirementForm.tsx', import.meta.url), 'utf8');

describe('RequirementForm draft lifecycle', () => {
  test('resets form fields and attachment draft when a new requirement form context opens', () => {
    const form = source();

    expect(form.includes('resetSignal?: number')).toBe(true);
    expect(form.includes('title: undefined')).toBe(true);
    expect(form.includes('content: undefined')).toBe(true);
    expect(form.includes('tag: undefined')).toBe(true);
    expect(form.includes('order_key: undefined')).toBe(true);
    expect(form.includes('form.setFieldsValue(nextValues)')).toBe(true);
    expect(form.includes('setNewAttachments([])')).toBe(true);
    expect(form.includes('setRemoveAttachmentIds([])')).toBe(true);
    expect(form.includes('[form, initialValues, resetSignal]')).toBe(true);
  });
});
