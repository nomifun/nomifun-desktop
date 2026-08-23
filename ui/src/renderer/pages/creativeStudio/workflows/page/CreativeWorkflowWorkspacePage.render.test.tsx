/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';

import { createWorkflowFixture } from '../domain/testFixtures';
import CreativeWorkflowWorkspacePage from './CreativeWorkflowWorkspacePage';

const css = readFileSync(
  new URL('./CreativeWorkflowWorkspacePage.module.css', import.meta.url),
  'utf8'
);

describe('Creative Workflow workspace page', () => {
  test('renders the source list hierarchy with real canonical workflow data', () => {
    const workflow = createWorkflowFixture();
    workflow.metadata.visibility = 'public';
    const html = renderToStaticMarkup(
      <CreativeWorkflowWorkspacePage
        autoLoad={false}
        initialWorkflows={[workflow]}
      />
    );

    expect(html.includes('data-creative-workflow-workspace="true"')).toBe(true);
    expect(html.includes('模板工作台')).toBe(true);
    expect(html.includes('AI 创建')).toBe(true);
    expect(html.includes('新建多图模板')).toBe(true);
    expect(html.includes('新建模板')).toBe(true);
    expect(html.includes('创作工作流')).toBe(false);
    expect(html.includes(`data-workflow-id="${workflow.id}"`)).toBe(true);
    expect(html.includes(workflow.metadata.name)).toBe(true);
    expect(html.includes('运行')).toBe(true);
    expect(html.includes('公开')).toBe(false);
    expect(html.includes('个人')).toBe(false);
  });

  test('keeps the focused workflow surface on a fixed light stone palette', () => {
    expect(css.includes('--color-bg-1: #f4f2ed')).toBe(true);
    expect(css.includes('--dialog-fill-0: #f4f2ed')).toBe(true);
    expect(css.includes('--nomi-modal-control-bg: #ffffff')).toBe(true);
    expect(css.includes('--primary-6: 87, 83, 78')).toBe(true);
    expect(css.includes('color-scheme: light')).toBe(true);
    expect(css.includes('.editorModal')).toBe(true);
    expect(css.includes('.runModal')).toBe(true);
    expect(css.includes('.reviewModal')).toBe(true);
  });
});
