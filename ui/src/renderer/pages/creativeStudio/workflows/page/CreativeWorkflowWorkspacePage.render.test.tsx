/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import { createWorkflowFixture } from '../domain/testFixtures';
import CreativeWorkflowWorkspacePage from './CreativeWorkflowWorkspacePage';

const css = readFileSync(
  new URL('./CreativeWorkflowWorkspacePage.module.css', import.meta.url),
  'utf8'
);
const testI18n = createInstance();
testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: {} } },
  interpolation: { escapeValue: false },
});

describe('Creative Workflow workspace page', () => {
  test('renders the source list hierarchy with real canonical workflow data', () => {
    const workflow = createWorkflowFixture();
    workflow.metadata.visibility = 'public';
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <CreativeWorkflowWorkspacePage
          autoLoad={false}
          initialWorkflows={[workflow]}
        />
      </I18nextProvider>
    );

    expect(html.includes('data-creative-workflow-workspace="true"')).toBe(true);
    expect(html.includes('Template Studio')).toBe(true);
    expect(html.includes('Create with AI')).toBe(true);
    expect(html.includes('New multi-image template')).toBe(true);
    expect(html.includes('New template')).toBe(true);
    expect(html.includes('Creative workflow')).toBe(false);
    expect(html.includes(`data-workflow-id="${workflow.id}"`)).toBe(true);
    expect(html.includes(workflow.metadata.name)).toBe(true);
    expect(html.includes('Run')).toBe(true);
    expect(html.includes('Public')).toBe(false);
    expect(html.includes('Private')).toBe(false);
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
