/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';
import { Message } from '@arco-design/web-react';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';

import { ipcBridge } from '@/common';

import { cloneTemplateDefinition, type CreativeTemplateDefinitionV1 } from '../domain';
import { createExecutableTemplateFixture, createTemplateFixture } from '../domain/testFixtures';
import type { CreativeTemplateRepository } from '../services';
import CreativeTemplateWorkspacePage, { type CreativeTemplateWorkspacePageProps } from './CreativeTemplateWorkspacePage';
import type { CreativeTemplateRunnerPort } from './TemplateRunModal';

let restoreHarness: () => void;
beforeEach(() => {
  // Toast rendering and provider transport are app bootstrap concerns. Keep
  // real workspace/dialog controls while isolating them from a live backend.
  const success = spyOn(Message, 'success').mockReturnValue(() => {});
  const error = spyOn(Message, 'error').mockReturnValue(() => {});
  const providers = spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([]);
  const changes = spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockReturnValue(() => {});
  const reconnects = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  restoreHarness = () => {
    success.mockRestore();
    error.mockRestore();
    providers.mockRestore();
    changes.mockRestore();
    reconnects.mockRestore();
  };
});
afterEach(() => { cleanup(); restoreHarness(); });

const testI18n = createInstance();
testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: {} } },
  interpolation: { escapeValue: false },
});

function memoryRepository(initial: readonly CreativeTemplateDefinitionV1[]): CreativeTemplateRepository {
  const records = new Map(initial.map((template) => [template.id, cloneTemplateDefinition(template)]));
  return {
    list: async () => [...records.values()].map(cloneTemplateDefinition),
    load: async (id) => cloneTemplateDefinition(records.get(id)!),
    create: async (template) => {
      records.set(template.id, cloneTemplateDefinition(template));
      return cloneTemplateDefinition(template);
    },
    save: async (id, expectedRevision, template) => {
      expect(records.get(id)?.revision).toBe(expectedRevision);
      records.set(id, cloneTemplateDefinition(template));
      return cloneTemplateDefinition(template);
    },
    remove: async (id) => { records.delete(id); },
  };
}

const renderWorkspace = (templates: readonly CreativeTemplateDefinitionV1[], props: Partial<CreativeTemplateWorkspacePageProps> = {}) =>
  render(
    <I18nextProvider i18n={testI18n}>
      <SWRConfig value={{ provider: () => new Map(), fallback: { providers: [] }, revalidateOnMount: false }}>
        <CreativeTemplateWorkspacePage autoLoad={false} initialTemplates={templates} repository={memoryRepository(templates)} {...props} />
      </SWRConfig>
    </I18nextProvider>
  );

const visibleNames = (container: HTMLElement) =>
  [...container.querySelectorAll('article[data-template-id] h2')].map((heading) => heading.textContent);

