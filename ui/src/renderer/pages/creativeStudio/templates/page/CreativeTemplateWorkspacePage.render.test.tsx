/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { within } from '@testing-library/dom';
import { describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import { createTemplateFixture, createTemplateRunFixture } from '../domain/testFixtures';
import CreativeTemplateWorkspacePage from './CreativeTemplateWorkspacePage';

const css = readFileSync(
  new URL('./CreativeTemplateWorkspacePage.module.css', import.meta.url),
  'utf8'
);
const dialogCss = readFileSync(
  new URL('./TemplateWorkspaceDialogs.module.css', import.meta.url),
  'utf8'
);
const testI18n = createInstance();
testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: {} } },
  interpolation: { escapeValue: false },
});

describe('Creative Template workspace page', () => {
  test('renders the source list hierarchy with real canonical template data', () => {
    const template = createTemplateFixture();
    template.metadata.visibility = 'public';
    template.metadata.tags = ['Campaign photography', 'Reusable-commercial-studio-template-with-a-deliberately-long-tag'];
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <CreativeTemplateWorkspacePage
          autoLoad={false}
          initialTemplates={[template]}
        />
      </I18nextProvider>
    );

    expect(html.includes('data-creative-template-workspace')).toBe(true);
    expect(html.includes('Template Studio')).toBe(true);
    expect(html.includes('Create with AI')).toBe(true);
    expect(html.includes('New multi-image template')).toBe(true);
    expect(html.includes('New template')).toBe(true);
    expect(html.includes('Creative template')).toBe(false);
    expect(html.includes(`data-template-id="${template.id}"`)).toBe(true);
    expect(html.includes(template.metadata.name)).toBe(true);
    expect(html.includes('Run')).toBe(true);
    expect(html.includes('aria-label="Template type"')).toBe(true);
    expect(html.includes('aria-pressed="true"')).toBe(true);
    const container = document.createElement('div');
    container.innerHTML = html;
    const card = container.querySelector('article[data-template-id]')! as HTMLElement;
    const cardQueries = within(card);
    expect(cardQueries.getByRole('heading', { name: template.metadata.name })).not.toBeNull();
    expect(cardQueries.getByText(template.metadata.description)).not.toBeNull();
    expect(cardQueries.getByText(template.metadata.category)).not.toBeNull();
    expect(cardQueries.getByText('Single image')).not.toBeNull();
    for (const tag of template.metadata.tags) {
      expect(cardQueries.getByText(tag)).not.toBeNull();
    }
    for (const name of ['Run', 'Edit Product poster', 'Duplicate Product poster', 'Delete Product poster']) {
      expect(cardQueries.getByRole('button', { name })).not.toBeNull();
    }
    expect(card.textContent).not.toContain('Prompt preview');
    expect(card.textContent).not.toContain('Create a poster for');
    expect(card.textContent).not.toContain('2 variables');
    expect(card.textContent).not.toContain('Updated');
    for (const variable of template.variables) {
      expect(card.textContent).not.toContain(variable.label);
    }
    expect(html.includes('Public')).toBe(false);
    expect(html.includes('Private')).toBe(false);
  });

  test('uses shared theme tokens and keeps dialog styles in their own module', () => {
    for (const source of [css, dialogCss]) {
      expect(source).not.toMatch(/--(?:brand|primary|primary-rgb|color-[\w-]+|bg-[\w-]+|dialog-fill-0|nomi-modal-control-bg)\s*:/);
      expect(source.includes('var(--brand)')).toBe(true);
      expect(source.includes('var(--bg-base)')).toBe(true);
      expect(source.includes('[data-theme=')).toBe(false);
    }
    expect(css.includes('var(--color-bg-2)')).toBe(true);
    expect(css.includes('var(--color-border-2)')).toBe(true);
    for (const selector of ['.editorModal', '.runModal', '.reviewModal']) {
      expect(css.includes(selector)).toBe(false);
      expect(dialogCss.includes(selector)).toBe(true);
    }
    expect(dialogCss.includes('var(--nomi-modal-max-height)')).toBe(true);
  });

  test.each([false, true])('keeps the task entry after template creation in the header when history exists: %s', (hasHistory) => {
    const run = createTemplateRunFixture();
    run.templateSnapshot.metadata.name = 'Historical template run';
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={testI18n}>
        <CreativeTemplateWorkspacePage
          autoLoad={false}
          initialTemplates={[createTemplateFixture()]}
          runCenter={{
            snapshot: { loading: false, loadError: null, runs: hasHistory ? [run] : [], activities: {} },
            assetUrl: () => '',
            resume: async () => undefined,
            cancel: async () => undefined,
            review: async () => undefined,
            retry: async () => undefined,
          }}
        />
      </I18nextProvider>
    );
    const container = document.createElement('div');
    container.innerHTML = html;
    const createButton = within(container).getByRole('button', { name: 'New template' });
    const actions = createButton.parentElement!;
    const taskEntry = container.querySelector('[data-template-run-center]');

    expect(taskEntry).not.toBeNull();
    expect(actions.lastElementChild).toBe(taskEntry);
    expect(createButton.nextElementSibling).toBe(taskEntry);
    expect(container.querySelectorAll('[data-template-run-center]')).toHaveLength(1);
    expect(container.textContent).not.toContain('Historical template run');
  });
});
