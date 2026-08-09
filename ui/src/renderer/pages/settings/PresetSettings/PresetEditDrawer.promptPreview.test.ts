/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const drawerSource = readFileSync(new URL('./PresetEditDrawer.tsx', import.meta.url), 'utf8');

describe('PresetEditDrawer prompt preview typography', () => {
  test('removes skill metadata before rendering the compact Markdown preview', () => {
    expect(drawerSource.includes("import { stripSkillFrontmatter }")).toBe(true);
    expect(
      drawerSource.includes(
        'const promptPreviewContent = useMemo(() => stripSkillFrontmatter(editContext).trim(), [editContext]);'
      )
    ).toBe(true);
    expect(drawerSource.includes('<MarkdownView hiddenCodeCopyButton compact>')).toBe(true);
    expect(
      drawerSource.match(
        /<MarkdownView[^>]*compact[^>]*>[\s\S]*?\{promptPreviewContent\}[\s\S]*?<\/MarkdownView>/
      )
    ).not.toBeNull();
    expect(drawerSource.includes('{editContext}</MarkdownView>')).toBe(false);
  });
});