describe('Creative Template workspace interactions', () => {
  test('combines kind, category, and search filters and clears a no-results state', async () => {
    const poster = createTemplateFixture();
    const series = createTemplateFixture(true);
    series.id = '018f0000-0000-7000-8000-000000000021';
    series.metadata.name = 'Summer campaign';
    series.metadata.tags = ['summer'];
    const portrait = createTemplateFixture();
    portrait.id = '018f0000-0000-7000-8000-000000000022';
    portrait.metadata.name = 'Summer portrait';
    portrait.metadata.category = 'Photography';
    const view = renderWorkspace([poster, series, portrait]);

    fireEvent.click(view.getByRole('button', { name: 'Single image' }));
    expect(visibleNames(view.container).sort()).toEqual(['Product poster', 'Summer portrait']);
    expect(view.getByRole('button', { name: 'Single image' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(view.getByRole('button', { name: 'Image series' }));
    expect(visibleNames(view.container)).toEqual(['Summer campaign']);
    fireEvent.click(view.getByRole('button', { name: 'All templates' }));

    fireEvent.change(view.getByRole('textbox', { name: 'Search names, categories, and descriptions' }), { target: { value: ' SUMMER ' } });
    fireEvent.click(view.getByRole('combobox', { name: 'All categories' }));
    fireEvent.click(await view.findByRole('option', { name: 'Marketing' }));
    expect(visibleNames(view.container)).toEqual(['Summer campaign']);
    expect(view.getByRole('status').textContent).toBe('1 of 3 templates');

    fireEvent.click(view.getByRole('button', { name: 'Single image' }));
    expect(view.getByRole('heading', { name: 'No matching templates' })).not.toBeNull();
    const emptyState = view.getByRole('heading', { name: 'No matching templates' }).parentElement!;
    fireEvent.click(within(emptyState).getByRole('button', { name: 'Clear filters' }));
    expect(visibleNames(view.container)).toHaveLength(3);
    expect((view.getByRole('textbox', { name: 'Search names, categories, and descriptions' }) as HTMLInputElement).value).toBe('');
    expect(view.getByRole('button', { name: 'All templates' }).getAttribute('aria-pressed')).toBe('true');
    expect(view.getByRole('status').textContent).toBe('3 of 3 templates');
  });

  test('duplicates privately and disables card mutations until persistence completes', async () => {
    const template = createTemplateFixture();
    template.metadata.visibility = 'public';
    const repository = memoryRepository([template]);
    let created: CreativeTemplateDefinitionV1 | undefined;
    let release: (() => void) | undefined;
    let calls = 0;
    repository.create = async (definition) => {
      calls += 1;
      created = definition;
      await new Promise<void>((resolve) => { release = resolve; });
      return cloneTemplateDefinition(definition);
    };
    const view = renderWorkspace([template], { repository });
    const article = view.container.querySelector('article[data-template-id]')! as HTMLElement;
    const card = within(article);
    fireEvent.click(card.getByRole('button', { name: 'Duplicate Product poster' }));

    expect(calls).toBe(1);
    expect(created?.metadata.visibility).toBe('private');
    expect(created?.id).not.toBe(template.id);
    for (const button of card.getAllByRole('button')) expect((button as HTMLButtonElement).disabled).toBe(true);
    expect((view.getByRole('button', { name: 'New template' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(card.getByRole('button', { name: 'Duplicate Product poster' }));
    expect(calls).toBe(1);

    await act(async () => { release!(); });
    await waitFor(() => expect(visibleNames(view.container)).toHaveLength(2));
    expect(visibleNames(view.container)).toContain(created!.metadata.name);
    expect(template.metadata.visibility).toBe('public');
    expect((card.getByRole('button', { name: 'Run' }) as HTMLButtonElement).disabled).toBe(false);
  });

  test('edits a template through its card and deletes it only after confirmation', async () => {
    const template = createTemplateFixture();
    const repository = memoryRepository([template]);
    const view = renderWorkspace([template], { repository });
    fireEvent.click(view.getByRole('button', { name: 'Edit Product poster' }));
    const editor = within(await view.findByRole('dialog', { name: 'Edit template' }));
    fireEvent.change(editor.getByPlaceholderText('Template name'), { target: { value: 'Updated poster' } });
    expect((await repository.load(template.id)).metadata.name).toBe('Product poster');
    fireEvent.click(editor.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(visibleNames(view.container)).toEqual(['Updated poster']));
    expect((await repository.load(template.id)).revision).toBe(2);
    expect(template.metadata.name).toBe('Product poster');

    fireEvent.click(view.getByRole('button', { name: 'Delete Updated poster' }));
    const confirm = within(await view.findByRole('dialog', { name: 'Delete template' }));
    expect(await repository.list()).toHaveLength(1);
    fireEvent.click(confirm.getByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(view.getByRole('heading', { name: 'No templates yet' })).not.toBeNull());
    expect(await repository.list()).toHaveLength(0);
  });

  test('changes a variable type through the visible selector and saves its typed default', async () => {
    const template = createTemplateFixture();
    const originalVariable = template.variables[0];
    const repository = memoryRepository([template]);
    const view = renderWorkspace([template], { repository });
    fireEvent.click(view.getByRole('button', { name: 'Edit Product poster' }));
    const editor = within(await view.findByRole('dialog', { name: 'Edit template' }));
    fireEvent.click(editor.getByRole('button', { name: /^Product name/ }));

    const typeSelector = editor.getByRole('combobox', { name: 'Variable type' });
    fireEvent.click(within(typeSelector).getByText('Single-line text'));
    expect(typeSelector.getAttribute('aria-expanded')).toBe('true');
    fireEvent.click(await view.findByRole('option', { name: 'Long text' }));
    expect(editor.getByRole('textbox', { name: 'Default long text (optional)' }).tagName).toBe('TEXTAREA');
    expect(editor.queryByRole('textbox', { name: 'Default value (optional)' })).toBeNull();

    fireEvent.click(within(typeSelector).getByText('Long text'));
    fireEvent.click(await view.findByRole('option', { name: 'Number' }));
    expect(editor.queryByRole('textbox', { name: 'Default long text (optional)' })).toBeNull();
    expect(editor.getByPlaceholderText('Minimum')).not.toBeNull();
    expect(editor.getByPlaceholderText('Maximum')).not.toBeNull();
    fireEvent.change(editor.getByPlaceholderText('Default number'), { target: { value: '42' } });
    await act(async () => { fireEvent.click(editor.getByRole('button', { name: 'Save' })); });
    const savedVariable = (await repository.load(template.id)).variables[0];
    expect(savedVariable).toMatchObject({
      id: originalVariable.id,
      key: originalVariable.key,
      label: originalVariable.label,
      required: originalVariable.required,
      type: 'number',
      defaultValue: 42,
    });
    expect(template.variables[0].type).toBe('text');
  });

  test('opens a card run dialog and dispatches typed inputs with an isolated template snapshot', async () => {
    const template = createExecutableTemplateFixture();
    let request: Parameters<CreativeTemplateRunnerPort['start']>[0] | undefined;
    const view = renderWorkspace([template], { runner: { start: async (value) => { request = value; } } });
    fireEvent.click(view.getByRole('button', { name: 'Run' }));
    const runner = within(await view.findByRole('dialog', { name: 'Product poster' }));
    expect((runner.getByRole('button', { name: 'Start task' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(runner.getByRole('textbox', { name: /Product name/ }), { target: { value: 'Nomi camera' } });
    expect(runner.getByText('Create a poster for Nomi camera')).not.toBeNull();
    fireEvent.click(runner.getByRole('button', { name: 'Start task' }));
    await waitFor(() => expect(request).toBeDefined());
    expect(request!.template).toEqual(template);
    expect(request!.template).not.toBe(template);
    expect(request!.inputs).toContainEqual({ variableId: template.variables[0].id, type: 'text', value: 'Nomi camera' });
    expect(request!.referenceAssetIds).toEqual([]);
  });
});
