/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';

import { createWorkflowFixture } from '../domain/testFixtures';
import CreativeWorkflowWorkspacePage from './CreativeWorkflowWorkspacePage';

describe('Creative Workflow workspace page', () => {
  test('renders the source list hierarchy with real canonical workflow data', () => {
    const workflow = createWorkflowFixture();
    const html = renderToStaticMarkup(
      <CreativeWorkflowWorkspacePage
        autoLoad={false}
        initialWorkflows={[workflow]}
      />
    );

    expect(html.includes('data-creative-workflow-workspace="true"')).toBe(true);
    expect(html.includes('创作工作流')).toBe(true);
    expect(html.includes('AI 创建')).toBe(true);
    expect(html.includes('新建多图')).toBe(true);
    expect(html.includes('新建工作流')).toBe(true);
    expect(html.includes(`data-workflow-id="${workflow.id}"`)).toBe(true);
    expect(html.includes(workflow.metadata.name)).toBe(true);
    expect(html.includes('运行')).toBe(true);
  });
});
