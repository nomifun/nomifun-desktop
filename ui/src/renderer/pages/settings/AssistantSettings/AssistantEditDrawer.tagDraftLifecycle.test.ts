/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = (url: URL) => readFileSync(url, 'utf8');

describe('AssistantEditDrawer unfinished tag drafts', () => {
  test('clears both drafts for saves and every drawer close path without flushing them', () => {
    const drawer = source(new URL('./AssistantEditDrawer.tsx', import.meta.url));
    const picker = source(new URL('./AssistantTagPicker.tsx', import.meta.url));

    expect(drawer.includes('type AssistantTagPickerHandle')).toBe(true);
    expect(drawer.includes('const audiencePickerRef = useRef<AssistantTagPickerHandle>(null);')).toBe(true);
    expect(drawer.includes('const scenarioPickerRef = useRef<AssistantTagPickerHandle>(null);')).toBe(true);
    expect(drawer.includes('createAssistantTagDraftLifecycle')).toBe(true);
    expect(drawer.includes('const { resetPendingTagDrafts, closeDrawer, handleDrawerSave } = useMemo(')).toBe(true);
    expect(drawer.includes('if (!editVisible) {')).toBe(true);
    expect(drawer.includes('onCancel={closeDrawer}')).toBe(true);
    expect(drawer.includes('onClick={handleDrawerSave}')).toBe(true);
    expect(drawer.includes('onClick={closeDrawer}')).toBe(true);
    expect(drawer.includes('flushPendingTag')).toBe(false);

    expect(picker.includes('showAddHint?: boolean;')).toBe(true);
    expect(picker.includes('showAddHint = false')).toBe(true);
    expect(picker.includes('{showAddHint &&')).toBe(true);
    expect(drawer.includes('showAddHint')).toBe(true);
  });
});
